# Mobius Architecture

## Current Shape

```text
mobius/
├── docs/                       ARCHITECTURE, PROJECT_CONTEXT, DECISIONS, KANBAN
├── src-tauri/
│   ├── examples/snapshot.rs     dev helper: print the live snapshot get_sessions returns
│   └── src/
│       ├── app_paths.rs
│       ├── collector/
│       │   ├── mod.rs           Collector: scan agent logs, cache, liveness, merge
│       │   ├── adapters/
│       │   │   ├── mod.rs
│       │   │   ├── claude.rs     ~/.claude/projects/**/*.jsonl -> AgentSession
│       │   │   ├── codex.rs      ~/.codex rollout + session_index -> AgentSession
│       │   │   └── hermes.rs     ~/.hermes/state.db -> AgentSession + run/activity
│       │   ├── scanner.rs        OS process scanner -> process trees
│       │   ├── registry.rs       in-memory registry (kept, not wired in v1)
│       │   └── session.rs        AgentSession / Tokens / FileEvent / FileAction model
│       └── lib.rs               Tauri setup; get_sessions -> Collector::snapshot
├── src/
│   ├── components/
│   │   ├── agentCard.ts          session card (real title, run/process/activity/help)
│   │   ├── contextGauge.ts       compact context pressure display
│   │   ├── processTree.ts        process tree renderer + kill plan helper
│   │   ├── fileLog.ts            file-activity list with status tags
│   │   ├── toolLogo.ts           per-tool SVG mark
│   │   ├── topBar.ts             active / working / total-tokens metrics
│   │   └── format.ts            shared escapeHtml / basename / formatTokens
│   ├── main.ts                  poll get_sessions every 1.5s, reconcile cards
│   ├── styles.css
│   └── types.ts                 frontend mirror of the serialized AgentSession
├── index.html
└── tests/fixtures/{claude,codex}/  committed, read-only sample logs
```

## Data Flow

```text
~/.claude/projects/**/*.jsonl  ─┐
~/.codex/sessions/**/*.jsonl    ├─►  Collector::snapshot(now)
~/.codex/session_index.jsonl    │       • scan files modified within the active window
~/.hermes/state.db              ┘       • parse each via its adapter (cache by mtime)
                                       • apply liveness from mtime/process scan
                                       • attach real provider titles, run data, process trees
   ►  Vec<AgentSession>  (serde camelCase)
   ►  get_sessions  Tauri command (polled every 1.5s by the webview)
   ►  main.ts keyed reconciler  ►  top metrics + agent cards + run/process/activity panels
```

## Module Responsibilities

- `collector/session.rs`: shared `AgentSession`, `Tool`, `Status`, `Tokens`, `FileEvent`,
  `FileAction`, `RunStats`, process tree, and Hermes connection metadata. Serialized
  camelCase for the UI.
- `collector/adapters/claude.rs`: pure `parse_session(path)` for a Claude Code log —
  tokens, model, branch, first prompt, file-activity log, current action, and a
  derived title.
- `collector/adapters/codex.rs`: best-effort `parse_session(path)` for a Codex rollout
  log, plus `load_thread_names(index)` for the session id → name join. Scanned across
  every Codex home — `~/.codex` and isolated profiles like `~/.codex-karim`.
- `collector/adapters/hermes.rs`: read-only SQLite adapter for Hermes/Fugu sessions,
  including run telemetry, child/sub-agent relationships, and lazy activity reconstruction.
- `collector/mod.rs`: `Collector` scans the Claude tree and every Codex home, reads Hermes,
  caches parsed sessions by file mtime, computes live status, merges process trees, and
  sorts newest-first.
- `lib.rs`: Tauri app; exposes `get_sessions` returning `Collector::snapshot(now)`.
- `src/components/*`: pure string renderers (card, file log, process tree, context gauge,
  tool logo, top bar).
- `src/main.ts`: polls `get_sessions` and reconciles cards by session id so new agents
  animate in and existing cards update in place.

## Liveness, Naming, And Activity

- Status from file mtime: `working` < 90s, `idle` < 10 min, hidden afterwards. This is
  what produces the "black screen until an agent is running" behaviour.
- Session name: only provider-sourced titles count. Claude = `aiTitle`/summary, Codex =
  `thread_name`, Hermes = `sessions.title`. If missing, the UI renders provider + session
  id and labels the provider title as unavailable.
- Expanded cards show first prompt, run telemetry, matched process trees, context
  capacity, and recent file/command activity. Each section header has a small info
  hover explaining what the section means and how to use it.
- Hermes parent/child rows become orchestrator/sub-agent metadata; child rows inherit the
  parent project path when Hermes leaves `cwd` empty.

## Planned / not in v1

- Cursor adapter (its data is in a VS Code SQLite store with no clean token log).
- Incremental byte-offset tailing (today an active file is re-parsed when its mtime
  changes; cheap because only recently-active files are scanned).
- Dollar cost is intentionally out of scope (see `docs/DECISIONS.md`).
