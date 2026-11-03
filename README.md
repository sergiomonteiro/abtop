# abtop

**Like [btop](https://github.com/aristocratos/btop), but for your AI coding agents.**

See every Claude Code, Codex CLI, and OpenCode session at a glance — token usage, context window %, rate limits, child processes, open ports, and more.
Claude Code, Codex CLI, and OpenCode sessions are discovered from local process/file state, so multiple active profiles are supported across macOS, Linux, and Windows.

![demo](https://raw.githubusercontent.com/graykode/abtop/main/assets/demo.gif)

## Why

- Running 3+ agents across projects? See them all in one screen.
- Hitting rate limits? Watch your quota in real-time.
- Agent spawned a server and forgot to kill it? Orphan port detection.
- Context window filling up? Per-session % bars with warnings.

All read-only. No API keys. No auth.

## Install

### macOS / Linux

> [!IMPORTANT]
> On Linux, ensure `sqlite3` is installed to enable monitoring for OpenCode sessions.

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/graykode/abtop/releases/latest/download/abtop-installer.sh | sh
```

### Cargo

```bash
cargo install abtop
```

### Windows

Native support — no WSL required. Uses `sysinfo` for process info and host CPU/MEM metrics, and `netstat -ano` for listening ports. Windows has no load average, so LOAD is reported as 0. OpenCode session discovery additionally requires the `sqlite3` CLI (`winget install SQLite.SQLite`); without it abtop prints a one-time warning to stderr.

```powershell
powershell -c "irm https://github.com/graykode/abtop/releases/latest/download/abtop-installer.ps1 | iex"
```

Or `cargo install abtop` from any terminal with Git in PATH. Claude Code config is resolved automatically from `%USERPROFILE%\.claude`.

### Other

Pre-built binaries for all platforms are available on the [GitHub Releases](https://github.com/graykode/abtop/releases) page.

## Usage

```bash
abtop                    # Launch TUI
abtop --once             # Print snapshot and exit
abtop --json             # Print one JSON snapshot and exit (for scripts/tools)
abtop --diagnose         # Print collector diagnostics without conversation content
abtop --setup            # Install rate limit collection hook
abtop --theme dracula    # Launch with a specific theme
abtop --mouse            # Enable mouse click/scroll navigation
```

Recommended terminal size: **120x40** or larger. Minimum 80x24 — panels hide gracefully when small.
Mouse capture is off by default so terminal drag selection and copy keep working. Launch with `--mouse` if you prefer click targets and wheel navigation.

### Terminal Jump

Press `Enter` to focus the terminal running the selected agent. abtop supports cmux, tmux, and iTerm2 on macOS.

```bash
tmux new -s work
# pane 0: abtop
# pane 1: claude (project A)
# pane 2: claude (project B)
# → Enter on a session in abtop jumps to its pane
```

## Supported Agents

✅ Supported when the required source is available · ◐ Partial/conditional coverage · ❌ Not collected.
These are collector capabilities, not a guarantee that every discovered session has every metric.

| Feature               | Claude Code | Codex CLI / App Server | OpenCode |
| --------------------- | :---------: | :--------------------: | :------: |
| Session Discovery     |     ✅      |           ✅           |    ✅    |
| Token Tracking        |     ✅      |           ◐            |    ◐     |
| Context Window %      |     ✅      |           ◐            |    ❌    |
| Status Detection      |     ✅      |           ◐            |    ◐     |
| Current Tool / Task   |     ✅      |           ◐            |    ❌    |
| Rate Limit            |     ✅      |           ◐            |    ❌    |
| Git Status            |     ✅      |           ✅           |    ✅    |
| Children / Ports      |     ✅      |           ◐            |    ◐     |
| Subagents             |     ✅      |           ❌           |    ❌    |
| Memory Status         |     ✅      |           ❌           |    ❌    |
| Collector Diagnostics |     ◐      |           ✅           |    ✅    |

Coverage details:

- **Codex CLI / App Server:** discovery includes eligible FD-owned App Server
  rollouts, including non-Desktop originators. On Linux, verified argument-free
  CLI processes can also appear before a rollout is available. These process-only
  rows have `Unknown` activity, no transcript metrics, and no collected children.
  Tokens, context, tasks, and quota require the corresponding rollout events;
  process-tree/port attribution requires process-tree ownership. A daemon alone
  is not a session.
- **OpenCode:** both legacy and `session_v2` SQLite schemas are supported. Tokens
  come from database-backed sessions; v2 cumulative totals are not current context
  usage. Recent qualifying global-service activity can appear as `Unknown`
  without assigning the service's PID, memory, children, or process controls.
  This is a recent-activity heuristic, not proof of execution; old or idle unowned
  rows remain hidden. On Linux, verified argument-free terminal processes can
  appear without a matching database row, with `Unknown` activity and no
  transcript metrics or collected children. Generic labels such as "thinking"
  do not constitute tool/task tracking.
- **Unavailable metrics:** the current model still uses zero for some unavailable
  values, including OpenCode context percentage and process-only token counters.
  Do not interpret these zeros as measured usage. Explicit availability is planned
  in the roadmap.
- **Status and controls:** status detection is heuristic for all agents. OpenCode
  process matching also uses cwd/command evidence, not an authoritative session/PID
  mapping. Rows without a PID cannot be killed or terminal-jumped.
- **Diagnostics:** `--diagnose` provides detailed discovery/error/filter information
  for Codex and OpenCode. Claude currently reports basic counts as
  `not_instrumented`, not equivalent failure coverage. JSON snapshots and the TUI
  also expose collection failures reported by collectors.
- **Quota:** Claude requires the StatusLine setup and available account data;
  Codex requires reported rollout rate-limit data. OpenCode does not contribute
  quota data.

OpenCode database collection requires `sqlite3` in `PATH` (on Windows:
`winget install SQLite.SQLite`). The default database is
`~/.local/share/opencode/opencode.db`; `XDG_DATA_HOME` overrides the data root.
On Windows, `%LOCALAPPDATA%\opencode` and `%APPDATA%\opencode` are probed as
fallbacks. The Linux process-only fallback does not require a database.

See [the collector audit](docs/collector-audit.md) for regression-test evidence,
known heuristics, and remaining real-environment validation gaps.

## Themes

12 built-in themes, including 4 colorblind-friendly options (`high-contrast`, `protanopia`, `deuteranopia`, `tritanopia`). Press `t` to cycle at runtime, or launch with `--theme <name>`. Your choice is saved to `~/.config/abtop/config.toml`.

| btop (default) | dracula | catppuccin |
|:-:|:-:|:-:|
| ![btop](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/btop.png) | ![dracula](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/dracula.png) | ![catppuccin](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/catppuccin.png) |

| tokyo-night | gruvbox | nord |
|:-:|:-:|:-:|
| ![tokyo-night](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/tokyo-night.png) | ![gruvbox](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/gruvbox.png) | ![nord](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/nord.png) |

Colorblind-friendly themes:

| high-contrast | protanopia |
|:-:|:-:|
| ![high-contrast](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/high-contrast.png) | ![protanopia](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/protanopia.png) |

| deuteranopia | tritanopia |
|:-:|:-:|
| ![deuteranopia](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/deuteranopia.png) | ![tritanopia](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/tritanopia.png) |

Light themes (`light` — Solarized cream, `white` — GitHub-style pure white) for bright terminals:

| light | white |
|:-:|:-:|
| ![light](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/light.png) | ![white](https://raw.githubusercontent.com/graykode/abtop/main/assets/themes/white.png) |

## Configuration

`~/.config/abtop/config.toml` supports:

```toml
theme = "btop"
# Hide specific agent CLIs from the TUI (case-insensitive).
# Useful if you only use one agent and want a cleaner view.
hidden_agents = ["codex"]
# Additional Claude Code profile roots to scan.
# abtop also auto-discovers ~/.claude and ~/.claude-* roots that contain
# both sessions/ and projects/.
claude_config_dirs = ["~/.claude-personal", "~/.claude-work-team"]
# UI language. Omit or leave empty to auto-detect from LANG.
language = "zh"
```

### Supported Languages

| Code | Language            |
| ---- | ------------------- |
| `en` | English (default)   |
| `zh` | Simplified Chinese  |

When `language` is unset, abtop auto-detects from `LANG` — any value starting with `zh` switches to Simplified Chinese, otherwise English.

## Key Bindings

| Key                | Action                               |
| ------------------ | ------------------------------------ |
| `↑`/`↓` or `k`/`j` | Select session                       |
| `Enter`            | Jump to session terminal             |
| `x`                | Kill selected session                |
| `X`                | Kill all orphan ports                |
| `t`                | Cycle theme                          |
| `1`–`5`            | Toggle panel visibility              |
| `Esc`              | Open/close config page               |
| `q`                | Quit                                 |
| `r`                | Force refresh                        |

## Library / JSON snapshot

abtop is also a library crate, so local tools can reuse its data-collection
layer in-process — no re-scanning, no subprocesses — and serialize the same
state the TUI renders.

```bash
abtop --json    # one-shot JSON snapshot for scripts
```

For long-running consumers, build an `App`, refresh it with
`App::tick_no_summaries()` (which never spawns `claude --print`, so it doesn't
touch your Claude quota), and call `App::to_snapshot(interval_ms)` to get a
JSON-serializable [`Snapshot`]:

```rust,no_run
use abtop::app::App;
use abtop::{config, theme::Theme};

let cfg = config::load_config();
let mut app = App::new_with_config_and_claude_dirs(
    Theme::default(), &cfg.hidden_agents, cfg.panels, &cfg.claude_config_dirs,
);
app.tick_no_summaries();
let json = serde_json::to_string(&app.to_snapshot(2_000)).unwrap();
```

`App` is not `Send` (it owns the collectors), so keep it on one thread and pass
the serialized JSON elsewhere. [abtop-web-ui](https://github.com/XKHoshizora/abtop-web-ui)
is a reference consumer: a local-first web dashboard built on exactly this API.

## Collector compatibility and diagnostics

OpenCode supports the legacy `session`/`message` schema and the OpenCode 2.1.4
`session_v2`/`session_message` schema. SQLite queries use `-readonly` with a bounded
busy timeout. Install the `sqlite3` CLI to enable database collection and run the
collector fixture tests. V2 token totals are read directly from the session row;
reasoning tokens are not added to output a second time. Lifetime token totals are
not used as current context usage, so OpenCode context percentage is unavailable
(represented as zero in the current snapshot model).

On Linux, a plain argument-free `opencode` terminal process is shown even if the
database is missing or has no matching session. Its row is `Unknown`, labeled
`OpenCode running; transcript unavailable`, and carries no inferred token totals.
It uses the same verified process observation as the Codex process-only fallback.
Service and daemon commands do not qualify for this fallback.

A directly matched OpenCode process shows its newest session per PID. A global
`opencode serve --service` process does not establish session ownership. With a
running service, non-archived, non-suspended v2 rows updated in the last 30 seconds
and not marked idle since that update appear as `Unknown`, with PID zero, no
process memory or children, and no kill or terminal-jump action. This is a recent
activity heuristic, not proof that the model is running. Older and idle unowned
rows stay hidden; a service does not make every historical session live. The
query considers at most 20 non-archived sessions, newest first.

On Linux, Codex CLI and App Server ownership uses open rollout file descriptors.
App Server FD discovery completes on the first tick, including `--once --json`.
A plain, argument-free `codex` process without an open rollout is also shown on
Linux, using its verified process cwd and kernel start time. The row is `Unknown`
and labeled `Codex running; transcript unavailable`; token/context values are
unavailable (represented as zero). It never borrows a recent historical rollout.
When an owned rollout becomes available, the normal transcript row replaces it.
The process-only fallback excludes App Servers, daemons, wrappers, and CLI
subcommands/options; those still need FD-backed discovery. Restart a running
abtop after rebuilding to load collector changes.

A recent, FD-owned App Server rollout does not require the `Codex Desktop`
originator label. App Server files older than 30 minutes remain excluded, even
if the server still holds them open for resume. A daemon without eligible
rollouts creates no session. Filesystem-only Desktop fallback still requires
that label and shows `Unknown` without confirmed ownership; recent unowned CLI
rollouts do not become live sessions merely because they were modified.

```bash
cargo run -- --diagnose       # Safe collection metadata, no generated summaries
cargo run -- --once --json    # Normal session snapshot; includes private local data
```

`--diagnose` prints the version and enabled collectors, symbolic source locations,
schema, process/candidate/accepted counts, aggregated filtering reasons, and
static error codes. It takes precedence over `--json` and `--once`. Expanded
private paths, SQL errors, database values, commands, and conversation text are
omitted. Claude currently reports only basic counts with `not_instrumented`;
detailed failure diagnostics cover OpenCode and Codex discovery.

Normal JSON snapshots include `collector_errors` only when collection fails.
The TUI also displays a collection failure message. Failed OpenCode refreshes
clear cached rows, and the diagnostic error persists until a successful refresh.
A missing source, an empty supported database, and an unsupported schema have
different diagnostic states. OpenCode filtering distinguishes idle, stale,
suspended, invalid-timestamp, and missing-ownership cases. See [the collector audit](docs/collector-audit.md)
for confirmed causes and remaining limits.

## Privacy

abtop reads local files and local process/open-file metadata only. No API keys, no auth. In the TUI and `--once` output, tool names and file paths are shown, but file contents and prompt text are never displayed. Session summaries are generated via `claude --print`, which makes its own API call — this is the only indirect network usage.

The JSON snapshot includes richer local dashboard data, including `summary`, `chat_messages`, working directories, config roots, tool-call previews, child process commands, token counts, and port metadata. Chat text is bounded and redacted by the collectors, but it is still derived from local transcripts and may contain sensitive project context. Treat JSON snapshots as local/private data and avoid writing them to shared logs or exposing them on a network without your own access controls.

## Acknowledgements

Huge thanks to [@tbouquet](https://github.com/tbouquet) for driving much of abtop's recent shape — themes, config overlay and panel toggles, session filtering, subagent tree view, the context window gauge with compaction detection, plus a steady stream of fixes and security hardening along the way.

## License

MIT
