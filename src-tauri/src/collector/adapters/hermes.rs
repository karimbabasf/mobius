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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::session::{Status, TitleSource, Tool};
    use rusqlite::Connection;
    use std::path::PathBuf;

    /// Create a throwaway Hermes-shaped DB at a unique temp path and run `body`
    /// to populate it. Returns the path.
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
        let s = sessions
            .iter()
            .find(|s| s.id == "s-1")
            .expect("cli session present");
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
