# Agent Mission Control Architecture

## Current Shape

```text
agent-mission-control/
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
│       │   │   └── codex.rs      ~/.codex rollout + session_index -> AgentSession
│       │   ├── registry.rs       in-memory registry (kept, not wired in v1)
│       │   └── session.rs        AgentSession / Tokens / FileEvent / FileAction model
│       └── lib.rs               Tauri setup; get_sessions -> Collector::snapshot
├── src/
│   ├── components/
│   │   ├── agentCard.ts          one session card (logo, name, id, tokens, file log)
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
~/.codex/session_index.jsonl  ──┘      • scan files modified within the active window
                                       • parse each via its adapter (cache by mtime)
                                       • apply liveness from mtime
                                       • override Codex titles with thread_name
   ►  Vec<AgentSession>  (serde camelCase)
   ►  get_sessions  Tauri command (polled every 1.5s by the webview)
   ►  main.ts keyed reconciler  ►  top metrics + agent cards + live file logs
```

## Module Responsibilities

- `collector/session.rs`: shared `AgentSession`, `Tool`, `Status`, `Tokens`, `FileEvent`,
  `FileAction` model. Serialized camelCase for the UI.
- `collector/adapters/claude.rs`: pure `parse_session(path)` for a Claude Code log —
  tokens, model, branch, file-activity log, current action, and a derived title.
- `collector/adapters/codex.rs`: best-effort `parse_session(path)` for a Codex rollout
  log, plus `load_thread_names(index)` for the session id → name join.
- `collector/mod.rs`: `Collector` scans both agent log trees, caches parsed sessions by
  file mtime, computes live status, merges, and sorts newest-first.
- `lib.rs`: Tauri app; exposes `get_sessions` returning `Collector::snapshot(now)`.
- `src/components/*`: pure string renderers (card, file log, tool logo, top bar).
- `src/main.ts`: polls `get_sessions` and reconciles cards by session id so new agents
  animate in and existing cards update in place.

## Liveness & naming

- Status from file mtime: `working` < 90s, `idle` < 10 min, hidden afterwards. This is
  what produces the "black screen until an agent is running" behaviour.
- Session name: Claude = summary → first real prompt → slug → project folder; Codex =
  `thread_name` → project folder.

## Planned / not in v1

- Cursor adapter (its data is in a VS Code SQLite store with no clean token log).
- Incremental byte-offset tailing (today an active file is re-parsed when its mtime
  changes; cheap because only recently-active files are scanned).
- Dollar cost is intentionally out of scope (see `docs/DECISIONS.md`).
