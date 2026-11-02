use super::{context_window_for_model, process};
use crate::model::{AgentSession, ChildProcess, LaunchSurface, SessionStatus};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// Maximum sessions to fetch from the DB per query.
const MAX_SESSIONS: u32 = 20;

/// Collector for OpenCode sessions.
///
/// Discovery strategy:
/// 1. `ps` to find running opencode processes (from shared process data)
/// 2. Query SQLite DB at ~/.local/share/opencode/opencode.db via `sqlite3` CLI
/// 3. Match running PIDs to sessions by cwd
///
/// Uses `sqlite3 -readonly -json` for safe concurrent reads (WAL mode).
/// DB rows are cached and only refreshed on `shared.slow_tick` (every ~10s)
/// so we don't fork a sqlite3 process every 2s. PID matching, status
/// derivation and the children walk run every tick using live process info.
pub struct OpenCodeCollector {
    db_path: PathBuf,
    diagnostics: super::CollectorDiagnostics,
    /// Whether sqlite3 CLI is available (checked once).
    sqlite3_available: Option<bool>,
    /// Cached DB rows from the last slow-tick query. Reused on fast ticks.
    cached_db_sessions: Vec<DbSession>,
    /// Whether the "sqlite3 missing" warning has been emitted (once).
    #[cfg(target_os = "windows")]
    warned_sqlite3_missing: bool,
}

