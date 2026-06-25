# Hermes Adapter for MOBI board — Design

**Date:** 2026-06-22
**Status:** Approved (pending implementation plan)
**Author:** Karim + Claude

## Goal

Add a third local-agent adapter to MOBI board — **Hermes** (Nous Research's
desktop agent app) — alongside the existing Claude and Codex adapters, so
Hermes coding sessions appear as agent cards in the dashboard with correct,
live information.

## Background

MOBI board is a live local-agent dashboard (Tauri v2). It detects running AI
coding agents by reading their local session data:

- **Claude:** one process + one JSONL file per session under
  `~/.claude/projects/**/*.jsonl`.
- **Codex:** one process + one `rollout-*.jsonl` per session under
  `~/.codex/sessions/` (+ `~/.codex-karim/sessions/`), with names in
  `session_index.jsonl`.

Hermes is architecturally different. It is **one long-running daemon**
(observed as a Python process running `~/.hermes/hermes-agent/venv/bin/hermes`)
that serves **many sessions from a single SQLite database** at
`~/.hermes/state.db` (WAL mode — `state.db-wal` / `state.db-shm` present).

The `~/.hermes/sessions/request_dump_*.json` files are raw API request dumps
(debug artifacts), **not** the source of truth. The SQLite `sessions` table is.

### Observed schema (relevant columns)

`sessions` table:

```
id TEXT PRIMARY KEY, source TEXT, user_id TEXT, model TEXT,
system_prompt TEXT, parent_session_id TEXT,
started_at REAL, ended_at REAL, end_reason TEXT,
message_count INTEGER, tool_call_count INTEGER,
input_tokens INTEGER, output_tokens INTEGER,
cache_read_tokens INTEGER, cache_write_tokens INTEGER, reasoning_tokens INTEGER,
cwd TEXT, estimated_cost_usd REAL, actual_cost_usd REAL,
title TEXT, api_call_count INTEGER,
handoff_state TEXT, handoff_platform TEXT,
archived INTEGER DEFAULT 0
```

`messages` table: `id, session_id, role, content, tool_calls, tool_name,
timestamp REAL, token_count, ...`.

Timestamps (`started_at`, `ended_at`, `messages.timestamp`) are **epoch
seconds (float)**. MOBI board uses **milliseconds since epoch** — convert.

Real data sample (this machine): 7 sessions, all `source='cli'`, model
`fugu-ultra`, `cwd=/Users/karimbaba`, auto-generated titles, 1 active
(`ended_at IS NULL`). Schema supports subagents (`parent_session_id`) and
platform handoffs (`handoff_platform`), but none present yet.

## Scope decision

**Surface only `source='cli'` sessions with a non-null `cwd`.** Hermes is
multi-surface (Telegram/Discord/Slack/WhatsApp/Email/CLI); restricting to CLI
sessions with a working directory keeps MOBI board a focused local-coding-agent
dashboard, consistent with Claude/Codex. Other sources are ignored in v1.

## Design

### 1. Integration shape — DB-backed collector branch

The existing per-file `parse_session(path) -> Option<AgentSession>` shape does
not fit a one-daemon-many-sessions DB. Instead introduce:

```rust
// adapters/hermes.rs
pub fn snapshot_sessions(db_path: &Path, active_window_ms: i64) -> Vec<AgentSession>
```

It opens the DB read-only and returns all qualifying sessions in one query.
The collector calls it once per snapshot (instead of walking files) and merges
the results into the same session list as Claude/Codex. This is an **additive**
branch — it does not change how the Claude/Codex adapters work.

### 2. SQLite access — read-only, WAL-aware

- Add `rusqlite` with the **`bundled`** feature (SQLite compiled in; no system
  dependency, deterministic build).
- Open with `OpenFlags::SQLITE_OPEN_READ_ONLY`. WAL mode means our reads never
  block Hermes's writes and vice-versa.
- **Strictly read-only.** MOBI board never writes to Hermes's live DB. Consequence:
  no rename support for Hermes in v1 (`can_rename = false`). Writing `title`
  into a daemon-owned WAL DB risks conflicts/corruption and is not worth it.

### 3. Query

