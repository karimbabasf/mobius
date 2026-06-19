pub mod adapters;
pub mod registry;
pub mod session;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::collector::adapters::{claude, codex};
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
/// sessions that are currently active. Parsed sessions are cached per file and
/// only re-parsed when the file's mtime changes, so an idle dashboard does no work.
/// Codex is scanned across multiple homes (the default profile plus isolated
/// profiles such as `~/.codex-karim`); their session-name indexes are merged.
pub struct Collector {
    claude_dir: PathBuf,
    codex_dirs: Vec<PathBuf>,
    codex_indexes: Vec<PathBuf>,
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
    pub fn snapshot(&self, now_ms: i64) -> Vec<AgentSession> {
        let mut files: Vec<(PathBuf, i64, Source)> = Vec::new();
        let mut claude_files = Vec::new();
        collect_active_jsonl(&self.claude_dir, now_ms, self.active_window_ms, &mut claude_files);
        for (path, mtime) in claude_files {
            files.push((path, mtime, Source::Claude));
        }
        for dir in &self.codex_dirs {
            let mut codex_files = Vec::new();
            collect_active_jsonl(dir, now_ms, self.active_window_ms, &mut codex_files);
            for (path, mtime) in codex_files {
                files.push((path, mtime, Source::Codex));
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
            let age = now_ms - entry.session.last_event_at;
            let Some(status) = status_for_age(age, self.active_window_ms) else {
                continue;
            };
            let mut session = entry.session.clone();
            if !seen.insert(session.id.clone()) {
                continue;
            }
            session.status = status;
            if matches!(session.tool, Tool::Codex) {
                if let Some(name) = thread_names.get(&session.id) {
                    session.title = Some(name.clone());
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

/// Map how long ago a session was last active into a display status.
/// Returns `None` once the session is older than the active window (hide it).
fn status_for_age(age_ms: i64, active_window_ms: i64) -> Option<Status> {
    if age_ms < WORKING_RECENCY_MS {
        Some(Status::Working)
    } else if age_ms < active_window_ms {
        Some(Status::Idle)
    } else {
        None
    }
}

/// Recursively collect `*.jsonl` files modified within `window_ms` of `now_ms`.
fn collect_active_jsonl(dir: &Path, now_ms: i64, window_ms: i64, out: &mut Vec<(PathBuf, i64)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            collect_active_jsonl(&path, now_ms, window_ms, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            if now_ms - mtime < window_ms {
                out.push((path, mtime));
            }
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

    #[test]
    fn status_for_age_maps_working_idle_and_expiry() {
        let window = DEFAULT_ACTIVE_WINDOW_MS;
        assert!(matches!(status_for_age(1_000, window), Some(Status::Working)));
        assert!(matches!(
            status_for_age(5 * 60 * 1000, window),
            Some(Status::Idle)
        ));
        assert!(status_for_age(11 * 60 * 1000, window).is_none());
    }

    #[test]
    fn snapshot_shows_recent_session_as_working() {
        let collector = Collector::with_claude_dir(claude_fixtures(), DEFAULT_ACTIVE_WINDOW_MS);
        let base = claude::parse_session(&claude_fixtures().join("session-basic.jsonl")).unwrap();
        let now = base.last_event_at + 30_000;
        let sessions = collector.snapshot(now);
        let found = sessions
            .iter()
            .find(|s| s.id == "sess-basic")
            .expect("recent session should be present");
        assert!(matches!(found.status, Status::Working));
    }

    #[test]
    fn snapshot_drops_sessions_older_than_window() {
        let collector = Collector::with_claude_dir(claude_fixtures(), DEFAULT_ACTIVE_WINDOW_MS);
        let base = claude::parse_session(&claude_fixtures().join("session-basic.jsonl")).unwrap();
        let now = base.last_event_at + 20 * 60 * 1000;
        let sessions = collector.snapshot(now);
        assert!(!sessions.iter().any(|s| s.id == "sess-basic"));
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
        let base = codex::parse_session(&codex_dir.join("rollout-sample.jsonl")).unwrap();
        let now = base.last_event_at + 30_000;
        let sessions = collector.snapshot(now);
        let found = sessions
            .iter()
            .find(|s| s.id == "codex-1")
            .expect("codex session should be present");
        assert!(matches!(found.tool, Tool::Codex));
        assert_eq!(found.title.as_deref(), Some("Build the thing"));
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
        let base = codex::parse_session(&codex_dir.join("rollout-sample.jsonl")).unwrap();
        let now = base.last_event_at + 30_000;
        let count = collector
            .snapshot(now)
            .iter()
            .filter(|s| s.id == "codex-1")
            .count();
        assert_eq!(count, 1);
    }
}