impl OpenCodeCollector {
    pub fn new() -> Self {
        let data_dir = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".local/share"));
        let db_path = data_dir.join("opencode").join("opencode.db");
        #[cfg(target_os = "windows")]
        let db_path = windows_db_path(db_path);
        Self {
            db_path,
            diagnostics: super::CollectorDiagnostics::new("opencode", "$XDG_DATA_HOME/opencode/opencode.db (default: ~/.local/share/opencode/opencode.db)"),
            sqlite3_available: None,
            cached_db_sessions: Vec::new(),
            #[cfg(target_os = "windows")]
            warned_sqlite3_missing: false,
        }
    }

    fn check_sqlite3(&mut self) -> bool {
        if let Some(available) = self.sqlite3_available {
            return available;
        }
        let available = Command::new("sqlite3")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        self.sqlite3_available = Some(available);
        available
    }

    fn collect_sessions(&mut self, shared: &super::SharedProcessData) -> Vec<AgentSession> {
        let mut sessions = self.collect_db_sessions(shared);
        #[cfg(target_os = "linux")]
        for pid in Self::find_opencode_pids(&shared.process_info) {
            if sessions.iter().any(|s| s.pid == pid) {
                continue;
            }
            if let Some(session) = process::bare_agent_session(
                pid,
                "opencode",
                &shared.process_info,
                super::abbrev_path(self.db_path.parent().unwrap_or(std::path::Path::new("."))),
            ) {
                self.diagnostics.candidates += 1;
                self.diagnostics.skip("process_only_transcript_unavailable");
                sessions.push(session);
            }
        }
        self.diagnostics.accepted = sessions.len();
        if !sessions.is_empty() && self.diagnostics.errors.is_empty() {
            self.diagnostics.state = "ok";
        }
        sessions.sort_by_key(|s| std::cmp::Reverse(s.started_at));
        sessions
    }

    fn collect_db_sessions(&mut self, shared: &super::SharedProcessData) -> Vec<AgentSession> {
        self.diagnostics.candidates = 0;
        self.diagnostics.accepted = 0;
        self.diagnostics.skipped.clear();
        self.diagnostics.processes = Self::find_opencode_pids(&shared.process_info).len();
        // Preserve refresh errors on fast ticks until the next successful query.
        self.diagnostics.state = if self.diagnostics.errors.is_empty() {
            "empty"
        } else {
            "error"
        };
        match fs::symlink_metadata(&self.db_path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.cached_db_sessions.clear();
                self.diagnostics.errors.clear();
                self.diagnostics.schema = None;
                self.diagnostics.state = "source_missing";
                return vec![];
            }
            Err(_) => {
                self.cached_db_sessions.clear();
                self.diagnostics.fail("database_metadata_unreadable");
                return vec![];
            }
            Ok(meta) if meta.file_type().is_symlink() => {
                self.cached_db_sessions.clear();
                self.diagnostics.fail("database_symlink_rejected");
                return vec![];
            }
            Ok(_) => {}
        }
        if !self.check_sqlite3() {
            self.diagnostics.fail("sqlite3_unavailable");
            // The DB exists but we can't read it: on Windows sqlite3 is
            // usually not preinstalled, so say why sessions are missing
            // instead of failing silently.
            #[cfg(target_os = "windows")]
            if !self.warned_sqlite3_missing {
                self.warned_sqlite3_missing = true;
                eprintln!(
                    "abtop: OpenCode database found at {} but the `sqlite3` CLI is not on PATH; \
                     OpenCode sessions will not appear. Install it (e.g. `winget install SQLite.SQLite`) \
                     and restart abtop.",
                    self.db_path.display()
                );
            }
            self.cached_db_sessions.clear();
            return vec![];
        }

        // Find running opencode PIDs and their commands for cwd matching
        let opencode_pids = Self::find_opencode_pids(&shared.process_info);
        let pid_commands: HashMap<u32, &str> = opencode_pids
            .iter()
            .filter_map(|&pid| {
                shared
                    .process_info
                    .get(&pid)
                    .filter(|p| !is_service_command(&p.command))
                    .map(|p| (pid, p.command.as_str()))
            })
            .collect();

        // Refresh DB rows on slow ticks only; reuse cache on fast ticks so
        // we don't fork sqlite3 every 2s.
        if shared.slow_tick {
            match self.query_sessions() {
                Ok(rows) => {
                    self.cached_db_sessions = rows;
                    self.diagnostics.errors.clear();
                    self.diagnostics.state = "empty";
                }
                Err(reason) => {
                    self.cached_db_sessions.clear();
                    self.diagnostics.fail(reason);
                }
            }
        }

        let has_service = opencode_pids.iter().any(|pid| {
            shared
                .process_info
                .get(pid)
                .is_some_and(|p| is_service_command(&p.command))
        });
        let now_ms = current_time_ms();
        let mut sessions = Vec::new();

        let mut claimed_pids = HashSet::new();
        self.diagnostics.candidates = self.cached_db_sessions.len();
        for ds in &self.cached_db_sessions {
            let matched_pid =
                Self::match_pid_to_session_once(&pid_commands, &ds.directory, &mut claimed_pids);
            // A global service is not proof of PID ownership. Surface only
            // recent, non-idle v2 activity as Unknown without exposing its PID.
            let matched_pid = match matched_pid {
                Some(pid) => pid,
                None if has_service => {
                    if let Some(reason) = ds.service_activity_rejection(now_ms) {
                        self.diagnostics.skip(reason);
                        continue;
                    }
                    0
                }
                None => {
                    self.diagnostics.skip("no_process_ownership");
                    continue;
                }
            };

            let proc = shared.process_info.get(&matched_pid);
            let mem_mb = proc.map(|p| p.rss_kb / 1024).unwrap_or(0);

            let age_ms = now_ms.saturating_sub(ds.time_updated);
            let since_update_secs = age_ms / 1000;
            let status = if matched_pid == 0 {
                SessionStatus::Unknown
            } else if since_update_secs < 30 {
                SessionStatus::Thinking
            } else {
                let cpu_active = proc.is_some_and(|p| p.cpu_pct > 1.0);
                let has_active_child = process::has_active_descendant(
                    matched_pid,
                    &shared.children_map,
                    &shared.process_info,
                    5.0,
                );
                if cpu_active || has_active_child {
                    SessionStatus::Thinking
                } else {
                    SessionStatus::Waiting
                }
            };

            let project_name = if !ds.project_name.is_empty() {
                ds.project_name.clone()
            } else {
                // last_path_segment also splits on `\` on Windows.
                process::last_path_segment(&ds.directory)
                    .unwrap_or("?")
                    .to_string()
            };

            let current_tasks = if matches!(status, SessionStatus::Unknown) {
                vec!["recent database activity; process owner unknown".to_string()]
            } else if matches!(status, SessionStatus::Waiting) {
                vec!["waiting for input".to_string()]
            } else {
                vec!["thinking...".to_string()]
            };

            // Collect child processes with cycle guard (visited set)
            let mut children = Vec::new();
            let mut stack: Vec<u32> = shared
                .children_map
                .get(&matched_pid)
                .cloned()
                .unwrap_or_default();
            let mut visited = std::collections::HashSet::new();
            while let Some(cpid) = stack.pop() {
                if !visited.insert(cpid) {
                    continue;
                }
                if let Some(cproc) = shared.process_info.get(&cpid) {
                    let port = shared.ports.get(&cpid).and_then(|v| v.first().copied());
                    children.push(ChildProcess {
                        pid: cpid,
                        command: cproc.command.clone(),
                        mem_kb: cproc.rss_kb,
                        port,
                    });
                }
                if let Some(grandchildren) = shared.children_map.get(&cpid) {
                    stack.extend(grandchildren);
                }
            }

            let model = if !ds.provider.is_empty() && !ds.model.is_empty() {
                format!("{}/{}", ds.provider, ds.model)
            } else if !ds.model.is_empty() {
                ds.model.clone()
            } else {
                "-".to_string()
            };

            let context_window = context_window_for_model(&model, "", 0);
            // Database totals cover the whole session, not the current request.
            let context_percent = 0.0;

            sessions.push(AgentSession {
                agent_cli: "opencode",
                launch_surface: LaunchSurface::Cli,
                pid: matched_pid,
                session_id: ds.id.clone(),
                cwd: ds.directory.clone(),
                project_name,
                started_at: ds.time_created,
                status,
                model,
                effort: String::new(),
                context_percent,
                total_input_tokens: ds.total_input,
                total_output_tokens: ds.total_output,
                total_cache_read: ds.total_cache_read,
                total_cache_create: ds.total_cache_write,
                turn_count: ds.turn_count,
                current_tasks,
                mem_mb,
                version: ds.version.clone(),
                git_branch: String::new(),
                git_added: 0,
                git_modified: 0,
                token_history: vec![],
                context_history: vec![],
                compaction_count: 0,
                context_window,
                subagents: vec![],
                mem_file_count: 0,
                mem_line_count: 0,
                children,
                initial_prompt: ds.title.clone(),
                first_assistant_text: String::new(),
                chat_messages: vec![],
                tool_calls: vec![],
                pending_since_ms: 0,
                thinking_since_ms: 0,
                file_accesses: vec![],
                config_root: super::abbrev_path(
                    self.db_path.parent().unwrap_or(std::path::Path::new(".")),
                ),
            });
        }

        sessions.sort_by_key(|s| std::cmp::Reverse(s.started_at));
        self.diagnostics.accepted = sessions.len();
        if self.diagnostics.errors.is_empty() {
            self.diagnostics.state = if sessions.is_empty() {
                "no_active_sessions"
            } else {
                "ok"
            };
        }
        sessions
    }

    fn find_opencode_pids(process_info: &HashMap<u32, process::ProcInfo>) -> Vec<u32> {
        process_info
            .iter()
            .filter(|(_, info)| {
                process::cmd_has_binary(&info.command, "opencode") && !info.command.contains("grep")
            })
            .map(|(pid, _)| *pid)
            .collect()
    }

    /// Match a running PID to a session by comparing its working directory
    /// with the DB session's `directory`, falling back to a command-line
    /// substring match. Returns `None` if no PID's cwd or command line ties
    /// to this session — we deliberately do not fall back to "the only
    /// opencode process" here, because that would mark every DB row as
    /// alive whenever a single opencode is running in an unrelated dir.
    #[cfg(test)]
    fn match_pid_to_session(pid_commands: &HashMap<u32, &str>, session_dir: &str) -> Option<u32> {
        Self::match_pid_to_session_excluding(pid_commands, session_dir, &HashSet::new())
    }

    fn match_pid_to_session_once(
        pid_commands: &HashMap<u32, &str>,
        session_dir: &str,
        claimed_pids: &mut HashSet<u32>,
    ) -> Option<u32> {
        let pid = Self::match_pid_to_session_excluding(pid_commands, session_dir, claimed_pids)?;
        claimed_pids.insert(pid);
        Some(pid)
    }

    fn match_pid_to_session_excluding(
        pid_commands: &HashMap<u32, &str>,
        session_dir: &str,
        claimed_pids: &HashSet<u32>,
    ) -> Option<u32> {
        // Empty / single-character `session_dir` (e.g. "" or "/") would
        // make the substring fallback match unrelated commands, so skip
        // matching entirely in that case.
        if session_dir.len() < 2 {
            return None;
        }
        for (&pid, &cmd) in pid_commands {
            if claimed_pids.contains(&pid) {
                continue;
            }
            if let Some(cwd) = get_process_cwd(pid) {
                if paths_equal(&cwd, session_dir) {
                    return Some(pid);
                }
            }
            if cmd.contains(session_dir) {
                return Some(pid);
            }
        }
        None
    }

    /// Run a single sqlite3 query and parse the JSON output.
    fn run_query(&self, sql: &str) -> Result<Vec<Value>, &'static str> {
        let output = Command::new("sqlite3")
            .args(["-readonly", "-json", "-cmd", ".timeout 1000"])
            .arg(&self.db_path)
            .arg(sql)
            .output()
            .map_err(|_| "sqlite3_execution_failed")?;
        if !output.status.success() {
            return Err("database_query_failed");
        }
        if output.stdout.iter().all(u8::is_ascii_whitespace) {
            return Ok(vec![]);
        }
        serde_json::from_slice(&output.stdout).map_err(|_| "database_result_invalid")
    }

    fn query_v2_sessions(&self) -> Result<Vec<DbSession>, &'static str> {
        let rows = self.run_query(&format!(
            r#"
SELECT s.id, s.title, s.directory, s.version, s.time_created, s.time_updated,
 s.tokens_input, s.tokens_output, s.tokens_cache_read, s.tokens_cache_write,
 s.time_idle, s.time_suspended,
 CASE WHEN json_valid(s.model) THEN
   COALESCE(json_extract(s.model, '$.id'), json_extract(s.model, '$.modelID'), '')
   ELSE COALESCE(s.model, '') END AS model,
 CASE WHEN json_valid(s.model) THEN
   COALESCE(json_extract(s.model, '$.providerID'), '') ELSE '' END AS provider,
 (SELECT COUNT(*) FROM session_message m
  WHERE m.session_id = s.id AND m.type = 'assistant') AS turn_count
FROM session_v2 s
WHERE s.time_archived IS NULL
ORDER BY s.time_updated DESC LIMIT {MAX_SESSIONS};"#
        ))?;
        Ok(rows
            .into_iter()
            .map(|row| DbSession {
                id: sanitize_db_field(row["id"].as_str().unwrap_or(""), 256),
                title: sanitize_db_title(row["title"].as_str().unwrap_or("")),
                directory: sanitize_db_field(row["directory"].as_str().unwrap_or(""), 4096),
                version: sanitize_db_field(row["version"].as_str().unwrap_or(""), 64),
                time_created: row["time_created"].as_u64().unwrap_or(0),
                time_updated: row["time_updated"].as_u64().unwrap_or(0),
                service_activity: true,
                time_idle: row["time_idle"].as_u64(),
                time_suspended: row["time_suspended"].as_u64(),
                project_name: String::new(),
                turn_count: row["turn_count"].as_u64().unwrap_or(0) as u32,
                total_input: row["tokens_input"].as_u64().unwrap_or(0),
                total_output: row["tokens_output"].as_u64().unwrap_or(0),
                total_cache_read: row["tokens_cache_read"].as_u64().unwrap_or(0),
                total_cache_write: row["tokens_cache_write"].as_u64().unwrap_or(0),
                model: sanitize_db_field(row["model"].as_str().unwrap_or(""), 256),
                provider: sanitize_db_field(row["provider"].as_str().unwrap_or(""), 256),
            })
            .collect())
    }

    fn query_sessions(&mut self) -> Result<Vec<DbSession>, &'static str> {
        // Introspect before querying: OpenCode 2.x stores cumulative usage on
        // session_v2, whereas legacy releases store usage on each message.
        let tables = self.run_query("SELECT name FROM sqlite_master WHERE type = 'table'")?;
        if tables.iter().any(|row| row["name"] == "session_v2") {
            self.diagnostics.schema = Some("session_v2");
            return self.query_v2_sessions();
        }
        if !tables.iter().any(|row| row["name"] == "session") {
            self.diagnostics.schema = Some("unsupported");
            return Err("unsupported_schema");
        }
        self.diagnostics.schema = Some("legacy");
        let session_sql = format!(
            r#"
SELECT
  s.id, s.title, s.directory, s.version, s.time_created, s.time_updated,
  COALESCE(p.name, '') as project_name,
  COUNT(m.id) as turn_count,
  COALESCE(SUM(json_extract(m.data, '$.tokens.input')), 0) as total_input,
  COALESCE(SUM(json_extract(m.data, '$.tokens.output')), 0) as total_output,
  COALESCE(SUM(json_extract(m.data, '$.tokens.cache.read')), 0) as total_cache_read,
  COALESCE(SUM(json_extract(m.data, '$.tokens.cache.write')), 0) as total_cache_write
FROM session s
LEFT JOIN project p ON s.project_id = p.id
LEFT JOIN message m ON m.session_id = s.id
  AND json_extract(m.data, '$.role') = 'assistant'
GROUP BY s.id
ORDER BY s.time_updated DESC
LIMIT {};"#,
            MAX_SESSIONS
        );

        let model_sql = format!(
            r#"
SELECT
  s.id,
  COALESCE((SELECT json_extract(m2.data, '$.modelID')
    FROM message m2 WHERE m2.session_id = s.id
    AND json_extract(m2.data, '$.role') = 'assistant'
    ORDER BY m2.time_created DESC LIMIT 1), '') as model,
  COALESCE((SELECT json_extract(m2.data, '$.providerID')
    FROM message m2 WHERE m2.session_id = s.id
    AND json_extract(m2.data, '$.role') = 'assistant'
    ORDER BY m2.time_created DESC LIMIT 1), '') as provider
FROM session s
ORDER BY s.time_updated DESC
LIMIT {};"#,
            MAX_SESSIONS
        );

        // Two separate invocations to avoid fragile concatenated JSON parsing
        let rows = self.run_query(&session_sql)?;
        let model_rows = self.run_query(&model_sql)?;

        // Build model lookup by session id
        let mut model_map: HashMap<String, (String, String)> = HashMap::new();
        for mr in &model_rows {
            if let Some(id) = mr["id"].as_str() {
                model_map.insert(
                    id.to_string(),
                    (
                        sanitize_db_field(mr["model"].as_str().unwrap_or(""), 256),
                        sanitize_db_field(mr["provider"].as_str().unwrap_or(""), 256),
                    ),
                );
            }
        }

        let mut sessions = Vec::new();
        for row in rows {
            let id = row["id"].as_str().unwrap_or("").to_string();
            let (model, provider) = model_map.remove(&id).unwrap_or_default();

            // Sanitize DB-sourced strings before they reach the TUI/JSON snapshot.
            let title = sanitize_db_title(row["title"].as_str().unwrap_or(""));
            let directory = sanitize_db_field(row["directory"].as_str().unwrap_or(""), 4096);
            let version = sanitize_db_field(row["version"].as_str().unwrap_or(""), 64);
            let project_name = sanitize_db_field(row["project_name"].as_str().unwrap_or(""), 256);

            sessions.push(DbSession {
                id,
                title,
                directory,
                version,
                // time_created and time_updated are in milliseconds since epoch
                time_created: row["time_created"].as_u64().unwrap_or(0),
                time_updated: row["time_updated"].as_u64().unwrap_or(0),
                service_activity: false,
                time_idle: None,
                time_suspended: None,
                project_name,
                turn_count: row["turn_count"].as_u64().unwrap_or(0) as u32,
                total_input: row["total_input"].as_u64().unwrap_or(0),
                total_output: row["total_output"].as_u64().unwrap_or(0),
                total_cache_read: row["total_cache_read"].as_u64().unwrap_or(0),
                total_cache_write: row["total_cache_write"].as_u64().unwrap_or(0),
                model,
                provider,
            });
        }

        Ok(sessions)
    }
}

