# abtop roadmap

Status: proposed implementation plan, agreed for planning purposes.

## GitHub tracking

1. [Collector reliability and real-environment validation](https://github.com/sergiomonteiro/abtop/issues/2)
2. [Trustworthy metrics and session states](https://github.com/sergiomonteiro/abtop/issues/3)
3. [Session navigation, filtering, and collection health](https://github.com/sergiomonteiro/abtop/issues/4)
4. [Privacy-preserving activity timeline](https://github.com/sergiomonteiro/abtop/issues/5)
5. [Opt-in local metrics history and comparisons](https://github.com/sergiomonteiro/abtop/issues/6)

## Direction

Evolve abtop from detecting agents into making their activity understandable and
trustworthy. Keep the product local-first, lightweight, and read-only with respect
to agent data. Do not introduce a mandatory daemon, remote backend, account, or
telemetry.

For now, develop this fork for personal daily use under the existing abtop name.
Prioritize reliable discovery, trustworthy metrics, and comfortable daily use.
Revisit a distinct name and public distribution only after that maturity gate;
renaming and distribution migrations are not part of the current milestones.

This roadmap describes milestones, not release versions. Assign semver versions
when preparing releases; the older version labels in the handover are conceptual.

## 1. Consolidate reliability

- Validate the corrected build against the user's actual running terminals.
- Document supported agent versions and known discovery limitations.
- Complete collector diagnostics, including currently uninstrumented paths.
- Distinguish legitimate empty results from collection failures.
- Preserve regression coverage for schema changes, process ownership, and token
  accounting.

Acceptance: real-environment discovery is validated, and collection failures are
explicit rather than silently presented as zero sessions.

The collector audit records remaining validation gaps; passing fixture tests alone
does not close them. See [collector-audit.md](collector-audit.md).

## 2. Make metrics honest — next implementation milestone

Milestone: **Trustworthy metrics and session states**.

After the real-environment validation gate, introduce a normalized,
evidence-aware session model:

- Represent unavailable metrics explicitly instead of using zero as a sentinel.
- Distinguish observed data, estimates, and unavailable values.
- Separate process presence from evidence of session activity.
- Separate recent service activity from verified session ownership.
- Expose metric sources and availability consistently in JSON and the TUI.
- Keep kill and terminal-jump controls restricted to verified ownership.
- Preserve correct cumulative token accounting and avoid treating lifetime usage
  as current context usage.
- Add abstractions incrementally where they simplify existing collectors, without
  a broad rewrite.

Acceptance: every displayed number and activity state has supporting evidence or
clearly communicates uncertainty. Unknown context must not appear as 0% context.

## 3. Improve daily usability

- Add filters by project, agent, and status.
- Clarify session details, ownership evidence, and collection health.
- Make it easy to identify which agent is working, in which project, and on what
  evidence.

Acceptance: users can distinguish active work, idle processes, unknown activity,
and collection problems without inspecting raw source files.

## 4. Add an activity timeline

- Surface tool calls, errors, retries, and compactions when local sources expose
  them reliably.
- Document source-specific gaps instead of inventing missing events.
- Avoid collecting or displaying full conversation content or tool file contents.

Acceptance: users can understand session progress without opening each terminal,
while preserving privacy and making incomplete event coverage explicit.

## 5. Add optional local history

- Make retention opt-in.
- Store normalized metrics locally, not conversation content.
- Bound storage and provide a clear retention policy.
- Enable comparisons of token consumption and activity across sessions.

Acceptance: historical comparisons remain local, bounded, and understandable,
with no prompt or response retention.

## Deferred work and guardrails

- Defer cloud dashboards, additional agent integrations, cost estimates, and
  alerts until measurements and activity semantics are trustworthy.
- Do not add OpenCode quota without a reliable account-level provider rate-limit
  source.
- Do not modify or migrate agent databases or transcripts.
- Keep collection failures isolated so one broken collector does not disable the
  others.
- Validate each milestone with targeted regression tests and relevant JSON/TUI
  checks before treating it as complete.
