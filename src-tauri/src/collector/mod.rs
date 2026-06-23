pub mod adapters;
pub mod context;
pub mod liveness;
pub mod registry;
pub mod scanner;
pub mod session;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{Connection, OpenFlags};

use crate::collector::adapters::{claude, codex, hermes};
use crate::collector::liveness::{ClaudeStatus, LiveClaude};
use crate::collector::scanner::ProcessRoot;
use crate::collector::session::{
    AgentSession, ContextWindow, FileEvent, Status, TitleSource, Tokens, Tool,
};

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

/// Cached reconstruction of a Hermes session's activity (context window + files
/// touched). Tokenizing the transcript is the expensive part, so it is recomputed
/// only when the session's `message_count` changes — an idle-but-visible card
/// reuses the prior result on every poll.
struct HermesCacheEntry {
    message_count: u32,
    context: Option<ContextWindow>,
    recent_files: Vec<FileEvent>,
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
    /// Per-session Hermes activity cache, keyed by session id (see
    /// [`HermesCacheEntry`]).
    hermes_activity: Mutex<HashMap<String, HermesCacheEntry>>,
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
            hermes_activity: Mutex::new(HashMap::new()),
        }
    }

    /// Active sessions right now, newest activity first.
    ///
    /// Gathers the live-process signals and delegates to [`Collector::snapshot_with`].
    pub fn snapshot(&self, now_ms: i64) -> Vec<AgentSession> {
        let live_claude = liveness::live_claude_sessions();
        let live_codex = liveness::live_codex_pids_by_file();
        let hermes_pid = liveness::hermes_daemon_pid();
        let mut sessions = self.snapshot_with(now_ms, &live_claude, &live_codex, hermes_pid);
        // Overlay the OS process scan: enrich matching cards with their process
        // tree, and surface signature-matched processes that have no session
        // card at all as flagged "untracked" cards.
        correlate_scan(&mut sessions, &scanner::scan_processes(now_ms), now_ms);
        sessions.sort_by(newest_started_first);
        sessions
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
            let raw = hermes::snapshot_sessions(db);
            // Hermes runs one session at a time (the `--continue` model), and a
            // Hermes process is alive right now (we're in the `Some(pid)` arm).
            // The newest *still-open* session (no `end_reason` yet) is the one it
            // is driving — exempt it from the recency window so a single long
            // tool turn, during which neither messages nor token counters move,
            // doesn't make the active agent vanish and resurface as a bare,
            // telemetry-less process card.
            let active_id = raw
                .iter()
                .filter(|s| s.run.as_ref().map_or(true, |r| r.end_reason.is_none()))
                .max_by_key(|s| s.started_at)
                .map(|s| s.id.clone());
            for mut session in raw {
                if !seen.insert(session.id.clone()) {
                    continue;
                }
                let age = now_ms - session.last_event_at;
                let is_active = active_id.as_deref() == Some(session.id.as_str());
                if age >= self.active_window_ms && !is_active {
                    continue;
                }
                session.pid = Some(pid);
                // The active session is what the live daemon is on, so it is
                // working even between flushes; others fall back to recency.
                session.status = if is_active {
                    Status::Working
                } else {
                    live_status_from_age(age)
                };
                sessions.push(session);
            }

            // Reconstruct context-window occupancy and files-touched for the
            // Hermes cards we kept (cheap rows became visible; now do the heavy,
            // cached tokenization pass for just those).
            self.enrich_hermes(db, &mut sessions);
        }

        // Order by creation time (newest first) so a card holds a fixed slot.
        // Sorting by activity made cards jump as agents flipped working/idle.
        sessions.sort_by(newest_started_first);
        sessions
    }

    /// Fill in `context` and `recent_files` for the Hermes cards in `sessions`.
    ///
    /// Opens the state DB once (read-only) and reconstructs each visible Hermes
    /// session's activity, reusing the cache when the session's `message_count` is
    /// unchanged so an idle card costs nothing. Silently does nothing if there are
    /// no Hermes cards or the DB can't be opened.
    fn enrich_hermes(&self, db: &Path, sessions: &mut [AgentSession]) {
        if !sessions.iter().any(|s| matches!(s.tool, Tool::Hermes)) {
            return;
        }
        let Ok(conn) = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
            return;
        };
        let mut cache = self.hermes_activity.lock().expect("hermes activity cache");
        let live: HashSet<String> = sessions
            .iter()
            .filter(|s| matches!(s.tool, Tool::Hermes))
            .map(|s| s.id.clone())
            .collect();
        cache.retain(|id, _| live.contains(id));

        for session in sessions.iter_mut().filter(|s| matches!(s.tool, Tool::Hermes)) {
            // `run.messages` carries the session's message_count (set by the adapter).
            let message_count = session.run.as_ref().map(|r| r.messages).unwrap_or(0);
            let fresh = match cache.get(&session.id) {
                Some(entry) => entry.message_count != message_count,
                None => true,
            };
            if fresh {
                let activity =
                    hermes::reconstruct_activity(&conn, &session.id, session.model.as_deref());
                cache.insert(
                    session.id.clone(),
                    HermesCacheEntry {
                        message_count,
                        context: activity.context,
                        recent_files: activity.recent_files,
                    },
                );
            }
            if let Some(entry) = cache.get(&session.id) {
                session.context = entry.context.clone();
                session.recent_files = entry.recent_files.clone();
            }
        }
    }
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

