pub mod adapters;
pub mod liveness;
pub mod registry;
pub mod session;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::collector::adapters::{claude, codex};
use crate::collector::liveness::{ClaudeStatus, LiveClaude};
use crate::collector::session::{AgentSession, Status, Tool};

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
    // Retained for API/test compatibility; liveness, not age, now gates display.
    #[allow(dead_code)]
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
            DEFAULT_ACTIVE_WINDOW_MS,
        )
    }

    /// Claude-only collector (used by tests that should not see local Codex logs).
    pub fn with_claude_dir(claude_dir: PathBuf, active_window_ms: i64) -> Self {
        Self::build(claude_dir, Vec::new(), Vec::new(), active_window_ms)
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
            active_window_ms,
        )
    }

    fn build(
        claude_dir: PathBuf,
        codex_dirs: Vec<PathBuf>,
        codex_indexes: Vec<PathBuf>,
        active_window_ms: i64,
    ) -> Self {
        Self {
            claude_dir,
            codex_dirs,
            codex_indexes,
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
        self.snapshot_with(now_ms, &live_claude, &live_codex)
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
        let thread_names = if files.iter().any(|(_, _, source)| matches!(source, Source::Codex)) {
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
                    }
                }
            }

            sessions.push(session);
        }

        sessions.sort_by(|a, b| b.last_event_at.cmp(&a.last_event_at));
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
        assert!(matches!(
            live_status_from_age(5 * 60 * 1000),
            Status::Idle
        ));
    }

    #[test]
    fn snapshot_shows_live_session_as_working_and_sets_pid() {
        let collector = Collector::with_claude_dir(claude_fixtures(), DEFAULT_ACTIVE_WINDOW_MS);
        let base = claude::parse_session(&claude_fixtures().join("sess-basic.jsonl")).unwrap();
        let now = base.last_event_at + 30_000;
        let sessions =
            collector.snapshot_with(now, &alive_claude("sess-basic"), &HashMap::new());
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
        let sessions = collector.snapshot_with(now, &HashMap::new(), &HashMap::new());
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
            collector.snapshot_with(now, &alive_claude("sess-basic"), &HashMap::new());
        let found = sessions
            .iter()
            .find(|s| s.id == "sess-basic")
            .expect("alive idle session should remain");
        assert!(matches!(found.status, Status::Idle));
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
        let sessions = collector.snapshot_with(now, &HashMap::new(), &codex_alive(rollout));
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
            DEFAULT_ACTIVE_WINDOW_MS,
        );
        let rollout = codex_dir.join("rollout-sample.jsonl");
        let base = codex::parse_session(&rollout).unwrap();
        let now = base.last_event_at + 30_000;
        let count = collector
            .snapshot_with(now, &HashMap::new(), &codex_alive(rollout))
            .iter()
            .filter(|s| s.id == "codex-1")
            .count();
        assert_eq!(count, 1);
    }
}
