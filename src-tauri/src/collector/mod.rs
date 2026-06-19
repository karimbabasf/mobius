pub mod adapters;
pub mod registry;
pub mod session;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::collector::adapters::claude;
use crate::collector::session::{AgentSession, Status};

const DEFAULT_ACTIVE_WINDOW_MS: i64 = 10 * 60 * 1000;
const WORKING_RECENCY_MS: i64 = 90 * 1000;

struct CacheEntry {
    mtime: i64,
    session: AgentSession,
}

/// Live registry of agent sessions, built from on-disk agent logs.
///
/// `snapshot` re-scans the agent log directories on each call and returns the
/// sessions that are currently active. Parsed sessions are cached per file and
/// only re-parsed when the file's mtime changes, so an idle dashboard does no work.
pub struct Collector {
    claude_dir: PathBuf,
    active_window_ms: i64,
    cache: Mutex<HashMap<PathBuf, CacheEntry>>,
}

impl Collector {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        Self::with_claude_dir(
            home.join(".claude").join("projects"),
            DEFAULT_ACTIVE_WINDOW_MS,
        )
    }

    pub fn with_claude_dir(claude_dir: PathBuf, active_window_ms: i64) -> Self {
        Self {
            claude_dir,
            active_window_ms,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// Active sessions right now, newest activity first.
    pub fn snapshot(&self, now_ms: i64) -> Vec<AgentSession> {
        let mut files = Vec::new();
        collect_active_jsonl(&self.claude_dir, now_ms, self.active_window_ms, &mut files);

        let mut cache = self.cache.lock().expect("collector cache lock");
        // Forget files that are no longer active so the cache cannot grow unbounded.
        let active: std::collections::HashSet<PathBuf> =
            files.iter().map(|(path, _)| path.clone()).collect();
        cache.retain(|path, _| active.contains(path));

        let mut sessions = Vec::new();
        for (path, mtime) in files {
            let needs_parse = match cache.get(&path) {
                Some(entry) => entry.mtime != mtime,
                None => true,
            };
            if needs_parse {
                match claude::parse_session(&path) {
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
            if let Some(status) = status_for_age(age, self.active_window_ms) {
                let mut session = entry.session.clone();
                session.status = status;
                sessions.push(session);
            }
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

    fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/claude")
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
        let collector = Collector::with_claude_dir(fixtures_dir(), DEFAULT_ACTIVE_WINDOW_MS);
        let base = claude::parse_session(&fixtures_dir().join("session-basic.jsonl")).unwrap();
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
        let collector = Collector::with_claude_dir(fixtures_dir(), DEFAULT_ACTIVE_WINDOW_MS);
        let base = claude::parse_session(&fixtures_dir().join("session-basic.jsonl")).unwrap();
        let now = base.last_event_at + 20 * 60 * 1000;
        let sessions = collector.snapshot(now);
        assert!(!sessions.iter().any(|s| s.id == "sess-basic"));
    }
}