impl Default for OpenCodeCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl super::AgentCollector for OpenCodeCollector {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn diagnostics(&self) -> Option<super::CollectorDiagnostics> {
        Some(self.diagnostics.clone())
    }

    fn collect(&mut self, shared: &super::SharedProcessData) -> Vec<AgentSession> {
        self.collect_sessions(shared)
    }
}

struct DbSession {
    id: String,
    title: String,
    directory: String,
    version: String,
    time_created: u64,
    time_updated: u64,
    service_activity: bool,
    time_idle: Option<u64>,
    time_suspended: Option<u64>,
    project_name: String,
    turn_count: u32,
    total_input: u64,
    total_output: u64,
    total_cache_read: u64,
    total_cache_write: u64,
    model: String,
    provider: String,
}

impl DbSession {
    fn service_activity_rejection(&self, now_ms: u64) -> Option<&'static str> {
        if !self.service_activity {
            Some("service_session_without_lifecycle_evidence")
        } else if self.time_suspended.is_some() {
            Some("service_session_suspended")
        } else if self.time_updated == 0 || self.time_updated > now_ms {
            Some("service_session_invalid_timestamp")
        } else if self.time_idle.is_some_and(|idle| idle >= self.time_updated) {
            Some("service_session_idle")
        } else if now_ms - self.time_updated >= 30_000 {
            Some("service_session_stale")
        } else {
            None
        }
    }
}

