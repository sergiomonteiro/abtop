# Collector audit

Audited baseline: `4b96568`, package version `0.5.5`.
Development branch: `fix/linux-session-discovery`.
The checkout preserves the upstream MIT license and copyright notice.

## Confirmed causes and implemented changes

### OpenCode database compatibility

The original `query_sessions` queried only `session`, `message`, and `project`.
Read-only inspection of the installed database confirmed `session_v2` and
`session_message`. Its model is a JSON object containing `id` and `providerID`.
A temporary v2 fixture failed against the original implementation before changes.

The collector now discovers table names through `sqlite_master`, selects the v2
query when available, and retains the legacy query otherwise. V2 totals come
from session rows; counting assistant messages cannot multiply token usage.
Reasoning tokens are not separately added to output. Archived rows are excluded.
SQLite runs with `-readonly` and a one-second busy timeout. The fixture tests
confirm database bytes are unchanged by collection.

Lifetime usage cannot establish current context usage. OpenCode now reports zero
for unavailable context percentage instead of deriving it from lifetime totals.
The current normalized model cannot distinguish unavailable percentage from zero.
Database-reported cost is not yet exposed; no cost estimate is introduced.

### OpenCode global services

The original collector required cwd/command matching and claimed each PID only
once. This misses service-hosted sessions in other projects. Assigning all rows
to the daemon would instead falsely revive historical sessions.

Service commands are now excluded from direct ownership matching. With a live
service, a non-archived, non-suspended v2 row updated within the last 30 seconds,
not in the future and not marked idle at or after that update, can appear as
`Unknown`. Its PID is zero, it has no process memory/children, and kill/jump
controls are disabled. Multiple recent projects can appear without sharing a
claimed PID. Idle and older unowned rows stay hidden. Legacy rows still require
process ownership because they lack the v2 lifecycle evidence.

This is deliberately a recent-activity heuristic. Metadata changes can qualify;
long operations without DB updates can disappear after 30 seconds. It does not
prove model execution or identify the owning client. At most 20 non-archived
rows are considered. No new authoritative active/idle/historical enum is claimed.

### Codex App Server discovery

The original code recognized App Server processes but required
`session_meta.originator == "Codex Desktop"` for FD-owned rollouts too. A
regression fixture with a recent open non-Desktop rollout reproduced zero
sessions before the fix. FD ownership now permits other originators; loading
still requires valid session metadata. Desktop filesystem-only fallback keeps
its originator requirement and uses `Unknown` without proven ownership.

Linux App Server FD discovery previously ran asynchronously, while `--json`
serialized after one tick. Linux now reads these `/proc` descriptors synchronously
on the first tick. macOS retains background scanning to avoid blocking on lsof.
A first-tick regression test opens a temporary rollout FD in the test process.

FD-owned App Server files still require mtime under 30 minutes to avoid treating
resume descriptors as active sessions. A daemon without an eligible rollout
creates no session. MCP suppression, duplicate exclusion, and stale-file filters
remain in place. Recent unowned CLI files are not promoted to live sessions.

A parser fixture covers repeated cumulative token counts, unknown events,
compaction, and a partially written tail. Existing cumulative totals are replaced,
not summed; cached input remains a subset of reported input. No parser root cause
was demonstrated for the original production incident.

### Follow-up: plain Codex CLI omitted without an open rollout

The user's new CLI session did not appear despite running the branch build.
Host inspection confirmed two argument-free `codex` processes with zero open
rollout descriptors. The original CLI path omitted both. A Linux integration
fixture reproduced the missing row before the fix.

The collector now surfaces these verified processes even before the sessions
root exists. It uses cwd and kernel start time from `/proc`, a PID/start-ticks
identity, and `Unknown` activity with `Codex running; transcript unavailable`.
Usage/context values remain unavailable (zero in the current model). No historical
rollout is guessed or assigned. The normal FD-backed row replaces the process-only
row when available; process exit removes it. Daemons, App Servers, wrappers, and
commands with arguments are excluded from this narrow fallback.

The integration test covers unowned rollouts with nonzero historical usage,
process exit, and a bare CLI with an owned rollout appearing exactly once.
Host diagnostics before/after rebuilding changed accepted Codex rows from one to
three, with four processes, two process-only rows, one stale-rollout exclusion,
and zero errors. The running UI binary must be restarted after rebuilding.
The installed executable differs from the branch build and was not overwritten.

### Follow-up: reported OpenCode session missing

Host diagnostics found one OpenCode global service and zero accepted OpenCode
sessions. Inspecting its actual open database confirmed the default database,
seven v2 rows, and a latest update approximately 42 hours old. All seven rows
were idle at their last update. No separate OpenCode client was identified by
the expanded executable-name check. This does not establish where the user's
reported open session is running; terminal/desktop/browser and remote/container
context must be clarified before changing ownership rules.