/// Overlay the process scan onto the session list.
///
/// A scanned root whose PID matches a live session enriches that session with
/// its process tree (Hermes sessions share the daemon PID, so several cards may
/// receive the same tree — correct, they *are* that process). A root that
/// matches no session is a process running without a card of its own: it becomes
/// a synthesized, flagged "untracked" session.
fn correlate_scan(sessions: &mut Vec<AgentSession>, roots: &[ProcessRoot], now_ms: i64) {
    let tracked: HashSet<i32> = sessions.iter().filter_map(|s| s.pid).collect();
    for root in roots {
        if tracked.contains(&root.pid) {
            for session in sessions.iter_mut() {
                if session.pid == Some(root.pid) {
                    session.process_tree = Some(root.tree.clone());
                }
            }
        } else {
            sessions.push(synthesize_untracked(root, now_ms));
        }
    }
}

/// Build a flagged card for a signature-matched process that has no session
/// store of its own. The id embeds the start time so a reused PID can't collide
/// with an earlier process's card.
fn synthesize_untracked(root: &ProcessRoot, now_ms: i64) -> AgentSession {
    let command = root.tree.command.clone();
    let binary = command
        .split_whitespace()
        .next()
        .and_then(|exe| exe.rsplit('/').next())
        .unwrap_or("agent")
        .to_string();
    AgentSession {
        id: format!("proc:{}:{}", root.pid, root.started_at),
        tool: root.tool,
        pid: Some(root.pid),
        project_path: String::new(),
        branch: None,
        model: None,
        status: Status::Working,
        current_action: Some(command),
        started_at: root.started_at,
        last_event_at: now_ms,
        tokens: Tokens::default(),
        context: None,
        title: Some(binary),
        title_source: TitleSource::Fallback,
        can_rename: false,
        recent_files: Vec::new(),
        run: None,
        process_tree: Some(root.tree.clone()),
        untracked: true,
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
    use crate::collector::scanner::ProcessNode;

    fn root(pid: i32, tool: Tool, command: &str, started_at: i64) -> ProcessRoot {
        ProcessRoot {
            pid,
            ppid: 1,
            tool,
            started_at,
            tree: ProcessNode {
                pid,
                command: command.into(),
                children: vec![ProcessNode {
                    pid: pid + 1,
                    command: "cargo build".into(),
                    children: vec![],
                }],
            },
        }
    }

    #[test]
    fn correlate_scan_enriches_matching_session_with_tree() {
        let mut sessions = vec![session_with("s1", 0, 0)];
        sessions[0].pid = Some(4321);
        let roots = vec![root(4321, Tool::Hermes, "/x/bin/hermes -z go", 500)];
        correlate_scan(&mut sessions, &roots, 1_000);
        assert_eq!(sessions.len(), 1, "no untracked card for a matched pid");
        let tree = sessions[0].process_tree.as_ref().expect("tree attached");
        assert_eq!(tree.pid, 4321);
        assert_eq!(tree.children[0].command, "cargo build");
        assert!(!sessions[0].untracked);
    }

    #[test]
    fn correlate_scan_synthesizes_untracked_card_for_unmatched_root() {
        let mut sessions: Vec<AgentSession> = Vec::new();
        let roots = vec![root(59421, Tool::Hermes, "/x/bin/hermes -z build", 200)];
        correlate_scan(&mut sessions, &roots, 9_000);
        assert_eq!(sessions.len(), 1);
        let s = &sessions[0];
        assert_eq!(s.id, "proc:59421:200");
        assert!(s.untracked);
        assert_eq!(s.pid, Some(59421));
        assert!(matches!(s.tool, Tool::Hermes));
        assert_eq!(s.title.as_deref(), Some("hermes"));
        assert_eq!(s.current_action.as_deref(), Some("/x/bin/hermes -z build"));
        assert_eq!(s.started_at, 200);
        assert!(s.process_tree.is_some());
    }

    #[test]
    fn correlate_scan_does_not_duplicate_when_one_of_many_pids_matches() {
        // Two sessions, only one shares the scanned pid: the other is untouched
        // and no untracked card appears.
        let mut a = session_with("a", 0, 0);
        a.pid = Some(100);
        let mut b = session_with("b", 0, 0);
        b.pid = Some(200);
        let mut sessions = vec![a, b];
        let roots = vec![root(100, Tool::Claude, "claude", 0)];
        correlate_scan(&mut sessions, &roots, 1_000);
        assert_eq!(sessions.len(), 2);
        assert!(sessions[0].process_tree.is_some());
        assert!(sessions[1].process_tree.is_none());
    }

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
                 model_config TEXT,
                 started_at REAL NOT NULL, ended_at REAL, end_reason TEXT,
                 input_tokens INTEGER DEFAULT 0, output_tokens INTEGER DEFAULT 0,
                 cache_read_tokens INTEGER DEFAULT 0, cache_write_tokens INTEGER DEFAULT 0,
                 message_count INTEGER DEFAULT 0, api_call_count INTEGER DEFAULT 0,
                 estimated_cost_usd REAL, actual_cost_usd REAL, cost_status TEXT,
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
    fn hermes_keeps_newest_open_session_alive_during_a_long_tool_turn() {
        // The active agent: its newest message is well past the window (a single
        // long tool turn flushes nothing), but the session is still open and a
        // Hermes process is alive — so it must stay visible and Working, not age
        // out into a bare process card.
        let db = hermes_db("active", 1_000.0, 1_000.0); // last_event_at = 1_000_000 ms
        let collector = Collector::with_hermes_db(db, DEFAULT_ACTIVE_WINDOW_MS);
        let now = 1_000_000 + 30 * 60 * 1000; // 30 min of silent tool work
        let sessions = collector.snapshot_with(now, &HashMap::new(), &HashMap::new(), Some(4321));
        let found = sessions
            .iter()
            .find(|s| s.id == "herm-1")
            .expect("newest open session stays while the daemon is alive");
        assert!(matches!(found.status, Status::Working));
    }

    #[test]
    fn snapshot_omits_finished_hermes_session_aged_out_of_window() {
        // A *finished* session (it has an end_reason) is no longer the active one,
        // so once it falls outside the window it ages out as before.
        let db = hermes_db("aged", 1_000.0, 1_000.0);
        Connection::open(&db)
            .unwrap()
            .execute(
                "UPDATE sessions SET end_reason = 'cli_close' WHERE id = 'herm-1'",
                [],
            )
            .unwrap();
        let collector = Collector::with_hermes_db(db, DEFAULT_ACTIVE_WINDOW_MS);
        let now = 1_000_000 + 20 * 60 * 1000; // 20 min later -> outside 10-min window
        let sessions = collector.snapshot_with(now, &HashMap::new(), &HashMap::new(), Some(4321));
        assert!(!sessions.iter().any(|s| s.id == "herm-1"));
    }
}
