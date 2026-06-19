# Agent Mission Control Project Context

> Living context file for future agents and future chats.
> Update this file every time the project direction, architecture, ticket plan, or implementation state changes.

## Quick Read

Agent Mission Control is a macOS desktop app for seeing all local AI coding agents in one place: Claude Code, Codex, and Cursor.

In plain English: it is a live dashboard for "what are my coding agents doing right now?"

The owner is Karim, who is a non-technical developer and vibe coder with crypto experience. Build the real thing with serious engineering, but explain it in simple language. The codebase should feel like a startup foundation: organized, understandable, easy to extend, and easy for future agents to navigate.

## Source Of Truth

- Product design: `docs/superpowers/specs/2026-06-19-agent-mission-control-design.md`
- Implementation plan: `docs/superpowers/plans/2026-06-19-agent-mission-control.md`
- Ticket board: `docs/KANBAN.md`
- Decision log: `docs/DECISIONS.md`
- Architecture map, once implementation starts: `docs/ARCHITECTURE.md`

When these disagree, treat the design spec as the product source of truth, then update the other docs so they match.

## Current State

- Tauri v2 + Vite/TypeScript app launches and shows **live** local agents.
- The collector reads real logs: Claude Code (`~/.claude/projects`) fully, Codex
  (`~/.codex`) best-effort. Cursor is deferred.
- `get_sessions` returns `Collector::snapshot(now)`: currently-active sessions (by file
  mtime) with tokens (in/out/cache), model, branch, a derived name + full session id, live
  status, and a file-activity log. The webview polls every 1.5s and reconciles cards.
- No dollar cost — token counts only. Observe-only. See `docs/DECISIONS.md`.
- Tests: `cargo test` (26) and `npm test` (10) green; `tsc` clean.
- `docs/ARCHITECTURE.md` maps the structure; `docs/KANBAN.md` tracks the slices.

## Product Goal

Build a macOS app that shows currently running AI coding agents in one clean dashboard.

The app is observe-only for v1. It shows status, project, branch, model, token usage, and estimated cost where available. It does not control agents, pause them, merge branches, or take actions on their behalf.

## Non-Negotiables

1. One obvious home for every file.
2. Runtime junk never lives in the repo.
3. Test fixtures are committed, read-only examples.
4. Meaningful decisions go into `docs/DECISIONS.md`.
5. The project map stays current.
6. Explain as we go in plain language.
7. No pushing or opening PRs/MRs unless Karim explicitly says so in that message.

## Architecture In One Page

The app has two main halves:

- Rust/Tauri backend: the collector. It watches agent activity, reads hooks/files, keeps a live registry of sessions, checks whether processes are still alive, calculates cost, and sends updates to the UI.
- TypeScript/Vite frontend: the dashboard. It receives normalized session data and renders cards, status pills, counters, and simple filters.

Everything gets normalized into one shared shape called `AgentSession`.

That shape includes:

- `id`: stable session id
- `tool`: Claude, Codex, or Cursor
- `pid`: process id when known
- `project_path`: repo or working folder
- `branch`: git branch
- `model`: model name when known
- `status`: starting, working, idle, waiting input, ended, or dead
- `current_action`: one-line description of what the agent is doing
- `started_at` and `last_event_at`: timestamps
- `tokens`: input, output, cache
- `cost_usd`: dollar estimate when possible

## Development Style

Build vertically.

That means each meaningful ticket should include a small piece of every layer needed to prove the feature works:

- fixture or input data
- backend parsing/state
- UI display if user-visible
- tests
- docs update

Avoid building the whole backend first, then the whole UI later. That creates hidden integration risk. A vertical slice lets us see real behavior early and keeps the project understandable.

Example:

- Bad slice: "Build all adapters."
- Better slice: "Show one Claude session from fixture to backend model to UI card."

## Project Folder Rules

Expected final layout:

```text
agent-mission-control/
├── README.md
├── docs/
│   ├── ARCHITECTURE.md
│   ├── DECISIONS.md
│   ├── KANBAN.md
│   ├── PROJECT_CONTEXT.md
│   └── superpowers/
├── src-tauri/
├── src/
├── hooks/
├── scripts/
└── tests/
    └── fixtures/
```

Runtime app data belongs outside the repo:

```text
~/Library/Application Support/AgentMissionControl/
├── state.sqlite
├── logs/
└── pricing-cache.json
```

## How To Update This File

At the end of every meaningful pass:

1. Update `Current State`.
2. Add a short entry to `Pass Log`.
3. If a decision changed, add it to `docs/DECISIONS.md`.
4. If tickets changed, update `docs/KANBAN.md`.
5. If source folders changed, update `docs/ARCHITECTURE.md`.

Keep this file short. Link to detailed docs instead of copying everything here.

## Pass Log

### 2026-06-19 - Project brain and Kanban created

- Added this living context file so future chats can quickly understand the project.
- Added `docs/KANBAN.md` to track implementation as vertical slices.
- Recorded the decision to build vertically in `docs/DECISIONS.md`.

### 2026-06-19 - AMC-040 fake session card implemented

- Added an in-memory registry and `get_sessions` Tauri command seeded with one fake Claude session.
- Added vanilla TypeScript renderers for the top metrics and first session card.
- Added `docs/ARCHITECTURE.md` as the current project map.

### 2026-06-19 - Live data slice (AMC-050 … AMC-070)

- Extended the model with file-activity events and a session title; dropped dollar cost.
- Built the live Claude adapter and a best-effort Codex adapter; the `Collector` scans the
  real agent logs, caches parsed sessions by file mtime, and applies liveness.
- Redesigned the card: per-tool logo, name + full session id, In/Out/Cache token split, and
  a live file-activity log. The webview polls `get_sessions` every 1.5s.
- Verified against real data — this very build session shows up in its own dashboard.