fn is_service_command(command: &str) -> bool {
    command
        .split_whitespace()
        .any(|arg| matches!(arg, "serve" | "--service"))
}

fn sanitize_db_title(raw: &str) -> String {
    super::redact_secrets(&sanitize_db_field(raw, 512))
}

fn sanitize_db_field(raw: &str, max_bytes: usize) -> String {
    let mut value = super::sanitize_terminal_text(raw);
    truncate_field(&mut value, max_bytes);
    value
}

/// Truncate a string at a char boundary to avoid panics on multi-byte UTF-8.
fn truncate_field(s: &mut String, max_bytes: usize) {
    if s.len() > max_bytes {
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
}

/// Compare a process cwd with a DB session directory.
/// On Windows paths are case-insensitive and may mix `/` and `\`, so
/// normalize before comparing; elsewhere keep the exact comparison.
#[cfg(target_os = "windows")]
fn paths_equal(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

#[cfg(not(target_os = "windows"))]
fn paths_equal(a: &str, b: &str) -> bool {
    a == b
}

/// On Windows, OpenCode builds (e.g. installed via npm) have been observed to
/// keep the XDG-style `~/.local/share/opencode` layout, so prefer the same
/// path as unix; fall back to probing `%LOCALAPPDATA%` / `%APPDATA%` in case
/// a build stores the DB there instead.
#[cfg(target_os = "windows")]
fn windows_db_path(default: PathBuf) -> PathBuf {
    if default.exists() {
        return default;
    }
    for var in ["LOCALAPPDATA", "APPDATA"] {
        if let Ok(base) = std::env::var(var) {
            if base.is_empty() {
                continue;
            }
            let candidate = PathBuf::from(base).join("opencode").join("opencode.db");
            if candidate.exists() {
                return candidate;
            }
        }
    }
    default
}

/// Get the current working directory of a process.
/// Uses /proc on Linux, sysinfo (PEB) on Windows, lsof on macOS/other Unix.
#[cfg(target_os = "linux")]
fn get_process_cwd(pid: u32) -> Option<String> {
    std::fs::read_link(format!("/proc/{}/cwd", pid))
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(target_os = "windows")]
fn get_process_cwd(pid: u32) -> Option<String> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    // `lsof` does not exist on Windows; sysinfo reads the cwd from the
    // process PEB. Refresh just this one PID — this runs only for the
    // handful of opencode PIDs, once per tick.
    let mut sys = System::new();
    let pid = Pid::from_u32(pid);
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::new().with_cwd(UpdateKind::Always),
    );
    sys.process(pid)
        .and_then(|p| p.cwd())
        .map(|p| p.to_string_lossy().into_owned())
}

