use serde::{Deserialize, Serialize};

use crate::collector::scanner::ProcessNode;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Claude,
    Codex,
    Cursor,
    Hermes,
    /// Generic catch-all for a signature-matched process the scanner found that
    /// isn't one of the first-class providers (e.g. ollama, aider, goose).
    Agent,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionRole {
    Orchestrator,
    SubAgent,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEvent {
    pub path: String,
    pub action: FileAction,
    pub at: i64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TitleSource {
    Provider,
    #[default]
    Fallback,
}

/// Run-level telemetry for autonomous agents (currently Hermes/Fugu one-shot
/// builds). Absent for Claude/Codex sessions, where it stays `None` and is
/// omitted from the serialized payload entirely.
///
/// Honest about the source: Fugu/Sakana does not write a dollar figure to
/// `state.db` (`estimated_cost_usd` is `0.0`, `actual_cost_usd` is null), so
/// `cost_usd` is usually `None` and **token burn is the de-facto cost signal**.
/// `turns` (model round-trips) against `max_turns` (the iteration cap) is the
/// other half of the burn picture.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStats {
    /// Model round-trips so far (`api_call_count`) — the real "turn" counter.
    pub turns: u32,
    /// Iteration cap from `model_config.max_iterations`, when known.
    pub max_turns: Option<u32>,
    /// Tool invocations recorded (`tool_call_count`).
    pub tool_calls: u32,
    /// Messages exchanged (`message_count`).
    pub messages: u32,
    /// Reasoning effort from `model_config` (e.g. "xhigh"), when known.
    pub effort: Option<String>,
    /// Provider cost in USD when reported and non-zero; `None` when unknown
    /// (the common case for Fugu/Sakana — see the struct docs).
    pub cost_usd: Option<f64>,
    /// Cost reliability flag from the provider (e.g. "unknown").
    pub cost_status: Option<String>,
    /// Why the session ended: "compression", "cli_close", or `None` while live.
    pub end_reason: Option<String>,
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
    pub title_source: TitleSource,
    pub can_rename: bool,
    pub recent_files: Vec<FileEvent>,
    /// Parent session row for providers that split one visible run into multiple
    /// local connections. Hermes/Fugu uses this for sub-agent rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// User-facing role inside a provider connection family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_role: Option<ConnectionRole>,
    /// Descendant sub-agent rows under this session. Zero is omitted from JSON.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub child_count: u32,
    /// Run-level telemetry (Hermes/Fugu). `None` for Claude/Codex and omitted
    /// from the JSON payload when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run: Option<RunStats>,
    /// Live process subtree for this agent, attached by the process scanner when
    /// the session's PID matches a scanned agent root. `None` (and omitted) when
    /// no scan match was found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_tree: Option<ProcessNode>,
    /// True only for synthesized cards: a signature-matched process the scanner
    /// found that has no session card of its own ("running without me knowing").
    #[serde(default)]
    pub untracked: bool,
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
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
            title_source: TitleSource::Provider,
            can_rename: true,
            recent_files: vec![FileEvent {
                path: "/Users/you/project/src/main.rs".into(),
                action: FileAction::Editing,
                at: 1000,
            }],
            parent_session_id: None,
            connection_role: None,
            child_count: 0,
            run: None,
            process_tree: None,
            untracked: false,
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
    fn hermes_tool_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Tool::Hermes).unwrap(), "\"hermes\"");
    }

    #[test]
    fn agent_tool_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Tool::Agent).unwrap(), "\"agent\"");
    }

    #[test]
    fn process_tree_omitted_when_absent_and_untracked_defaults_false() {
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(
            !json.contains("processTree"),
            "process_tree must be omitted when None: {json}"
        );
        assert!(json.contains("\"untracked\":false"));
    }

    #[test]
    fn process_tree_serializes_to_camel_case_when_present() {
        use crate::collector::scanner::ProcessNode;
        let mut session = sample();
        session.untracked = true;
        session.process_tree = Some(ProcessNode {
            pid: 200,
            command: "/x/bin/hermes -z go".into(),
            children: vec![ProcessNode {
                pid: 300,
                command: "cargo build".into(),
                children: vec![],
            }],
        });
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("\"processTree\""));
        assert!(json.contains("\"untracked\":true"));
        assert!(json.contains("\"pid\":300"));
        assert!(json.contains("\"children\""));
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
        // A Claude/Codex session carries no run telemetry, so the whole `run`
        // block (and its `costUsd`) is omitted from the payload.
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(json.contains("\"recentFiles\""));
        assert!(json.contains("\"title\""));
        assert!(json.contains("\"titleSource\":\"provider\""));
        assert!(json.contains("\"canRename\":true"));
        assert!(
            !json.contains("\"run\""),
            "run block must be omitted: {json}"
        );
        assert!(!json.contains("costUsd"), "cost was dropped: {json}");
    }

    #[test]
    fn run_stats_serialize_to_camel_case_when_present() {
        let mut session = sample();
        session.run = Some(RunStats {
            turns: 110,
            max_turns: Some(800),
            tool_calls: 63,
            messages: 220,
            effort: Some("xhigh".into()),
            cost_usd: None,
            cost_status: Some("unknown".into()),
            end_reason: Some("compression".into()),
        });
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("\"run\""));
        assert!(json.contains("\"maxTurns\":800"));
        assert!(json.contains("\"toolCalls\":63"));
        assert!(json.contains("\"effort\":\"xhigh\""));
        assert!(json.contains("\"endReason\":\"compression\""));
        // costUsd is null here but the key is present under run
        assert!(json.contains("\"costUsd\":null"));
    }
}
