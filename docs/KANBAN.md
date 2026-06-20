# Kanban

Vertical slices: each ships fixture → backend → UI → tests → docs.

## Done

- **AMC-040** — Seeded fake Claude session → `get_sessions` → one visible card.
- **AMC-050** — Model: `FileEvent` / `FileAction` / `recentFiles` / `title`; drop `cost_usd`
  (Rust + TS mirror). Top bar shows total live tokens.
- **AMC-051** — Live Claude adapter: tokens, model, branch, file-activity log
  (tool → action map incl. Bash redirect detection), current action, derived title.
- **AMC-052** — `Collector` wired into `get_sessions` against real `~/.claude/projects`;
  mtime cache + liveness; `main.ts` live-polls and reconciles cards (pop-in animation).
- **AMC-053** — Per-tool logos, session name heading + full id chip, In/Out/Cache tiles,
  and the file-activity log UI with status tags.
- **AMC-060** — Best-effort Codex adapter: rollout parsing + `thread_name` join; Collector
  scans `~/.codex/sessions` too.
- **AMC-070** — Docs pass (this file, ARCHITECTURE, DECISIONS, PROJECT_CONTEXT) + live
  verification.

## Backlog

- **AMC-08x** — Cursor adapter (VS Code SQLite store).
- **AMC-08x** — Incremental byte-offset tailing for active files.
- **AMC-08x** — Tauri event push instead of UI polling.
- **AMC-08x** — Packaged/signed `.app` bundle and auto-update.