#[cfg(all(not(target_os = "linux"), not(target_os = "windows")))]
fn get_process_cwd(pid: u32) -> Option<String> {
    // -a ANDs the selection terms; without it, lsof ORs `-p <pid>` with
    // `-d cwd` and returns cwd entries for unrelated processes too.
    let output = Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    // lsof -Fn output: lines starting with 'n' contain the path
    stdout
        .lines()
        .find(|l| l.starts_with('n') && l.len() > 1)
        .map(|l| l[1..].to_string())
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_collector(sql: &str) -> (tempfile::TempDir, OpenCodeCollector) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("opencode.db");
        let output = Command::new("sqlite3")
            .arg(&db_path)
            .arg(sql)
            .output()
            .expect("SQLite CLI is required for database regression tests");
        assert!(output.status.success(), "fixture creation failed");
        let mut collector = OpenCodeCollector::new();
        collector.db_path = db_path;
        (dir, collector)
    }

    fn v2_fixture() -> (tempfile::TempDir, OpenCodeCollector) {
        fixture_collector(
            r#"
CREATE TABLE session_v2 (
 id TEXT, title TEXT, directory TEXT, version TEXT,
 time_created INTEGER, time_updated INTEGER, time_archived INTEGER,
 time_idle INTEGER, time_suspended INTEGER,
 model TEXT, tokens_input INTEGER, tokens_output INTEGER,
 tokens_cache_read INTEGER, tokens_cache_write INTEGER);
CREATE TABLE session_message (id TEXT, session_id TEXT, type TEXT);
INSERT INTO session_v2 VALUES ('live', 'Example', '/tmp/project', '2.1.4',
 1000, 2000, NULL, NULL, NULL, '{"id":"gpt-5","providerID":"openai"}', 100, 20, 30, 4);
INSERT INTO session_message VALUES ('a', 'live', 'assistant'), ('b', 'live', 'assistant'),
 ('u', 'live', 'user');
INSERT INTO session_v2 SELECT 'archived', title, directory, version,
 time_created, time_updated, 3000, NULL, NULL, model, 999, 999, 999, 999 FROM session_v2;
"#,
        )
    }

    #[test]
    fn query_v2_reads_session_totals_without_double_counting_messages() {
        let (_dir, mut collector) = v2_fixture();
        let rows = collector
            .query_sessions()
            .expect("v2 schema must be supported");
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert_eq!(row.id, "live");
        assert_eq!((row.total_input, row.total_output), (100, 20));
        assert_eq!((row.total_cache_read, row.total_cache_write), (30, 4));
        assert_eq!(row.turn_count, 2);
        assert_eq!((&*row.model, &*row.provider), ("gpt-5", "openai"));
    }

    #[test]
    fn query_legacy_and_empty_database() {
        let (_dir, mut collector) = fixture_collector(
            r#"
CREATE TABLE session (id TEXT, title TEXT, directory TEXT, version TEXT,
 time_created INTEGER, time_updated INTEGER, project_id TEXT);
CREATE TABLE project (id TEXT, name TEXT);
CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, data TEXT);
"#,
        );
        assert!(collector.query_sessions().unwrap().is_empty());
        let output = Command::new("sqlite3")
            .arg(&collector.db_path)
            .arg(
                r#"INSERT INTO session VALUES
 ('legacy', 'Example', '/tmp/project', '1', 1000, 2000, 'project');
INSERT INTO message VALUES ('message', 'legacy', 2000,
 '{"role":"assistant","modelID":"gpt-5","tokens":{"input":10,"output":2}}');"#,
            )
            .output()
            .unwrap();
        assert!(output.status.success());
        let rows = collector.query_sessions().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].total_input, 10);
        assert_eq!(rows[0].total_output, 2);
    }

    fn shared_with_service() -> super::super::SharedProcessData {
        super::super::SharedProcessData {
            process_info: HashMap::from([(
                u32::MAX,
                process::ProcInfo {
                    pid: u32::MAX,
                    ppid: 1,
                    rss_kb: 1024,
                    cpu_pct: 99.0,
                    // Even a matching service cwd argument must not establish ownership.
                    command: "opencode serve --service --cwd=/tmp/project".into(),
                },
            )]),
            children_map: HashMap::new(),
            ports: HashMap::new(),
            slow_tick: true,
            mcp_server_pids: HashSet::new(),
            mcp_owned_rollouts: HashSet::new(),
            mcp_suppress: true,
            desktop_rollout_fd_map: HashMap::new(),
        }
    }

    fn update_fixture(collector: &OpenCodeCollector, sql: &str) {
        let output = Command::new("sqlite3")
            .arg(&collector.db_path)
            .arg(sql)
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    #[test]
    fn service_activity_is_unknown_without_assigning_daemon_resources() {
        let (_dir, mut collector) = v2_fixture();
        update_fixture(
            &collector,
            &format!(
                "UPDATE session_v2 SET time_updated = {} WHERE id = 'live';",
                current_time_ms()
            ),
        );
        let original = fs::read(&collector.db_path).unwrap();
        let sessions = collector.collect_sessions(&shared_with_service());
        assert_eq!(sessions.len(), 1);
        let session = &sessions[0];
        assert_eq!(session.status, SessionStatus::Unknown);
        assert_eq!(session.pid, 0);
        assert_eq!(session.mem_mb, 0);
        assert_eq!(session.context_percent, 0.0);
        assert!(session.children.is_empty());
        assert_eq!(fs::read(&collector.db_path).unwrap(), original);
        assert_eq!(collector.diagnostics.state, "ok");
    }

    #[test]
    fn service_diagnostics_explain_idle_stale_and_suspended_filters() {
        let (_dir, mut collector) = v2_fixture();
        let now = current_time_ms();
        for (sql, reason) in [
            (
                format!("UPDATE session_v2 SET time_updated = {now}, time_idle = {now}"),
                "service_session_idle",
            ),
            (
                format!(
                    "UPDATE session_v2 SET time_idle = NULL, time_updated = {}",
                    now - 60_000
                ),
                "service_session_stale",
            ),
            (
                format!("UPDATE session_v2 SET time_updated = {now}, time_suspended = {now}"),
                "service_session_suspended",
            ),
        ] {
            update_fixture(&collector, &sql);
            assert!(collector
                .collect_sessions(&shared_with_service())
                .is_empty());
            assert_eq!(collector.diagnostics.skipped.get(reason), Some(&1));
            assert!(collector.diagnostics.errors.is_empty());
        }
    }

    #[test]
    fn service_does_not_revive_historical_idle_suspended_or_future_rows() {
        let (_dir, mut collector) = v2_fixture();
        assert!(collector
            .collect_sessions(&shared_with_service())
            .is_empty());
        let now = current_time_ms();
        for sql in [
            format!("UPDATE session_v2 SET time_updated = {now}, time_idle = {now}"),
            format!("UPDATE session_v2 SET time_idle = NULL, time_suspended = {now}"),
            format!(
                "UPDATE session_v2 SET time_suspended = NULL, time_updated = {}",
                now + 60_000
            ),
        ] {
            update_fixture(&collector, &sql);
            assert!(collector
                .collect_sessions(&shared_with_service())
                .is_empty());
        }
        assert!(collector.diagnostics.errors.is_empty());
    }

    #[test]
    fn service_handles_multiple_recent_projects_without_claiming_one_pid() {
        let (_dir, mut collector) = v2_fixture();
        update_fixture(
            &collector,
            &format!(
                r#"
UPDATE session_v2 SET time_updated = {} WHERE id = 'live';
INSERT INTO session_v2 SELECT 'second', title, '/tmp/other-project', version,
 time_created, time_updated, NULL, NULL, NULL, model,
 tokens_input, tokens_output, tokens_cache_read, tokens_cache_write
 FROM session_v2 WHERE id = 'live';"#,
                current_time_ms()
            ),
        );
        let sessions = collector.collect_sessions(&shared_with_service());
        assert_eq!(sessions.len(), 2);
        assert!(sessions
            .iter()
            .all(|s| s.pid == 0 && s.status == SessionStatus::Unknown));
        let mut shared = shared_with_service();
        shared.process_info.clear();
        assert!(collector.collect_sessions(&shared).is_empty());
    }

    #[test]
    fn query_failure_is_visible_and_clears_stale_cache_until_recovery() {
        let (_dir, mut collector) = v2_fixture();
        update_fixture(
            &collector,
            &format!("UPDATE session_v2 SET time_updated = {}", current_time_ms()),
        );
        let mut shared = shared_with_service();
        assert!(!collector.collect_sessions(&shared).is_empty());
        update_fixture(
            &collector,
            "ALTER TABLE session_message RENAME TO unsupported_messages",
        );
        assert!(collector.collect_sessions(&shared).is_empty());
        assert_eq!(collector.diagnostics.errors, vec!["database_query_failed"]);
        shared.slow_tick = false;
        assert!(collector.collect_sessions(&shared).is_empty());
        assert_eq!(collector.diagnostics.state, "error");
        update_fixture(
            &collector,
            "ALTER TABLE unsupported_messages RENAME TO session_message",
        );
        shared.slow_tick = true;
        assert!(!collector.collect_sessions(&shared).is_empty());
        assert!(collector.diagnostics.errors.is_empty());
    }

    #[test]
    fn diagnostics_distinguish_unknown_schema_and_missing_database() {
        let (_dir, mut collector) = fixture_collector("CREATE TABLE unknown_schema (id TEXT)");
        let shared = shared_with_service();
        assert!(collector.collect_sessions(&shared).is_empty());
        assert_eq!(collector.diagnostics.errors, vec!["unsupported_schema"]);
        collector.db_path.set_file_name("missing.db");
        assert!(collector.collect_sessions(&shared).is_empty());
        assert_eq!(collector.diagnostics.state, "source_missing");
        assert!(collector.diagnostics.errors.is_empty());
    }

    #[test]
    fn diagnostics_do_not_expose_database_content_or_paths() {
        let (_dir, mut collector) =
            fixture_collector("CREATE TABLE secret_prompt (secret_response TEXT)");
        collector.collect_sessions(&shared_with_service());
        let report = serde_json::to_string(&collector.diagnostics).unwrap();
        assert!(!report.contains("secret_prompt"));
        assert!(!report.contains("secret_response"));
        assert!(!report.contains(collector.db_path.to_str().unwrap()));
        assert!(report.contains("unsupported_schema"));
    }

    #[test]
    fn test_find_opencode_pids() {
        let mut info = HashMap::new();
        info.insert(
            100,
            process::ProcInfo {
                pid: 100,
                ppid: 1,
                rss_kb: 1000,
                cpu_pct: 0.0,
                command: "/home/user/.opencode/bin/opencode".to_string(),
            },
        );
        info.insert(
            200,
            process::ProcInfo {
                pid: 200,
                ppid: 1,
                rss_kb: 500,
                cpu_pct: 0.0,
                command: "grep opencode".to_string(),
            },
        );
        info.insert(
            300,
            process::ProcInfo {
                pid: 300,
                ppid: 1,
                rss_kb: 800,
                cpu_pct: 0.0,
                command: "node /usr/bin/opencode run test".to_string(),
            },
        );
        let pids = OpenCodeCollector::find_opencode_pids(&info);
        assert!(pids.contains(&100));
        assert!(!pids.contains(&200)); // grep excluded
        assert!(pids.contains(&300));
        assert_eq!(pids.len(), 2);
    }

    #[test]
    fn test_db_path_default() {
        let collector = OpenCodeCollector::new();
        let path_str = collector.db_path.to_string_lossy();
        assert!(path_str.contains("opencode"));
        assert!(path_str.ends_with("opencode.db"));
    }

    #[test]
    fn sanitize_db_field_removes_terminal_control_chars() {
        assert_eq!(
            sanitize_db_field("proj\u{202E}\u{0008}name", 512),
            "projname"
        );
    }

    #[test]
    fn sanitize_db_title_redacts_known_secret_prefixes() {
        assert_eq!(
            sanitize_db_title("debug sk-ant-secret-value now"),
            "debug [REDACTED] now"
        );
    }

    #[test]
    fn match_pid_short_session_dir_never_matches() {
        // Regression: previously, an empty `directory` made `cmd.contains("")`
        // always true, and `/` matched every absolute path. Both should fail
        // the length guard now, regardless of how many opencode procs run.
        let mut pid_commands: HashMap<u32, &str> = HashMap::new();
        pid_commands.insert(100, "/usr/local/bin/opencode");
        pid_commands.insert(200, "/usr/local/bin/opencode --foo");
        assert_eq!(
            OpenCodeCollector::match_pid_to_session(&pid_commands, ""),
            None
        );
        assert_eq!(
            OpenCodeCollector::match_pid_to_session(&pid_commands, "/"),
            None
        );
    }

    #[test]
    fn match_pid_no_last_resort_when_cwd_and_cmdline_disagree() {
        // Regression: previously, when exactly one opencode process was
        // running, the `pid_commands.len() == 1` last-resort branch matched
        // every DB session to it, even when neither the cwd nor the command
        // line matched. With the fallback removed, this returns None.
        let mut pid_commands: HashMap<u32, &str> = HashMap::new();
        // u32::MAX is a synthetic PID that has no /proc/<pid>/cwd entry,
        // so the cwd branch can't accidentally succeed.
        pid_commands.insert(u32::MAX, "/usr/local/bin/opencode");
        assert_eq!(
            OpenCodeCollector::match_pid_to_session(&pid_commands, "/home/u/proj-a"),
            None
        );
    }

    #[test]
    fn match_pid_substring_fallback_still_works() {
        let mut pid_commands: HashMap<u32, &str> = HashMap::new();
        pid_commands.insert(u32::MAX, "node /usr/bin/opencode run --cwd=/home/u/proj-a");
        assert_eq!(
            OpenCodeCollector::match_pid_to_session(&pid_commands, "/home/u/proj-a"),
            Some(u32::MAX)
        );
    }

    #[test]
    fn match_pid_to_session_once_does_not_reuse_pid_for_old_rows() {
        let mut pid_commands: HashMap<u32, &str> = HashMap::new();
        pid_commands.insert(u32::MAX, "node /usr/bin/opencode run --cwd=/home/u/proj-a");
        let mut claimed_pids = std::collections::HashSet::new();

        assert_eq!(
            OpenCodeCollector::match_pid_to_session_once(
                &pid_commands,
                "/home/u/proj-a",
                &mut claimed_pids,
            ),
            Some(u32::MAX)
        );
        assert_eq!(
            OpenCodeCollector::match_pid_to_session_once(
                &pid_commands,
                "/home/u/proj-a",
                &mut claimed_pids,
            ),
            None
        );
    }
}
