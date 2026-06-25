# Decisions

Short log of meaningful, hard-to-reverse choices. Newest first.

## 2026-06-23 — Real names and section help

- **Provider titles are the only agent names.** Claude `aiTitle`/summary, Codex
  `thread_name`, and Hermes `sessions.title` are real names. When none exists,
  MOBI board shows provider + session id instead of inventing a project/folder name.
- **Rename only writes real provider titles.** The UI shows rename controls only
  when the backend can write back to the provider title source.
- **Expanded cards explain themselves.** Every expanded section has an info hover
  so operators can understand session identity, run burn, process trees, context
  capacity, and recent activity without extra documentation.
- **Hermes relationships are surfaced.** Hermes/Fugu parent/child rows are exposed
  as orchestrator/sub-agent metadata, with child rows inheriting project context
  from their parent when Hermes leaves `cwd` empty.

## 2026-06-19 — Live data slice (AMC-050 … AMC-070)

- **No dollar cost.** Cards show token counts only (input / output / cache). Estimating
  dollars from public list prices was considered and dropped at Karim's request — most
  local agents run on a subscription, so a dollar figure would be notional and misleading.
- **Liveness comes from file mtime**, not a heartbeat: `working` < 90s, `idle` < 10 min,
  hidden after 10 min. Simple, robust, zero-config, and it gives the "empty screen until
  an agent is active" behaviour for free.
- **Session names are provider-sourced.** Claude uses `aiTitle`/summary; Codex uses
  `thread_name` from `~/.codex/session_index.jsonl`; Hermes uses `sessions.title`.
  Missing provider titles stay missing rather than falling back to folders.
- **Adapter contract:** each tool has a pure `parse_session(path) -> Option<AgentSession>`
  that is unit-tested against committed fixtures with no access to the real home dir. The
  `Collector` owns scanning, the mtime cache, and liveness, so adapters stay pure.
- **Cursor is deferred.** Its session data lives in a VS Code SQLite store with no clean
  token log; a partial card would mislead. The adapter layer is pluggable for it later.
- **Codex tokens are best-effort.** Codex `token_count` payloads frequently carry a null
  `info`, so token totals may be 0. We read them when present and never block on them.
- **Multiple Codex homes.** The collector scans every Codex profile — the default
  `~/.codex` plus isolated profiles like `~/.codex-karim` (the "karimscodex" instance) —
  merging their `session_index.jsonl` name maps and de-duplicating sessions by id.
- **Privacy:** the app only reads local agent logs and never sends anything off the
  machine. v1 is observe-only — it never controls, pauses, or modifies agents.
- **Repo hygiene:** `tests/fixtures/` and `docs/{DECISIONS,KANBAN}.md` are force-tracked
  (the broad `tests/` and `docs/*` ignores otherwise exclude them).
