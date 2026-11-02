# Changelog

## Unreleased

- Support OpenCode 2.1.4 session_v2 databases while retaining legacy queries.
- Read cumulative OpenCode totals without multiplying usage by message count;
  exclude archived rows and stop presenting lifetime totals as current context.
- Surface recent non-idle global-service activity as Unknown without assigning
  the daemon's PID, resources, or process controls to sessions.
- Discover recent Codex App Server FD-owned rollouts with non-Desktop originators.
- Complete Linux App Server FD scans on the first collection tick for JSON output.
- Add privacy-preserving --diagnose output, collection errors in JSON snapshots,
  and a TUI failure message; clear stale OpenCode cache after failed refreshes.
- Add SQLite, process-FD, parser, privacy, and end-to-end JSON regression tests.
- Show argument-free Linux Codex CLI processes before an open rollout is available,
  with Unknown activity and explicitly unavailable transcript metrics.
- Show verified plain OpenCode terminal processes when the database is missing or
  no session matches, using Unknown activity and unavailable transcript metrics.
