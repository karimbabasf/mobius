# Hermes Adapter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a third local-agent adapter — Hermes (Nous Research's desktop agent) — so its local CLI coding sessions appear as live agent cards in MOBIUS alongside Claude and Codex.

**Architecture:** Hermes runs one long-lived daemon that records every session in a single SQLite DB at `~/.hermes/state.db` (WAL mode). Unlike the per-file Claude/Codex adapters, Hermes is read with one **read-only** SQL query that returns many sessions. The adapter does pure DB→`AgentSession` mapping; the collector applies daemon liveness, the recency window, and the working/idle split — exactly as it does for Codex. A single daemon PID (found via `pgrep`) gates all Hermes cards.

**Tech Stack:** Rust (Tauri v2 backend), `rusqlite` (bundled SQLite), TypeScript/Vite frontend, `cargo test` + `vitest`.

## Global Constraints

- **Read-only access to Hermes data.** Never write to `~/.hermes/state.db`. Consequence: Hermes sessions are `can_rename = false`.
- **Surface only `source='cli'` sessions with a non-empty `cwd`** (and `archived = 0`). Other Hermes surfaces (Telegram/Discord/etc.) are out of scope.
- **Timestamps in the DB are epoch *seconds* (float).** MOBIUS uses **milliseconds**. Convert every timestamp.
- **Degrade gracefully:** a missing/unreadable DB, absent `pgrep`, or non-Unix target must yield no Hermes cards, never an error or panic.
- **`rusqlite` pinned with the `bundled` feature** (compiles SQLite in; no system dependency).
- Follow existing adapter/collector patterns; do not restructure Claude/Codex code.

---

### Task 1: Add the `Hermes` tool variant (backend)

**Files:**
- Modify: `src-tauri/src/collector/session.rs:5-9` (the `Tool` enum)
- Test: `src-tauri/src/collector/session.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `Tool::Hermes`, serializing to the JSON string `"hermes"`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src-tauri/src/collector/session.rs`:

```rust
#[test]
fn hermes_tool_serializes_lowercase() {
    assert_eq!(serde_json::to_string(&Tool::Hermes).unwrap(), "\"hermes\"");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p mobius_lib hermes_tool_serializes_lowercase`
Expected: FAIL to compile — `no variant named Hermes found for enum Tool`.

- [ ] **Step 3: Add the variant**

In `src-tauri/src/collector/session.rs`, change the `Tool` enum:

```rust
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Claude,
    Codex,
    Cursor,
    Hermes,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test -p mobius_lib hermes_tool_serializes_lowercase`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/collector/session.rs
git commit -m "feat(collector): add Hermes tool variant"
```

---

### Task 2: Hermes adapter — read sessions from SQLite

**Files:**
- Create: `src-tauri/src/collector/adapters/hermes.rs`
- Modify: `src-tauri/src/collector/adapters/mod.rs:1-2`
- Modify: `src-tauri/Cargo.toml:20-28` (add `rusqlite`)
- Test: `src-tauri/src/collector/adapters/hermes.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces:
  - `pub fn snapshot_sessions(db_path: &Path) -> Vec<AgentSession>` — all `source='cli'` sessions with a `cwd`, status defaulted to `Idle`, `pid = None`, no time filtering. Empty vec if the DB is missing/unreadable.
  - `fn build_session(...) -> AgentSession` — pure row→session mapping (private; tested directly).
- Consumes: `AgentSession`, `Status`, `TitleSource`, `Tokens`, `Tool` from `collector::session`.

- [ ] **Step 1: Add the `rusqlite` dependency**

In `src-tauri/Cargo.toml`, under `[dependencies]` (after the `tiktoken-rs` line), add:

```toml
# Read-only access to Hermes's session store (~/.hermes/state.db). `bundled`
# compiles SQLite in, so there is no system-library dependency.
rusqlite = { version = "0.32", features = ["bundled"] }
```