The service filter now reports separate idle, stale, suspended, invalid timestamp,
and missing lifecycle-evidence reasons instead of one generic ownership/activity
reason. A regression test verifies these distinctions. Live Codex process-only
rows can remain visible while idle; they are not evidence of model activity.
No historical OpenCode row is promoted merely because the daemon is running.

### Follow-up: plain OpenCode terminal launch

The user confirmed launching `opencode` without arguments. The host checks still
identified only the original background service, not the user's reported terminal
client. A separate isolated launch established that a plain terminal client can
exist before any database row is available. That launch later failed because its
managed service port was already occupied; it was not a successful full TUI
reproduction. Only temporary XDG data/config/state were used, no prompts were sent,
and cleanup verified no isolated OpenCode processes remained. The original
service was left running.

A synthetic plain-process integration test independently reproduced the collector
omission with no database. The fix observes verified argument-free OpenCode CLI
processes even when the database is absent or contains only unrelated history.
Rows use `Unknown` and `OpenCode running; transcript unavailable`, not guessed
historical tokens. This shares the Linux process observer with Codex and does not
promote service/daemon processes to sessions. Database-backed rows still take
precedence for a matched PID. The integration test verifies no-database and
unrelated-history cases, zero inferred usage, and removal after process exit.
Linux process fixtures use executable symlinks to avoid copy/exec ETXTBSY races.

This closes a verified discovery gap. The user's specific terminal still requires
validation after restarting abtop; its client process was not located by the host
checks, so the actual reported session is not claimed as confirmed detected.

### Collection diagnostics

OpenCode SQL failures previously became empty results or silently preserved old
cached rows. Static error codes now survive fast ticks until a successful refresh;
failed refreshes clear the session cache. Missing sources, unknown schemas,
unreadable directories, failed queries, invalid rollout metadata, and legitimate
empty results have distinct states or codes.

`--diagnose` returns version, enabled collectors, symbolic sources, schema,
process/candidate/accepted counts, aggregated filtering reasons, and errors.
It takes precedence over `--json` and `--once` and never starts summary jobs.
No SQL stderr, database values, raw commands, expanded private paths, or transcript
contents are included. Claude reports basic counts as `not_instrumented`; detailed
failure instrumentation currently covers OpenCode and Codex discovery.

Normal snapshots add `collector_errors` only on failure. Both compact and wide
TUI footers persistently indicate collection failure and point to `--diagnose`.
One collector's failure does not prevent others from returning sessions.

## Validation

- Baseline: 223 tests including doctests passed.
- Regression: OpenCode v2 query and non-Desktop App Server fixture each failed
  before their corresponding fix.
- Updated suite covers SQLite schemas/lifecycle/cache recovery, FD ownership,
  stale and MCP suppression, malformed rollouts, token accounting, PID-zero
  controls, TUI failure visibility, and CLI privacy/JSON behavior.
- End-to-end JSON tests use temporary databases and a harmless `sleep` executable
  named `opencode` to establish actual Linux cwd evidence without an AI agent.
- `cargo test --quiet`: 245 tests including integration tests and doctests pass.
- `cargo clippy --all-targets -- -D warnings`: passes.
- `cargo build`: passes.
- Changed modules are formatted. `cargo fmt --check` still identifies baseline
  formatting issues in `collector/claude.rs` and `collector/mcp.rs`; unrelated
  formatting is left unchanged.
- Interactive TUI launched and exited normally in a PTY with no visible agents.
- Against the installed database, `--diagnose` successfully reads seven v2
  candidates with zero errors. No matching agent processes are visible from this
  execution environment, so none are accepted as live sessions. This validates
  schema reading, not end-to-end discovery of the user's actual running agents.

Safe observed OpenCode diagnostic excerpt:

```json
{
  "agent": "opencode",
  "schema": "session_v2",
  "processes": 0,
  "candidates": 7,
  "accepted": 0,
  "state": "no_active_sessions",
  "skipped": { "no_process_ownership_or_recent_activity": 7 },
  "errors": []
}
```

## Build and run

```bash
cargo build
cargo run -- --diagnose
cargo run -- --once --json
cargo run
```

SQLite CLI is required for OpenCode collection and database fixture tests.
Normal JSON snapshots contain private local session details; diagnostic output
is the appropriate output to share when investigating discovery.

## Remaining validation and follow-up

Host diagnostics now confirm Codex process-only and App Server discovery. Validate
UI behavior after restarting the user's binary, exercise OpenCode activity, and test
long-running/idle transitions. Check sustained CPU/memory overhead with real
large transcripts and many App Server descriptors; this has not been measured.

Future work includes stronger service ownership evidence, database-reported cost,
availability/source metadata for metrics, column-level schema evolution, detailed
Claude/parser diagnostics, and an explicit history model. These changes do not
introduce a daemon, cloud infrastructure, telemetry, or database migrations.
