use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Claude,
    Codex,
    Cursor,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Starting,
    Working,
    Idle,
    WaitingInput,
    Ended,
    Dead,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache: u64,
}

/// Where a context-window limit came from (drives a UI trust signal).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LimitSource {
    /// Reported in-band by the provider (Codex `model_context_window`).
    Reported,
    /// Looked up from a static model table.
    ModelTable,
    /// Read from `~/.codex*/models_cache.json` (reserved; not yet used).
    CacheFile,
    #[default]
    Unknown,
}

/// Which part of the context window a slice of tokens belongs to.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextCategory {
    SystemInstructions,
    ToolDefinitions,
    Memory,
    FileReads,
    Conversation,
    #[default]
    Other,
}

/// One reconstructed slice of the occupancy total (Layer 2).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategorySlice {
    pub name: ContextCategory,
    pub tokens: u64,
    /// True when the count is a heuristic estimate rather than tokenized text
    /// (e.g. Claude tool definitions, which the transcript never serializes).
    pub estimated: bool,
}

/// A compaction detected in the transcript.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Compaction {
    pub at: i64,
    pub pre_tokens: Option<u64>,
    pub post_tokens: Option<u64>,
    /// True for an explicit provider signal (Claude `compact_boundary`),
    /// false when inferred from a drop in occupancy (Codex).
    pub explicit: bool,
}

/// One point on the per-turn occupancy curve (the sawtooth).
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSnapshot {
    pub at: i64,
    pub used: u64,
}

/// Current context-window occupancy for a session.
///
/// Layer 1 (`used`/`limit`/`fill_pct`/`cached`/`fresh`/`history`/`compactions`)
/// is ground truth read straight from the numbers the provider writes to disk:
/// the size of the *most recent* model call, not a cumulative sum. Layer 2
/// (`categories`/`residual`) is a reconstructed per-category breakdown, tokenized
/// from the transcript and normalized so the category sum equals `used`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextWindow {
    /// Occupancy: tokens in the most recent prompt sent to the model.
    pub used: u64,
    /// Context-window size in tokens, when known.
    pub limit: Option<u64>,
    /// `used / limit * 100`; `None` when the limit is unknown.
    pub fill_pct: Option<f32>,
    pub limit_source: LimitSource,
    /// Portion of `used` served from cache.
    pub cached: u64,
    /// Portion of `used` that was fresh (non-cached) input.
    pub fresh: u64,
    /// Layer-2 breakdown; empty when no tokenizer ran. Sums exactly to `used`.
    pub categories: Vec<CategorySlice>,
    /// Tokens folded into the `Other` slice by normalization.
    pub residual: u64,
    /// Per-turn occupancy points, oldest first, capped at the most recent 20.
    pub history: Vec<ContextSnapshot>,
    pub compactions: Vec<Compaction>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileAction {
    Reading,
    Writing,
    Editing,
    Appending,
    Running,
    Searching,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEvent {
    pub path: String,
    pub action: FileAction,
    pub at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    pub id: String,
    pub tool: Tool,
    pub pid: Option<i32>,
    pub project_path: String,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub status: Status,
    pub current_action: Option<String>,
    pub started_at: i64,
    pub last_event_at: i64,
    pub tokens: Tokens,
    #[serde(default)]
    pub context: Option<ContextWindow>,
    pub title: Option<String>,
    pub recent_files: Vec<FileEvent>,
}

impl AgentSession {
    pub fn touch(&mut self, now_ms: i64) {
        self.last_event_at = now_ms;
    }

    pub fn mark_idle_if_stale(&mut self, now_ms: i64, idle_after_ms: i64) {
        if matches!(self.status, Status::Working) && now_ms - self.last_event_at >= idle_after_ms {
            self.status = Status::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AgentSession {
        AgentSession {
            id: "s1".into(),
            tool: Tool::Claude,
            pid: Some(123),
            project_path: "/Users/you/project".into(),
            branch: Some("main".into()),
            model: Some("claude-opus-4-8".into()),
            status: Status::Working,
            current_action: Some("running Bash".into()),
            started_at: 0,
            last_event_at: 1000,
            tokens: Tokens::default(),
            context: None,
            title: Some("demo session".into()),
            recent_files: vec![FileEvent {
                path: "/Users/you/project/src/main.rs".into(),
                action: FileAction::Editing,
                at: 1000,
            }],
        }
    }

    #[test]
    fn working_session_becomes_idle_when_stale() {
        let mut session = sample();
        session.mark_idle_if_stale(6000, 4000);
        assert!(matches!(session.status, Status::Idle));
    }

    #[test]
    fn working_session_stays_working_when_recent() {
        let mut session = sample();
        session.mark_idle_if_stale(2000, 4000);
        assert!(matches!(session.status, Status::Working));
    }

    #[test]
    fn touch_updates_last_event_timestamp_without_changing_status() {
        let mut session = sample();
        session.touch(2500);
        assert_eq!(session.last_event_at, 2500);
        assert!(matches!(session.status, Status::Working));
    }

    #[test]
    fn serializes_to_camel_case_for_ui() {
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(json.contains("\"projectPath\""));
        assert!(json.contains("\"currentAction\""));
        assert!(json.contains("\"lastEventAt\""));
        assert!(json.contains("\"tool\":\"claude\""));
        assert!(json.contains("\"status\":\"working\""));
    }

    #[test]
    fn file_event_serializes_to_camel_case_for_ui() {
        let event = FileEvent {
            path: "/tmp/notes.rs".into(),
            action: FileAction::Appending,
            at: 42,
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"path\":\"/tmp/notes.rs\""));
        assert!(json.contains("\"action\":\"appending\""));
        assert!(json.contains("\"at\":42"));
    }

    #[test]
    fn session_serializes_recent_files_and_title_without_cost() {
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(json.contains("\"recentFiles\""));
        assert!(json.contains("\"title\""));
        assert!(!json.contains("costUsd"), "cost was dropped: {json}");
    }
}
