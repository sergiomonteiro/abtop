use serde_json::Value;
use std::process::Command;

fn fixture(sql: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("opencode");
    std::fs::create_dir(&data).unwrap();
    let result = Command::new("sqlite3")
        .arg(data.join("opencode.db"))
        .arg(sql)
        .output()
        .expect("SQLite CLI is required for collector integration tests");
    assert!(result.status.success());
    root
}

fn run(root: &tempfile::TempDir, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_abtop"))
        .args(args)
        .env("HOME", root.path())
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_DATA_HOME", root.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).expect("CLI must emit valid JSON")
}

#[test]
fn diagnostics_and_snapshot_distinguish_schema_failure_without_leaking_contents() {
    let root = fixture("CREATE TABLE private_conversation (secret_response TEXT)");
    let diagnosis = run(&root, &["--diagnose", "--json"]);
    assert!(diagnosis["version"].is_string());
    assert!(diagnosis.get("sessions").is_none());
    let opencode = diagnosis["collectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["agent"] == "opencode")
        .unwrap();
    assert_eq!(opencode["schema"], "unsupported");
    assert_eq!(opencode["errors"][0], "unsupported_schema");
    let text = diagnosis.to_string();
    assert!(!text.contains("private_conversation"));
    assert!(!text.contains("secret_response"));
    assert!(!text.contains(root.path().to_str().unwrap()));

    let snapshot = run(&root, &["--once", "--json"]);
    assert!(snapshot["sessions"].is_array());
    assert!(snapshot["collector_errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["agent"] == "opencode" && row["state"] == "error"));
}

#[test]
fn empty_supported_database_is_not_a_collection_error() {
    let root = fixture(
        r#"
CREATE TABLE session (id TEXT, title TEXT, directory TEXT, version TEXT,
 time_created INTEGER, time_updated INTEGER, project_id TEXT);
CREATE TABLE project (id TEXT, name TEXT);
CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, data TEXT);
"#,
    );
    let diagnosis = run(&root, &["--diagnose"]);
    let opencode = diagnosis["collectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["agent"] == "opencode")
        .unwrap();
    assert_eq!(opencode["schema"], "legacy");
    // Other tests or real terminals may have bare clients in the shared process
    // namespace. An empty database is still valid when those clients are shown.
    let accepted = opencode["accepted"].as_u64().unwrap();
    assert_eq!(
        opencode["state"],
        if accepted == 0 {
            "no_active_sessions"
        } else {
            "ok"
        }
    );
    assert_eq!(opencode["candidates"], accepted);
    assert!(opencode["errors"].as_array().unwrap().is_empty());
    let snapshot = run(&root, &["--json"]);
    assert!(snapshot
        .get("collector_errors")
        .and_then(Value::as_array)
        .is_none_or(|errors| !errors.iter().any(|row| row["agent"] == "opencode")));
}

#[cfg(target_os = "linux")]
#[test]
fn json_snapshot_contains_a_session_with_confirmed_process_cwd() {
    struct AgentProcess(std::process::Child);
    impl Drop for AgentProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let root = fixture(
        r#"
CREATE TABLE session_v2 (id TEXT, title TEXT, directory TEXT, version TEXT,
 time_created INTEGER, time_updated INTEGER, time_archived INTEGER,
 time_idle INTEGER, time_suspended INTEGER, model TEXT, tokens_input INTEGER,
 tokens_output INTEGER, tokens_cache_read INTEGER, tokens_cache_write INTEGER);
CREATE TABLE session_message (id TEXT, session_id TEXT, type TEXT);
"#,
    );
    // A harmless executable with the monitored binary name gives the collector
    // real /proc cwd evidence without starting an AI agent or touching user data.
    let executable = root.path().join("opencode").join("opencode");
    std::os::unix::fs::symlink("/bin/sleep", &executable).unwrap();
    let agent = AgentProcess(
        Command::new(&executable)
            .arg("30")
            .current_dir(root.path())
            .spawn()
            .unwrap(),
    );
    let directory = root.path().to_str().unwrap().replace('\'', "''");
    let sql = format!(
        r#"INSERT INTO session_v2 VALUES
 ('fixture-session', 'Fixture', '{directory}', '2.1.4', 1000, 2000,
 NULL, NULL, NULL, '{{"id":"gpt-5","providerID":"openai"}}', 100, 20, 30, 4);"#
    );
    let output = Command::new("sqlite3")
        .arg(root.path().join("opencode/opencode.db"))
        .arg(sql)
        .output()
        .unwrap();
    assert!(output.status.success());
    let snapshot = run(&root, &["--once", "--json"]);
    let session = snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["session_id"] == "fixture-session")
        .unwrap();
    assert_eq!(session["pid"], agent.0.id());
    assert_eq!(session["agent_cli"], "opencode");
    assert_eq!(session["input_tokens"], 100);
    assert_eq!(session["output_tokens"], 20);
    assert_eq!(session["cache_read_tokens"], 30);
    assert_eq!(session["total_tokens"], 154);
    assert_eq!(session["status"], "Waiting");
}

#[cfg(target_os = "linux")]
#[test]
fn bare_codex_cli_without_rollout_is_visible_without_borrowing_historical_usage() {
    struct AgentProcess(std::process::Child);
    impl Drop for AgentProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("codex");
    // cat with an open stdin pipe is a harmless process with no CLI arguments.
    std::os::unix::fs::symlink("/bin/cat", &executable).unwrap();
    let agent = AgentProcess(
        Command::new(&executable)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .current_dir(root.path())
            .spawn()
            .unwrap(),
    );
    // Wait for the child to finish exec and establish its working directory.
    for _ in 0..100 {
        if std::fs::read_link(format!("/proc/{}/cwd", agent.0.id()))
            .ok()
            .as_deref()
            == Some(root.path())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let initial = run(&root, &["--json"]);
    assert!(initial["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["pid"] == agent.0.id()));
    let sessions_dir = root
        .path()
        .join(".codex/sessions")
        .join(chrono::Local::now().format("%Y/%m/%d").to_string());
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::write(sessions_dir.join("rollout-unowned.jsonl"), concat!(
        r#"{"type":"session_meta","payload":{"id":"unowned","cwd":"/other-project"}}"#, "\n",
        r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":9999}}}}"#, "\n"
    )).unwrap();
    let snapshot = run(&root, &["--once", "--json"]);
    let session = snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["pid"] == agent.0.id())
        .expect("live bare Codex must be visible before a rollout exists");
    assert_eq!(session["agent_cli"], "codex");
    assert_eq!(session["status"], "Unknown");
    assert_eq!(session["total_tokens"], 0);
    assert_eq!(session["turn_count"], 0);
    assert_eq!(session["cwd"], root.path().to_str().unwrap());
    assert!(session["current_task"]
        .as_str()
        .unwrap()
        .contains("transcript unavailable"));
    let old_pid = agent.0.id();
    drop(agent);
    let snapshot = run(&root, &["--json"]);
    assert!(!snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["pid"] == old_pid));

    let rollout_path = sessions_dir.join("rollout-owned.jsonl");
    std::fs::write(&rollout_path, r#"{"type":"session_meta","payload":{"id":"owned-session","cwd":"/fixture"}}
{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":50,"output_tokens":10}}}}
"#).unwrap();
    let agent = AgentProcess(
        Command::new(&executable)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(rollout_path)
                    .unwrap(),
            )
            .current_dir(root.path())
            .spawn()
            .unwrap(),
    );
    let snapshot = run(&root, &["--json"]);
    let matching: Vec<_> = snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["pid"] == agent.0.id())
        .collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0]["session_id"], "owned-session");
    assert_eq!(matching[0]["total_tokens"], 60);
}

#[cfg(target_os = "linux")]
#[test]
fn bare_opencode_without_database_or_matching_session_is_visible() {
    struct AgentProcess(std::process::Child);
    impl Drop for AgentProcess {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let executable = bin.join("opencode");
    std::os::unix::fs::symlink("/bin/cat", &executable).unwrap();
    let agent = AgentProcess(
        Command::new(executable)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .current_dir(root.path())
            .spawn()
            .unwrap(),
    );
    for with_history in [false, true] {
        if with_history {
            let db_dir = root.path().join("opencode");
            std::fs::create_dir(&db_dir).unwrap();
            let output = Command::new("sqlite3").arg(db_dir.join("opencode.db")).arg(r#"
CREATE TABLE session (id TEXT, title TEXT, directory TEXT, version TEXT,
 time_created INTEGER, time_updated INTEGER, project_id TEXT);
CREATE TABLE project (id TEXT, name TEXT);
CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, data TEXT);
INSERT INTO session VALUES ('unrelated', 'Unrelated history', '/other-project', '1', 1000, 2000, 'project');
INSERT INTO message VALUES ('old-message', 'unrelated', 2000,
 '{"role":"assistant","tokens":{"input":9999,"output":8888}}');
"#).output().unwrap();
            assert!(output.status.success());
        }
        let snapshot = run(&root, &["--json"]);
        let matching: Vec<_> = snapshot["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|row| row["pid"] == agent.0.id())
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "bare OpenCode must be visible with_history={with_history}"
        );
        let row = matching[0];
        assert_eq!(row["agent_cli"], "opencode");
        assert_eq!(row["status"], "Unknown");
        assert_eq!(row["cwd"], root.path().to_str().unwrap());
        assert_eq!(row["total_tokens"], 0);
        assert_eq!(row["turn_count"], 0);
        assert!(row["current_task"]
            .as_str()
            .unwrap()
            .contains("transcript unavailable"));
    }
    let pid = agent.0.id();
    drop(agent);
    let snapshot = run(&root, &["--json"]);
    assert!(!snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["pid"] == pid));
}
