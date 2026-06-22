pub mod adapters;
pub mod context;
pub mod liveness;
pub mod registry;
pub mod session;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::collector::adapters::{claude, codex, hermes};
use crate::collector::liveness::{ClaudeStatus, LiveClaude};
use crate::collector::session::{AgentSession, Status, TitleSource};

const DEFAULT_ACTIVE_WINDOW_MS: i64 = 10 * 60 * 1000;
const WORKING_RECENCY_MS: i64 = 90 * 1000;

#[derive(Clone, Copy)]
enum Source {
    Claude,
    Codex,
}

struct CacheEntry {
    mtime: i64,
    session: AgentSession,
}

/// Live registry of agent sessions, built from on-disk agent logs.
///
/// `snapshot` re-scans the agent log directories on each call and returns the
/// sessions whose underlying process is still alive (see [`liveness`]). Parsed
/// sessions are cached per file and only re-parsed when the file's mtime
/// changes, so an idle dashboard does no work. Codex is scanned across multiple
/// homes (the default profile plus isolated profiles such as `~/.codex-karim`);
/// their session-name indexes are merged.
pub struct Collector {
    claude_dir: PathBuf,
    codex_dirs: Vec<PathBuf>,
    codex_indexes: Vec<PathBuf>,
    /// Hermes session DB (`~/.hermes/state.db`), if Hermes is installed.
    hermes_db: Option<PathBuf>,
    /// Recency window: a Hermes session quieter than this ages out of the
    /// dashboard. (Claude/Codex are gated by process liveness instead.)
    active_window_ms: i64,
    cache: Mutex<HashMap<PathBuf, CacheEntry>>,
}

impl Collector {
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

    /// Active sessions right now, newest activity first.
    ///
    /// Gathers the live-process signals and delegates to [`Collector::snapshot_with`].
    pub fn snapshot(&self, now_ms: i64) -> Vec<AgentSession> {
        let live_claude = liveness::live_claude_sessions();
        let live_codex = liveness::live_codex_pids_by_file();
        let hermes_pid = liveness::hermes_daemon_pid();
        self.snapshot_with(now_ms, &live_claude, &live_codex, hermes_pid)
    }

    pub fn rename_session(&self, session_id: &str, new_title: &str) -> Result<(), String> {
        if new_title.trim().is_empty() {
            return Err("Agent name cannot be empty.".into());
        }

        let mut claude_files = Vec::new();
        collect_jsonl(&self.claude_dir, &mut claude_files);
        for (path, _) in claude_files {
            if path.file_stem().and_then(|s| s.to_str()) == Some(session_id) {
                return claude::rename_ai_title(&path, new_title);
            }
        }

        for index in &self.codex_indexes {
            if codex::load_thread_names(index).contains_key(session_id) {
                return codex::rename_thread_name(index, session_id, new_title);
            }
        }

        Err("No writable provider name found for this session.".into())
    }