- [ ] **Step 2: Register the module**

Change `src-tauri/src/collector/adapters/mod.rs` to:

```rust
pub mod claude;
pub mod codex;
pub mod hermes;
```

- [ ] **Step 3: Write the failing test (create the file with tests first)**

Create `src-tauri/src/collector/adapters/hermes.rs` containing **only** the test module and a stub, so the test compiles and fails meaningfully:

```rust
//! Hermes adapter (Nous Research desktop agent) — see this module's tests for
//! the contract. Stub during TDD; real implementation added in the next step.

use std::path::Path;

use crate::collector::session::AgentSession;

pub fn snapshot_sessions(_db_path: &Path) -> Vec<AgentSession> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::session::{Status, TitleSource, Tool};
    use rusqlite::Connection;
    use std::path::PathBuf;

    /// Create a throwaway Hermes-shaped DB at a unique temp path and run `body`
    /// to populate it. Returns the path; caller is responsible for nothing —
    /// temp files are fine to leave for the test run.
    fn make_db(tag: &str, body: impl FnOnce(&Connection)) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mobius-hermes-{}-{}.db",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_file(&path);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY, source TEXT NOT NULL, model TEXT,
                 started_at REAL NOT NULL, ended_at REAL,
                 input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0,
                 cache_read_tokens INTEGER DEFAULT 0, cache_write_tokens INTEGER DEFAULT 0,
                 cwd TEXT, title TEXT, archived INTEGER NOT NULL DEFAULT 0
             );
             CREATE TABLE messages (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT NOT NULL, timestamp REAL NOT NULL
             );",
        )
        .unwrap();
        body(&conn);
        path
    }

    fn insert_cli_session(conn: &Connection, id: &str, started: f64, cwd: Option<&str>) {
        conn.execute(
            "INSERT INTO sessions
                (id, source, model, started_at, ended_at,
                 input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                 cwd, title, archived)
             VALUES (?1,'cli','fugu-ultra',?2,NULL,100,20,5,3,?3,'Demo Session',0)",
            rusqlite::params![id, started, cwd],
        )
        .unwrap();
    }

    #[test]
    fn returns_cli_session_with_mapped_fields() {
        let db = make_db("basic", |conn| {
            insert_cli_session(conn, "s-1", 1000.0, Some("/work/proj"));
            conn.execute(
                "INSERT INTO messages (session_id, timestamp) VALUES ('s-1', 1500.0)",
                [],
            )
            .unwrap();
        });
        let sessions = snapshot_sessions(&db);
        let s = sessions.iter().find(|s| s.id == "s-1").expect("cli session present");
        assert!(matches!(s.tool, Tool::Hermes));
        assert_eq!(s.project_path, "/work/proj");
        assert_eq!(s.model.as_deref(), Some("fugu-ultra"));
        // seconds -> milliseconds
        assert_eq!(s.started_at, 1_000_000);
        // last_event_at from MAX(messages.timestamp)
        assert_eq!(s.last_event_at, 1_500_000);
        // tokens: input/output direct, cache = read + write
        assert_eq!(s.tokens.input, 100);
        assert_eq!(s.tokens.output, 20);
        assert_eq!(s.tokens.cache, 8);
        assert_eq!(s.title.as_deref(), Some("Demo Session"));
        assert!(matches!(s.title_source, TitleSource::Provider));
        assert!(!s.can_rename);
        assert!(matches!(s.status, Status::Idle)); // collector overrides later
        assert!(s.pid.is_none());
    }

    #[test]
    fn excludes_non_cli_sources() {
        let db = make_db("source", |conn| {
            conn.execute(
                "INSERT INTO sessions
                    (id, source, started_at, cwd, archived)
                 VALUES ('tg-1','telegram',1000.0,'/work/proj',0)",
                [],
            )
            .unwrap();
        });
        assert!(snapshot_sessions(&db).is_empty());
    }

    #[test]
    fn excludes_sessions_without_cwd() {
        let db = make_db("nocwd", |conn| {
            insert_cli_session(conn, "s-null", 1000.0, None);
            insert_cli_session(conn, "s-empty", 1000.0, Some(""));
        });
        assert!(snapshot_sessions(&db).is_empty());
    }

    #[test]
    fn excludes_archived_sessions() {
        let db = make_db("arch", |conn| {
            conn.execute(
                "INSERT INTO sessions
                    (id, source, started_at, cwd, archived)
                 VALUES ('a-1','cli',1000.0,'/work/proj',1)",
                [],
            )
            .unwrap();
        });
        assert!(snapshot_sessions(&db).is_empty());
    }

    #[test]
    fn last_event_falls_back_to_started_at_without_messages() {
        let db = make_db("fallback", |conn| {
            insert_cli_session(conn, "s-2", 2000.0, Some("/work/proj"));
        });
        let sessions = snapshot_sessions(&db);
        let s = sessions.iter().find(|s| s.id == "s-2").unwrap();
        assert_eq!(s.last_event_at, 2_000_000);
    }

    #[test]
    fn missing_db_returns_empty() {
        let path = std::env::temp_dir().join("mobius-hermes-does-not-exist.db");
        let _ = std::fs::remove_file(&path);
        assert!(snapshot_sessions(&path).is_empty());
    }
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p mobius_lib collector::adapters::hermes`
Expected: tests compile but FAIL (stub returns empty; `returns_cli_session_with_mapped_fields` panics on `expect("cli session present")`).

