# Agent Mission Control Architecture

## Current Shape

```text
agent-mission-control/
├── docs/
│   ├── ARCHITECTURE.md
│   └── PROJECT_CONTEXT.md
├── src-tauri/
│   └── src/
│       ├── app_paths.rs
│       ├── collector/
│       │   ├── mod.rs
│       │   ├── registry.rs
│       │   └── session.rs
│       └── lib.rs
├── src/
│   ├── components/
│   │   ├── agentCard.ts
│   │   ├── agentCard.test.ts
│   │   └── topBar.ts
│   ├── main.ts
│   ├── styles.css
│   └── types.ts
└── index.html
```

## Data Flow

```text
seeded AgentSession
  -> Registry managed by Tauri
  -> get_sessions command
  -> TypeScript AgentSession[]
  -> top metrics + session card
```

## Module Responsibilities

- `src-tauri/src/collector/session.rs`: shared `AgentSession`, `Tool`, `Status`, and `Tokens` model used by the backend and serialized for the UI.
- `src-tauri/src/collector/registry.rs`: in-memory session registry keyed by stable session id. It supports seeded fake sessions for AMC-040 and later real adapter updates.
- `src-tauri/src/lib.rs`: Tauri application setup. It manages the seeded registry and exposes `get_sessions` to the webview.
- `src/types.ts`: frontend mirror of the serialized `AgentSession` payload.
- `src/components/agentCard.ts`: pure card renderer for a single session.
- `src/components/topBar.ts`: pure metric renderer for active and working counts plus live estimated spend.
- `src/main.ts`: webview entrypoint. It calls `get_sessions`, renders metrics, and switches between empty and session-grid states.

## Planned

- Real hook ingest, heartbeat, pricing, and per-tool adapters are still planned. The current fake session proves the command bridge and card rendering before those layers arrive.