    /// Build the snapshot from explicit liveness maps (the injection point that
    /// keeps the gating logic testable without real OS processes).
    ///
    /// A session is included only if its process is alive:
    /// * Claude — its `sessionId` (the transcript filename stem) is present in
    ///   `live_claude`; status comes from Claude's own registry, falling back to
    ///   recency when the registry omits it.
    /// * Codex — its transcript path is held open by a live `codex` process.
    ///
    /// Anything else is omitted entirely, so a terminated agent disappears on
    /// the next poll rather than lingering as a stale "idle" card.
    fn snapshot_with(
        &self,
        now_ms: i64,
        live_claude: &HashMap<String, LiveClaude>,
        live_codex: &HashMap<PathBuf, i32>,
        hermes_pid: Option<i32>,
    ) -> Vec<AgentSession> {
        // Codex's open files come from lsof as resolved absolute paths; resolve
        // the candidate paths the same way so the comparison is symlink-stable.
        let codex_live: HashMap<PathBuf, i32> = live_codex
            .iter()
            .filter_map(|(path, pid)| std::fs::canonicalize(path).ok().map(|c| (c, *pid)))
            .collect();

        let mut files: Vec<(PathBuf, i64, Source)> = Vec::new();
        let mut claude_files = Vec::new();
        collect_jsonl(&self.claude_dir, &mut claude_files);
        for (path, mtime) in claude_files {
            // Claude transcripts are named `<sessionId>.jsonl`; only keep files
            // whose session is reported alive.
            let alive = path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(|stem| live_claude.contains_key(stem))
                .unwrap_or(false);
            if alive {
                files.push((path, mtime, Source::Claude));
            }
        }
        for dir in &self.codex_dirs {
            let mut codex_files = Vec::new();
            collect_jsonl(dir, &mut codex_files);
            for (path, mtime) in codex_files {
                let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if codex_live.contains_key(&key) {
                    files.push((path, mtime, Source::Codex));
                }
            }
        }

        // Codex stores session names out-of-band; merge every home's index if any
        // Codex file is active (session ids are globally unique, so merging is safe).
        let thread_names = if files
            .iter()
            .any(|(_, _, source)| matches!(source, Source::Codex))
        {
            let mut names = HashMap::new();
            for index in &self.codex_indexes {
                for (id, name) in codex::load_thread_names(index) {
                    names.insert(id, name);
                }
            }
            names
        } else {
            HashMap::new()
        };

        let mut cache = self.cache.lock().expect("collector cache lock");
        let active: HashSet<PathBuf> = files.iter().map(|(path, _, _)| path.clone()).collect();
        cache.retain(|path, _| active.contains(path));

        let mut sessions = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for (path, mtime, source) in files {
            let needs_parse = match cache.get(&path) {
                Some(entry) => entry.mtime != mtime,
                None => true,
            };
            if needs_parse {
                let parsed = match source {
                    Source::Claude => claude::parse_session(&path),
                    Source::Codex => codex::parse_session(&path),
                };
                match parsed {
                    Some(session) => {
                        cache.insert(path.clone(), CacheEntry { mtime, session });
                    }
                    None => continue,
                }
            }
            let Some(entry) = cache.get(&path) else {
                continue;
            };
            let mut session = entry.session.clone();
            if !seen.insert(session.id.clone()) {
                continue;
            }

            let age = now_ms - session.last_event_at;
            match source {
                Source::Claude => {
                    let Some(live) = live_claude.get(&session.id) else {
                        // Stem matched but parsed id differs (shouldn't happen) —
                        // be conservative and skip rather than show a ghost.
                        continue;
                    };
                    session.pid = Some(live.pid);
                    session.status = match live.status {
                        Some(ClaudeStatus::Busy) => Status::Working,
                        Some(ClaudeStatus::Idle) => Status::Idle,
                        None => live_status_from_age(age),
                    };
                }
                Source::Codex => {
                    let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                    session.pid = codex_live.get(&key).copied();
                    session.status = live_status_from_age(age);
                    if let Some(name) = thread_names.get(&session.id) {
                        session.title = Some(name.clone());
                        session.title_source = TitleSource::Provider;
                        session.can_rename = true;
                    }
                }
            }

            sessions.push(session);
        }

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

        // Order by creation time (newest first) so a card holds a fixed slot.
        // Sorting by activity made cards jump as agents flipped working/idle.
        sessions.sort_by(newest_started_first);
        sessions
    }
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

/// Recency-based status for a session already confirmed alive: a recent write
/// means it's actively working, otherwise it's alive-but-idle. (Unlike the old
/// time-window gate, this never hides a session — liveness decides visibility.)
fn live_status_from_age(age_ms: i64) -> Status {
    if age_ms < WORKING_RECENCY_MS {
        Status::Working
    } else {
        Status::Idle
    }
}

/// Snapshot ordering: newest-created session first (by `started_at`), with the
/// id as a tie-break so equal timestamps never swap between snapshots. Ordering
/// is independent of activity, so a card keeps its slot as status changes.
fn newest_started_first(a: &AgentSession, b: &AgentSession) -> std::cmp::Ordering {
    b.started_at
        .cmp(&a.started_at)
        .then_with(|| a.id.cmp(&b.id))
}

/// Recursively collect every `*.jsonl` file under `dir` with its mtime.
fn collect_jsonl(dir: &Path, out: &mut Vec<(PathBuf, i64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            collect_jsonl(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            out.push((path, mtime));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::session::Tool;

    fn claude_fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/claude")
    }

    fn codex_fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/codex")
    }

    /// A live-Claude map asserting a single session is alive, with no explicit
    /// status (so the collector falls back to recency).
    fn alive_claude(id: &str) -> HashMap<String, LiveClaude> {
        let mut map = HashMap::new();
        map.insert(
            id.to_string(),
            LiveClaude {
                pid: 4242,
                status: None,
                entrypoint: Some("cli".into()),
            },
        );
        map
    }

    fn codex_alive(path: PathBuf) -> HashMap<PathBuf, i32> {
        let mut map = HashMap::new();
        map.insert(path, 9001);
        map
    }

    #[test]
    fn live_status_from_age_splits_working_and_idle() {
        assert!(matches!(live_status_from_age(1_000), Status::Working));
        assert!(matches!(live_status_from_age(5 * 60 * 1000), Status::Idle));
    }

    #[test]
    fn snapshot_shows_live_session_as_working_and_sets_pid() {
        let collector = Collector::with_claude_dir(claude_fixtures(), DEFAULT_ACTIVE_WINDOW_MS);
        let base = claude::parse_session(&claude_fixtures().join("sess-basic.jsonl")).unwrap();
        let now = base.last_event_at + 30_000;
        let sessions =
            collector.snapshot_with(now, &alive_claude("sess-basic"), &HashMap::new(), None);
        let found = sessions
            .iter()
            .find(|s| s.id == "sess-basic")
            .expect("live session should be present");
        assert!(matches!(found.status, Status::Working));
        assert_eq!(found.pid, Some(4242));
    }

    #[test]
    fn snapshot_omits_session_whose_process_is_dead() {
        let collector = Collector::with_claude_dir(claude_fixtures(), DEFAULT_ACTIVE_WINDOW_MS);
        let base = claude::parse_session(&claude_fixtures().join("sess-basic.jsonl")).unwrap();
        // Recent file, but no live process for it — must not appear (the headline bug).
        let now = base.last_event_at + 30_000;
        let sessions = collector.snapshot_with(now, &HashMap::new(), &HashMap::new(), None);
        assert!(!sessions.iter().any(|s| s.id == "sess-basic"));
    }

    #[test]
    fn snapshot_keeps_live_but_idle_session_past_old_window() {
        let collector = Collector::with_claude_dir(claude_fixtures(), DEFAULT_ACTIVE_WINDOW_MS);
        let base = claude::parse_session(&claude_fixtures().join("sess-basic.jsonl")).unwrap();
        // 20 minutes silent: the old time-window would have dropped it, but the
        // process is alive, so it should remain — shown as idle.
        let now = base.last_event_at + 20 * 60 * 1000;
        let sessions =
            collector.snapshot_with(now, &alive_claude("sess-basic"), &HashMap::new(), None);
        let found = sessions
            .iter()
            .find(|s| s.id == "sess-basic")
            .expect("alive idle session should remain");
        assert!(matches!(found.status, Status::Idle));
    }

    /// Clone a parsed session, overriding the fields the sort cares about.
    fn session_with(id: &str, started_at: i64, last_event_at: i64) -> AgentSession {
        let mut s = claude::parse_session(&claude_fixtures().join("sess-basic.jsonl")).unwrap();
        s.id = id.to_string();
        s.started_at = started_at;
        s.last_event_at = last_event_at;
        s
    }

    #[test]
    fn snapshot_orders_by_started_at_newest_first_not_by_activity() {
        // The newest-created session sorts first regardless of who acted last:
        // `old` has far more recent activity but an earlier start, yet must sort
        // below `new`. This is what keeps a card from jumping on status changes.
        let new = session_with("new", 2_000, 10);
        let old = session_with("old", 1_000, 9_999);
        let mut list = vec![old.clone(), new.clone()];
        list.sort_by(newest_started_first);
        assert_eq!(
            list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["new", "old"],
        );
    }

    #[test]
    fn snapshot_order_is_stable_on_started_at_ties() {
        // Equal start times break ties on id, so snapshots never swap two cards.
        let b = session_with("b", 1_000, 50);
        let a = session_with("a", 1_000, 10);
        let mut list = vec![b.clone(), a.clone()];
        list.sort_by(newest_started_first);
        assert_eq!(
            list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"],
        );
    }

    #[test]
    fn snapshot_includes_codex_session_with_thread_name_title() {
        let codex_dir = codex_fixtures();
        let collector = Collector::with_dirs(
            PathBuf::from("/nonexistent-claude"),
            codex_dir.clone(),
            codex_dir.join("session_index.jsonl"),
            DEFAULT_ACTIVE_WINDOW_MS,
        );
        let rollout = codex_dir.join("rollout-sample.jsonl");
        let base = codex::parse_session(&rollout).unwrap();
        let now = base.last_event_at + 30_000;
        let sessions = collector.snapshot_with(now, &HashMap::new(), &codex_alive(rollout), None);
        let found = sessions
            .iter()
            .find(|s| s.id == "codex-1")
            .expect("codex session should be present");
        assert!(matches!(found.tool, Tool::Codex));
        assert_eq!(found.title.as_deref(), Some("Build the thing"));
        assert_eq!(found.pid, Some(9001));
    }

    #[test]
    fn snapshot_deduplicates_codex_sessions_across_homes() {
        let codex_dir = codex_fixtures();
        // Two Codex homes pointing at the same logs must not double-count a session.
        let collector = Collector::build(
            PathBuf::from("/nonexistent-claude"),
            vec![codex_dir.clone(), codex_dir.clone()],
            vec![codex_dir.join("session_index.jsonl")],
            None,
            DEFAULT_ACTIVE_WINDOW_MS,
        );
        let rollout = codex_dir.join("rollout-sample.jsonl");
        let base = codex::parse_session(&rollout).unwrap();
        let now = base.last_event_at + 30_000;
        let count = collector
            .snapshot_with(now, &HashMap::new(), &codex_alive(rollout), None)
            .iter()
            .filter(|s| s.id == "codex-1")
            .count();
        assert_eq!(count, 1);
    }

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
                 cwd TEXT, title TEXT, tool_call_count INTEGER NOT NULL DEFAULT 0,
                 archived INTEGER NOT NULL DEFAULT 0
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
}