- [ ] **Step 5: Implement the adapter**

Replace the entire contents of `src-tauri/src/collector/adapters/hermes.rs` above the `#[cfg(test)]` line with:

```rust
//! Hermes adapter (Nous Research desktop agent).
//!
//! Unlike Claude/Codex (one process + one JSONL file per session), Hermes runs
//! a single long-lived daemon that records every session in one SQLite database
//! at `~/.hermes/state.db` (WAL mode). This adapter reads that DB **read-only**
//! and returns the local coding sessions (`source='cli'` with a working
//! directory). DB timestamps are epoch *seconds*; we convert to milliseconds.
//!
//! Status is left at `Idle` and `pid` unset here: the collector applies daemon
//! liveness, the recency window, and the working/idle split, exactly as it does
//! for Codex.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

use crate::collector::session::{AgentSession, Status, TitleSource, Tokens, Tool};

/// Read all local Hermes coding sessions from the state DB (read-only).
/// Returns an empty vec if the DB is missing or unreadable.
pub fn snapshot_sessions(db_path: &Path) -> Vec<AgentSession> {
    let conn = match Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(conn) => conn,
        Err(_) => return Vec::new(),
    };

    let mut stmt = match conn.prepare(
        "SELECT s.id, s.title, s.model, s.cwd, s.started_at, s.ended_at, \
                s.input_tokens, s.output_tokens, \
                s.cache_read_tokens, s.cache_write_tokens, \
                (SELECT MAX(m.timestamp) FROM messages m WHERE m.session_id = s.id) \
         FROM sessions s \
         WHERE s.source = 'cli' AND s.cwd IS NOT NULL AND s.cwd <> '' AND s.archived = 0 \
         ORDER BY s.started_at DESC",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };

    let rows = stmt.query_map([], |row| {
        Ok(build_session(
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, f64>(4)?,
            row.get::<_, Option<f64>>(5)?,
            row.get::<_, Option<i64>>(6)?.unwrap_or(0),
            row.get::<_, Option<i64>>(7)?.unwrap_or(0),
            row.get::<_, Option<i64>>(8)?.unwrap_or(0),
            row.get::<_, Option<i64>>(9)?.unwrap_or(0),
            row.get::<_, Option<f64>>(10)?,
        ))
    });

    match rows {
        Ok(iter) => iter.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

fn secs_to_ms(secs: f64) -> i64 {
    (secs * 1000.0) as i64
}

/// Pure mapping of one `sessions` row to an `AgentSession`. `last_event_at`
/// prefers the newest message timestamp, then `ended_at`, then `started_at`.
#[allow(clippy::too_many_arguments)]
fn build_session(
    id: String,
    title: Option<String>,
    model: Option<String>,
    cwd: String,
    started_at: f64,
    ended_at: Option<f64>,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    last_msg: Option<f64>,
) -> AgentSession {
    let last_secs = last_msg.or(ended_at).unwrap_or(started_at);
    let (title, title_source) = match title {
        Some(t) if !t.trim().is_empty() => (Some(t), TitleSource::Provider),
        _ => (None, TitleSource::Fallback),
    };
    AgentSession {
        id,
        tool: Tool::Hermes,
        pid: None,
        project_path: cwd,
        branch: None,
        model,
        status: Status::Idle,
        current_action: None,
        started_at: secs_to_ms(started_at),
        last_event_at: secs_to_ms(last_secs),
        tokens: Tokens {
            input: input.max(0) as u64,
            output: output.max(0) as u64,
            cache: (cache_read.max(0) + cache_write.max(0)) as u64,
        },
        context: None,
        title,
        title_source,
        can_rename: false,
        recent_files: Vec::new(),
    }
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p mobius_lib collector::adapters::hermes`
Expected: all six tests PASS. (First run compiles bundled SQLite — may take a minute.)

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/collector/adapters/mod.rs src-tauri/src/collector/adapters/hermes.rs
git commit -m "feat(collector): read Hermes sessions from state.db (read-only)"
```

---

### Task 3: Hermes daemon liveness

**Files:**
- Modify: `src-tauri/src/collector/liveness.rs` (add function + test)

**Interfaces:**
- Produces:
  - `pub fn hermes_daemon_pid() -> Option<i32>` — PID of the running Hermes daemon, or `None`.
  - `fn first_pid(output: &str) -> Option<i32>` — first PID line from `pgrep` output (private; tested).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `src-tauri/src/collector/liveness.rs`:

```rust
#[test]
fn first_pid_picks_first_line_and_ignores_blanks() {
    assert_eq!(first_pid("83766\n83770\n"), Some(83766));
    assert_eq!(first_pid("\n  91234 \n"), Some(91234));
    assert_eq!(first_pid(""), None);
    assert_eq!(first_pid("not-a-pid\n"), None);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p mobius_lib first_pid_picks_first_line_and_ignores_blanks`
Expected: FAIL to compile — `cannot find function first_pid`.

- [ ] **Step 3: Implement**

Add to `src-tauri/src/collector/liveness.rs` (after `live_codex_pids_by_file`, before the registry helpers):

```rust
/// PID of the running Hermes daemon, if any.
///
/// Hermes runs a single background process whose argv contains
/// `.hermes/<...>/bin/hermes`. There is no per-session process (every session
/// lives in one SQLite DB), so this single PID gates all Hermes cards. Uses
/// `pgrep -f`; yields `None` where `pgrep` is missing, nothing matches, or on
/// non-Unix targets.
pub fn hermes_daemon_pid() -> Option<i32> {
    let output = Command::new("pgrep")
        .args(["-f", r"\.hermes/.*/bin/hermes"])
        .output()
        .ok()?;
    if !output.status.success() {
        // pgrep exits non-zero when nothing matched.
        return None;
    }
    first_pid(&String::from_utf8_lossy(&output.stdout))
}

/// First PID from `pgrep` output (one PID per line).
fn first_pid(output: &str) -> Option<i32> {
    output
        .lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
        .next()
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test -p mobius_lib first_pid_picks_first_line_and_ignores_blanks`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/collector/liveness.rs
git commit -m "feat(collector): detect the Hermes daemon via pgrep"
```

---

### Task 4: Wire Hermes into the Collector

**Files:**
- Modify: `src-tauri/src/collector/mod.rs` (imports, `Source` is unchanged; `Collector` field, constructors, `snapshot`, `snapshot_with`, tests)

**Interfaces:**
- Consumes: `hermes::snapshot_sessions` (Task 2), `liveness::hermes_daemon_pid` (Task 3).
- Produces: `Collector::with_hermes_db(db: PathBuf, active_window_ms: i64) -> Self` (test helper); `snapshot_with` gains a trailing `hermes_pid: Option<i32>` parameter.

- [ ] **Step 1: Write the failing integration tests**

Add to the `tests` module in `src-tauri/src/collector/mod.rs`. (These use `rusqlite` directly to build a temp Hermes DB.)

```rust
fn hermes_db(tag: &str, started: f64, last_msg: f64) -> PathBuf {
    use rusqlite::Connection;
    let path = std::env::temp_dir().join(format!(
        "mobius-collector-hermes-{}-{}.db",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_file(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE sessions (
             id TEXT PRIMARY KEY, source TEXT NOT NULL, model TEXT,
             started_at REAL NOT NULL, ended_at REAL,
             input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0,
             cache_read_tokens INTEGER DEFAULT 0, cache_write_tokens INTEGER DEFAULT 0,
             cwd TEXT, title TEXT, archived INTEGER NOT NULL DEFAULT 0
         );
         CREATE TABLE messages (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             session_id TEXT NOT NULL, timestamp REAL NOT NULL
         );",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO sessions (id, source, model, started_at, cwd, title, archived)
         VALUES ('herm-1','cli','fugu-ultra',?1,'/work/proj','Demo',0)",
        rusqlite::params![started],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO messages (session_id, timestamp) VALUES ('herm-1', ?1)",
        rusqlite::params![last_msg],
    )
    .unwrap();
    path
}

#[test]
fn snapshot_includes_live_hermes_session_as_working() {
    let db = hermes_db("live", 1_000.0, 1_000.0); // last_event_at = 1_000_000 ms
    let collector = Collector::with_hermes_db(db, DEFAULT_ACTIVE_WINDOW_MS);
    let now = 1_000_000 + 30_000; // 30s after last activity -> Working
    let sessions = collector.snapshot_with(now, &HashMap::new(), &HashMap::new(), Some(4321));
    let found = sessions
        .iter()
        .find(|s| s.id == "herm-1")
        .expect("live hermes session should be present");
    assert!(matches!(found.tool, Tool::Hermes));
    assert!(matches!(found.status, Status::Working));
    assert_eq!(found.pid, Some(4321));
    assert_eq!(found.title.as_deref(), Some("Demo"));
}

#[test]
fn snapshot_omits_hermes_when_daemon_not_running() {
    let db = hermes_db("nodaemon", 1_000.0, 1_000.0);
    let collector = Collector::with_hermes_db(db, DEFAULT_ACTIVE_WINDOW_MS);
    let now = 1_000_000 + 30_000;
    let sessions = collector.snapshot_with(now, &HashMap::new(), &HashMap::new(), None);
    assert!(!sessions.iter().any(|s| s.id == "herm-1"));
}

#[test]
fn snapshot_omits_hermes_session_aged_out_of_window() {
    let db = hermes_db("aged", 1_000.0, 1_000.0);
    let collector = Collector::with_hermes_db(db, DEFAULT_ACTIVE_WINDOW_MS);
    let now = 1_000_000 + 20 * 60 * 1000; // 20 min later -> outside 10-min window
    let sessions = collector.snapshot_with(now, &HashMap::new(), &HashMap::new(), Some(4321));
    assert!(!sessions.iter().any(|s| s.id == "herm-1"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p mobius_lib collector::tests::snapshot_includes_live_hermes_session_as_working`
Expected: FAIL to compile — `no function with_hermes_db`, and `snapshot_with` takes 3 args not 4.

- [ ] **Step 3: Import the adapter and liveness function**

In `src-tauri/src/collector/mod.rs`, change the import (line 11):

```rust
use crate::collector::adapters::{claude, codex, hermes};
```

- [ ] **Step 4: Add the `hermes_db` field and update constructors**

In the `Collector` struct, replace the `active_window_ms` field's comment/attribute (lines 41-43) and add `hermes_db`:

```rust
    codex_indexes: Vec<PathBuf>,
    /// Hermes session DB (`~/.hermes/state.db`), if Hermes is installed.
    hermes_db: Option<PathBuf>,
    /// Recency window: a Hermes session quieter than this ages out of the
    /// dashboard. (Claude/Codex are gated by process liveness instead.)
    active_window_ms: i64,
    cache: Mutex<HashMap<PathBuf, CacheEntry>>,
```

Update `new()` to pass the Hermes DB path:

```rust
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        Self::build(
            home.join(".claude").join("projects"),
            vec![
                home.join(".codex").join("sessions"),
                home.join(".codex-karim").join("sessions"),
            ],
            vec![
                home.join(".codex").join("session_index.jsonl"),
                home.join(".codex-karim").join("session_index.jsonl"),
            ],
            Some(home.join(".hermes").join("state.db")),
            DEFAULT_ACTIVE_WINDOW_MS,
        )
    }
```

Update the two existing test helpers to pass `None`:

```rust
    /// Claude-only collector (used by tests that should not see local Codex logs).
    pub fn with_claude_dir(claude_dir: PathBuf, active_window_ms: i64) -> Self {
        Self::build(claude_dir, Vec::new(), Vec::new(), None, active_window_ms)
    }

    /// Single-Codex-home collector (test helper).
    pub fn with_dirs(
        claude_dir: PathBuf,
        codex_dir: PathBuf,
        codex_index: PathBuf,
        active_window_ms: i64,
    ) -> Self {
        Self::build(
            claude_dir,
            vec![codex_dir],
            vec![codex_index],
            None,
            active_window_ms,
        )
    }

    /// Hermes-only collector (test helper).
    pub fn with_hermes_db(hermes_db: PathBuf, active_window_ms: i64) -> Self {
        Self::build(
            PathBuf::from("/nonexistent-claude"),
            Vec::new(),
            Vec::new(),
            Some(hermes_db),
            active_window_ms,
        )
    }
```

Update `build` to take and store `hermes_db`:

```rust
    fn build(
        claude_dir: PathBuf,
        codex_dirs: Vec<PathBuf>,
        codex_indexes: Vec<PathBuf>,
        hermes_db: Option<PathBuf>,
        active_window_ms: i64,
    ) -> Self {
        Self {
            claude_dir,
            codex_dirs,
            codex_indexes,
            hermes_db,
            active_window_ms,
            cache: Mutex::new(HashMap::new()),
        }
    }
```

- [ ] **Step 5: Pass the daemon PID through `snapshot`**

Update `snapshot` (around line 102):

```rust
    pub fn snapshot(&self, now_ms: i64) -> Vec<AgentSession> {
        let live_claude = liveness::live_claude_sessions();
        let live_codex = liveness::live_codex_pids_by_file();
        let hermes_pid = liveness::hermes_daemon_pid();
        self.snapshot_with(now_ms, &live_claude, &live_codex, hermes_pid)
    }
```

- [ ] **Step 6: Add the `hermes_pid` parameter and the Hermes branch**

Change the `snapshot_with` signature:

```rust
    fn snapshot_with(
        &self,
        now_ms: i64,
        live_claude: &HashMap<String, LiveClaude>,
        live_codex: &HashMap<PathBuf, i32>,
        hermes_pid: Option<i32>,
    ) -> Vec<AgentSession> {
```

Then, immediately **before** the `sessions.sort_by(newest_started_first);` line at the end of `snapshot_with`, insert:

```rust
        // Hermes: one daemon, many sessions in one SQLite DB. Gate on the daemon
        // being alive (like a process check), then apply the recency window so
        // old sessions age out even though the daemon lives on. Status is set
        // here, consistent with the Codex branch above.
        if let (Some(db), Some(pid)) = (self.hermes_db.as_ref(), hermes_pid) {
            for mut session in hermes::snapshot_sessions(db) {
                if !seen.insert(session.id.clone()) {
                    continue;
                }
                let age = now_ms - session.last_event_at;
                if age >= self.active_window_ms {
                    continue;
                }
                session.pid = Some(pid);
                session.status = live_status_from_age(age);
                sessions.push(session);
            }
        }

```

- [ ] **Step 7: Fix the remaining existing `snapshot_with` call sites**

The following existing tests call `snapshot_with` with 3 args — add a trailing `None` to each:
- `snapshot_shows_live_session_as_working_and_sets_pid`
- `snapshot_omits_session_whose_process_is_dead`
- `snapshot_keeps_live_but_idle_session_past_old_window`
- `snapshot_includes_codex_session_with_thread_name_title`
- `snapshot_deduplicates_codex_sessions_across_homes`

For each, change the call, e.g.:

```rust
        let sessions = collector.snapshot_with(now, &alive_claude("sess-basic"), &HashMap::new(), None);
```

and the dedup test's `.snapshot_with(now, &HashMap::new(), &codex_alive(rollout), None)`.

Also update the `Collector::build(...)` call inside `snapshot_deduplicates_codex_sessions_across_homes` to insert `None` before `DEFAULT_ACTIVE_WINDOW_MS`:

```rust
        let collector = Collector::build(
            PathBuf::from("/nonexistent-claude"),
            vec![codex_dir.clone(), codex_dir.clone()],
            vec![codex_dir.join("session_index.jsonl")],
            None,
            DEFAULT_ACTIVE_WINDOW_MS,
        );
```

Ensure the test module imports `Status` (it already imports `Tool` via `use crate::collector::session::Tool;` — add `Status`):

```rust
    use crate::collector::session::{Status, Tool};
```

- [ ] **Step 8: Run the full backend test suite**

Run: `cd src-tauri && cargo test -p mobius_lib`
Expected: PASS — all prior tests plus the three new Hermes collector tests.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/collector/mod.rs
git commit -m "feat(collector): surface live Hermes sessions in snapshots"
```

---

### Task 5: Frontend — render Hermes as a third provider

**Files:**
- Modify: `src/types.ts:1` (the `Tool` union)
- Modify: `src/components/agentCard.ts:7-17` (labels + badges)
- Modify: `src/components/toolLogo.ts:5-12` (mark)
- Modify: `src/styles.css` (color var + `data-tool` accent)
- Test: existing `vitest` suites (TypeScript's `Record<Tool, ...>` enforces completeness at build)

**Interfaces:**
- Consumes: backend `tool: "hermes"` value from `AgentSession` JSON.

- [ ] **Step 1: Extend the `Tool` union**

In `src/types.ts`, change line 1:

```ts
export type Tool = "claude" | "codex" | "cursor" | "hermes";
```

- [ ] **Step 2: Verify the type error surfaces (the failing check)**

Run: `npx tsc --noEmit`
Expected: FAIL — `Property 'hermes' is missing` in the `Record<Tool, string>` maps in `agentCard.ts` and the `Record<Tool, ...>` in `toolLogo.ts`. This is the type system acting as the test.

- [ ] **Step 3: Add labels and badge**

In `src/components/agentCard.ts`:

```ts
const toolLabels: Record<Tool, string> = {
  claude: "Claude",
  codex: "Codex",
  cursor: "Cursor",
  hermes: "Hermes",
};

const providerBadges: Record<Tool, string> = {
  claude: "CLAUDE",
  codex: "CODEX",
  cursor: "CURSOR",
  hermes: "HERMES",
};
```

- [ ] **Step 4: Add the Hermes logo mark**

In `src/components/toolLogo.ts`, add a `hermes` entry to `marks` (an original geometric mark — an upward triangle with a stem, tinted by CSS):

```ts
  hermes:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round"><path d="M8 2l5.5 9.5h-11z"/><path d="M8 6.5v4"/></svg>',
```

- [ ] **Step 5: Add the color var and `data-tool` accent**

In `src/styles.css`, add the color var after `--cursor: #66d68f;` (line 19):

```css
  --hermes: #c0a7ff;
```

And add a `data-tool` rule after the `cursor` block (after line 295):

```css
.agent-block[data-tool="hermes"] {
  --accent: var(--hermes);
}
```

- [ ] **Step 6: Verify types and run the frontend test suite**

Run: `npx tsc --noEmit && npm test`
Expected: PASS — type check clean, all `vitest` suites green.

- [ ] **Step 7: Commit**

```bash
git add src/types.ts src/components/agentCard.ts src/components/toolLogo.ts src/styles.css
git commit -m "feat(ui): render Hermes as a third agent provider"
```

---

### Task 6: End-to-end verification against the live daemon

**Files:** none (verification only)

- [ ] **Step 1: Confirm the full backend suite passes**

Run: `cd src-tauri && cargo test -p mobius_lib`
Expected: all PASS.

- [ ] **Step 2: Observe a real Hermes session via the snapshot example**

With Hermes running and a recent CLI session active, run:

```bash
cd src-tauri && . ~/.cargo/env && cargo run --example snapshot
```

Expected: at least one session with `"tool":"hermes"`, a real `projectPath`, non-null `model`, and `status` `working`/`idle`. If no Hermes card appears, send a message in a Hermes CLI session (to refresh `last_event_at` inside the 10-minute window) and re-run.

- [ ] **Step 3: Note any WAL read-only caveat**

If `cargo run --example snapshot` shows no Hermes session while one is clearly active, verify the DB opens read-only:

```bash
sqlite3 -readonly ~/.hermes/state.db "SELECT id, source, cwd FROM sessions WHERE ended_at IS NULL;"
```

If that returns rows but the example does not, the read-only WAL open may be the cause — switch the adapter's open to immutable URI mode (`Connection::open_with_flags("file:<path>?immutable=1", SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_URI)`) and re-run. (Document the change if made.)

- [ ] **Step 4: Final commit (if Step 3 required a change)**

```bash
git add src-tauri/src/collector/adapters/hermes.rs
git commit -m "fix(collector): open Hermes DB in immutable mode for WAL reads"
```

---

## Self-Review

- **Spec coverage:** DB-backed read-only adapter (Task 2) ✓; `source='cli'`+`cwd` filter (Task 2) ✓; `rusqlite` bundled (Task 2) ✓; seconds→ms (Task 2) ✓; token mapping incl. cache=read+write (Task 2) ✓; daemon liveness gate (Task 3) ✓; recency window + working/idle (Task 4) ✓; `Tool::Hermes` + frontend rendering (Tasks 1, 5) ✓; `can_rename=false` / no rename wiring (Task 2 mapping; `rename_session` untouched) ✓; fixture-driven tests (Tasks 2, 4) ✓. Out-of-scope items (recent_files, context breakdown, non-CLI sources, subagent hierarchy) intentionally absent.
- **Placeholder scan:** none — every code step contains full code.
- **Type consistency:** `snapshot_sessions(&Path) -> Vec<AgentSession>` and `hermes_daemon_pid() -> Option<i32>` are defined in Tasks 2/3 and consumed with matching signatures in Task 4; `snapshot_with`'s new trailing `Option<i32>` is applied at every call site in Step 7.