```sql
SELECT s.id, s.title, s.model, s.cwd, s.started_at, s.ended_at,
       s.message_count, s.tool_call_count,
       s.input_tokens, s.output_tokens,
       s.cache_read_tokens, s.cache_write_tokens, s.reasoning_tokens,
       (SELECT MAX(m.timestamp) FROM messages m WHERE m.session_id = s.id) AS last_msg_at
FROM sessions s
WHERE s.source = 'cli' AND s.cwd IS NOT NULL AND s.archived = 0
ORDER BY s.started_at DESC;
```

`last_event_at` = `last_msg_at` if present, else `ended_at`, else `started_at`.

### 4. Field mapping → `AgentSession`

| AgentSession field | Hermes source |
|---|---|
| `tool` | new `Tool::Hermes` |
| `id` | `sessions.id` |
| `project_path` | `cwd` |
| `branch` | `None` (not tracked) |
| `model` | `model` (e.g. `fugu-ultra`) |
| `title` / `title_source` | `title` → `Provider`; fallback per existing convention |
| `can_rename` | `false` (read-only) |
| `tokens.input` | `input_tokens` |
| `tokens.output` | `output_tokens` |
| `tokens.cache` | `cache_read_tokens + cache_write_tokens` |
| `started_at` | `started_at` (seconds → ms) |
| `last_event_at` | derived `last_event_at` (seconds → ms) |
| `status` | derived (see §5) |
| `pid` | the Hermes daemon pid (shared across all Hermes sessions) |
| `recent_files` | **v2** — needs parsing `messages.tool_calls` JSON |
| `context` (Layer-2 breakdown) | **v2** — v1 uses DB token totals; skip tiktoken occupancy reconstruction |

### 5. Liveness / status model

Hermes has no per-session process, so per-process liveness (used for
Claude/Codex) is replaced by:

- **Daemon gate:** is the Hermes daemon alive? Detect by scanning processes
  for `.hermes/hermes-agent/venv/bin/hermes`. If not running, surface no live
  Hermes cards.
- **Recency filter:** apply the collector's existing `active_window_ms` to
  `last_event_at`, identical to the other adapters, so old sessions age out the
  same way.
- **Working vs Idle:** `ended_at IS NULL` + recent activity → Working;
  ended-recently or quiet-but-within-window → Idle; outside window → dropped.

### 6. Touchpoints

**New file:**

- `src-tauri/src/collector/adapters/hermes.rs`

**Modified:**

- `src-tauri/src/collector/adapters/mod.rs` — `pub mod hermes;`
- `src-tauri/src/collector/session.rs` — add `Hermes` to the `Tool` enum.
- `src-tauri/src/collector/mod.rs` — add the Hermes DB path to `new()`/`build()`,
  a daemon-liveness scan, a call to `hermes::snapshot_sessions`, and merge of
  results into the snapshot list.
- `src-tauri/src/collector/liveness.rs` — Hermes daemon detector
  (`hermes_daemon_pid() -> Option<i32>` or equivalent).
- `src-tauri/Cargo.toml` — add `rusqlite` with the `bundled` feature (pin to a
  current release at implementation time, e.g. `rusqlite = { version = "0.32", features = ["bundled"] }`).
- Frontend: `src/types.ts` (Hermes in the tool union) and
  `src/components/agentCard.ts` (icon/label/styling so it renders as a third
  agent).

### 7. Testing

- Commit a tiny fixture `state.db` and unit-test `snapshot_sessions` for:
  - `source='cli'` filter and `cwd IS NOT NULL` filter,
  - token mapping (incl. cache = read + write),
  - seconds → milliseconds conversion,
  - `last_event_at` fallback chain,
  - status derivation (active vs ended vs aged-out).
- Mirrors the existing fixture-driven adapter tests for Claude/Codex.

## Out of scope (v1)

- `recent_files` parsed from `messages.tool_calls`.
- Layer-2 context-window occupancy breakdown (tiktoken reconstruction).
- Rename support for Hermes sessions.
- Non-CLI sources (Telegram, Discord, Slack, WhatsApp, Email).
- Subagent hierarchy (`parent_session_id`) and platform-handoff display.

## Open questions

None blocking. v2 candidates (recent_files, context breakdown, subagent
hierarchy) are deferred deliberately, not unresolved.
