//! Codex adapter (best-effort).
//!
//! Reads a Codex `rollout-*.jsonl` session log into an `AgentSession`. Codex stores
//! its session name separately in `~/.codex/session_index.jsonl` (`thread_name`),
//! which the collector joins on the session id. Token accounting in Codex logs is
//! intermittent, so tokens are best-effort (0 when absent).

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::collector::adapters::claude::{action_phrase, basename, classify_bash, file_mtime_ms};
use crate::collector::context::{self, OccupancyRaw, SegmentAccumulator};
use crate::collector::session::{
    AgentSession, Compaction, ContextCategory, ContextSnapshot, FileAction, FileEvent, LimitSource,
    Status, Tokens, Tool,
};

/// Parse one Codex rollout log into a normalized session (title is provisional;
/// the collector overrides it with the `thread_name` from the session index).
pub fn parse_session(path: &Path) -> Option<AgentSession> {
    let content = std::fs::read_to_string(path).ok()?;
    let when = file_mtime_ms(path);

    let mut id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut model: Option<String> = None;
    let mut tokens = Tokens::default();
    let mut files: Vec<FileEvent> = Vec::new();

    // Context-window reconstruction state (occupancy = the most recent call).
    let mut occ = OccupancyRaw::default();
    let mut seg = SegmentAccumulator::default();
    let mut history: Vec<ContextSnapshot> = Vec::new();
    let mut compactions: Vec<Compaction> = Vec::new();
    let mut prev_used: u64 = 0;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let event: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let ts = event
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(context::parse_iso8601_ms)
            .unwrap_or(when);
        let kind = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let payload = event.get("payload");
        match kind {
            "session_meta" => {
                if let Some(p) = payload {
                    if id.is_none() {
                        id = p.get("id").and_then(|v| v.as_str()).map(String::from);
                    }
                    if cwd.is_none() {
                        cwd = p.get("cwd").and_then(|v| v.as_str()).map(String::from);
                    }
                    // Tool schemas are sent once per session; count them once.
                    if let Some(tools) = p.get("tools").filter(|t| !t.is_null()) {
                        seg.set_tool_defs_once(&tools.to_string());
                    }
                    if let Some(instr) = p.get("instructions").and_then(|v| v.as_str()) {
                        seg.push(ContextCategory::SystemInstructions, instr);
                    }
                }
            }
            "turn_context" => {
                if let Some(p) = payload {
                    if let Some(m) = p.get("model").and_then(|v| v.as_str()) {
                        model = Some(m.to_string());
                    }
                    if cwd.is_none() {
                        cwd = p.get("cwd").and_then(|v| v.as_str()).map(String::from);
                    }
                }
            }
            "event_msg" => {
                if let Some(p) = payload {
                    if p.get("type").and_then(|v| v.as_str()) == Some("token_count") {
                        if let Some(info) = p.get("info").filter(|i| i.is_object()) {
                            add_tokens(&mut tokens, info);
                            capture_occupancy(
                                info,
                                ts,
                                &mut occ,
                                &mut history,
                                &mut compactions,
                                &mut prev_used,
                            );
                        }
                    }
                }
            }
            "response_item" => {
                if let Some(p) = payload {
                    match p.get("type").and_then(|v| v.as_str()) {
                        Some("function_call")
                            if p.get("name").and_then(|v| v.as_str()) == Some("exec_command") =>
                        {
                            if let Some(args) = p.get("arguments").and_then(|v| v.as_str()) {
                                if let Ok(parsed) = serde_json::from_str::<Value>(args) {
                                    if let Some(cmd) = parsed.get("cmd").and_then(|v| v.as_str()) {
                                        let (action, target) = classify_bash(cmd);
                                        files.push(FileEvent { path: target, action, at: when });
                                    }
                                }
                            }
                        }
                        Some("patch_apply_end") => {
                            if let Some(changes) = p.get("changes").and_then(|c| c.as_object()) {
                                for (file_path, change) in changes {
                                    let action = match change.get("type").and_then(|v| v.as_str()) {
                                        Some("add") => FileAction::Writing,
                                        _ => FileAction::Editing,
                                    };
                                    files.push(FileEvent {
                                        path: file_path.clone(),
                                        action,
                                        at: when,
                                    });
                                }
                            }
                        }
                        Some("message") => {
                            let text = message_text(p.get("content"));
                            let cat = if text.contains("AGENTS.md") {
                                ContextCategory::Memory
                            } else {
                                ContextCategory::Conversation
                            };
                            seg.push(cat, &text);
                        }
                        Some("function_call_output") => {
                            if let Some(out) = p.get("output").and_then(|v| v.as_str()) {
                                seg.push(ContextCategory::FileReads, out);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    let id = id?;
    let project_path = cwd.unwrap_or_default();
    let current_action = files.last().map(action_phrase);
    files.reverse();
    files.truncate(12);

    // Prefer the in-band `model_context_window`; fall back to a static table.
    if occ.limit.is_none() {
        if let Some(m) = &model {
            occ.limit = context::codex_limit(m);
            if occ.limit.is_some() {
                occ.limit_source = LimitSource::ModelTable;
            }
        }
    }
    let context_window = context::build(occ, &seg, history, compactions, 0, true);

    Some(AgentSession {
        id,
        tool: Tool::Codex,
        pid: None,
        title: Some(basename(&project_path)),
        project_path,
        branch: None,
        model,
        status: Status::Working,
        current_action,
        started_at: when,
        last_event_at: when,
        tokens,
        context: Some(context_window),
        recent_files: files,
    })
}

/// Load the Codex session id -> thread_name map from `session_index.jsonl`.
pub fn load_thread_names(index_path: &Path) -> HashMap<String, String> {
    let mut names = HashMap::new();
    let Ok(content) = std::fs::read_to_string(index_path) else {
        return names;
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let (Some(id), Some(name)) = (
            value.get("id").and_then(|v| v.as_str()),
            value.get("thread_name").and_then(|v| v.as_str()),
        ) {
            if !name.is_empty() {
                names.insert(id.to_string(), name.to_string());
            }
        }
    }
    names
}

/// Capture Layer-1 occupancy from a `token_count` payload's `info` object: the
/// most recent call's prompt size, its cached portion, the reported limit, and a
/// per-call history point. A large drop in occupancy is recorded as an inferred
/// compaction (Codex emits no explicit signal).
fn capture_occupancy(
    info: &Value,
    ts: i64,
    occ: &mut OccupancyRaw,
    history: &mut Vec<ContextSnapshot>,
    compactions: &mut Vec<Compaction>,
    prev_used: &mut u64,
) {
    // `last_token_usage` is the prompt size of the most recent call; for older
    // logs that only carry the cumulative block, fall back to it.
    let source = info
        .get("last_token_usage")
        .or_else(|| info.get("total_token_usage"));
    if let Some(usage) = source {
        if let Some(used) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
            // Inferred compaction: occupancy fell to under 60% of the prior call.
            if *prev_used > 0 && used < *prev_used * 3 / 5 {
                compactions.push(Compaction {
                    at: ts,
                    pre_tokens: Some(*prev_used),
                    post_tokens: Some(used),
                    explicit: false,
                });
            }
            occ.used = used;
            occ.cached = usage
                .get("cached_input_tokens")
                .or_else(|| usage.get("cache_read_input_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            history.push(ContextSnapshot { at: ts, used });
            *prev_used = used;
        }
    }
    if let Some(window) = info.get("model_context_window").and_then(|v| v.as_u64()) {
        occ.limit = Some(window);
        occ.limit_source = LimitSource::Reported;
    }
}

/// Flatten a Codex `message` payload's `content` array into plain text.
fn message_text(content: Option<&Value>) -> String {
    let Some(Value::Array(blocks)) = content else {
        return content.and_then(|v| v.as_str()).map(String::from).unwrap_or_default();
    };
    let mut out = String::new();
    for block in blocks {
        if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
            out.push_str(t);
            out.push('\n');
        }
    }
    out
}

/// Best-effort token extraction from a `token_count` payload's `info` object.
fn add_tokens(tokens: &mut Tokens, info: &Value) {
    let source = info
        .get("total_token_usage")
        .or_else(|| info.get("last_token_usage"))
        .unwrap_or(info);
    let get = |key: &str| source.get(key).and_then(|v| v.as_u64());
    if let Some(v) = get("input_tokens") {
        tokens.input = tokens.input.max(v);
    }
    if let Some(v) = get("output_tokens") {
        tokens.output = tokens.output.max(v);
    }
    if let Some(v) = get("cached_input_tokens").or_else(|| get("cache_read_input_tokens")) {
        tokens.cache = tokens.cache.max(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/codex")
            .join(name)
    }

    #[test]
    fn parse_extracts_codex_fields() {
        let s = parse_session(&fixture("rollout-sample.jsonl")).expect("should parse");
        assert_eq!(s.id, "codex-1");
        assert!(matches!(s.tool, Tool::Codex));
        assert_eq!(s.project_path, "/Users/demo/proj");
        assert_eq!(s.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(s.tokens.input, 111);
        assert_eq!(s.tokens.output, 22);
        assert_eq!(s.tokens.cache, 33);
    }

    #[test]
    fn parse_builds_file_activity_from_exec_and_patch() {
        let s = parse_session(&fixture("rollout-sample.jsonl")).unwrap();
        assert!(s
            .recent_files
            .iter()
            .any(|e| matches!(e.action, FileAction::Editing) && e.path.ends_with("lib.rs")));
        assert!(s
            .recent_files
            .iter()
            .any(|e| matches!(e.action, FileAction::Running) && e.path.contains("cargo build")));
        assert_eq!(s.current_action.as_deref(), Some("Editing lib.rs"));
    }

    #[test]
    fn context_occupancy_from_last_token_usage_and_window() {
        let s = parse_session(&fixture("rollout-context.jsonl")).unwrap();
        let ctx = s.context.expect("context window present");
        // Occupancy is the last call's `last_token_usage.input_tokens`, and the
        // limit comes straight from the in-band `model_context_window`.
        assert_eq!(ctx.used, 70_000);
        assert_eq!(ctx.cached, 40_000);
        assert_eq!(ctx.limit, Some(258_400));
        assert!(matches!(ctx.limit_source, LimitSource::Reported));
    }

    #[test]
    fn context_infers_compaction_from_occupancy_drop() {
        let s = parse_session(&fixture("rollout-context.jsonl")).unwrap();
        let ctx = s.context.unwrap();
        assert_eq!(ctx.history.iter().map(|h| h.used).collect::<Vec<_>>(), vec![120_000, 200_000, 70_000]);
        assert_eq!(ctx.compactions.len(), 1);
        assert!(!ctx.compactions[0].explicit, "Codex compaction is inferred, not explicit");
        assert_eq!(ctx.compactions[0].pre_tokens, Some(200_000));
        let sum: u64 = ctx.categories.iter().map(|c| c.tokens).sum();
        assert_eq!(sum, ctx.used);
    }

    #[test]
    fn load_thread_names_maps_id_to_title() {
        let names = load_thread_names(&fixture("session_index.jsonl"));
        assert_eq!(names.get("codex-1").map(String::as_str), Some("Build the thing"));
        assert_eq!(names.get("other-9").map(String::as_str), Some("Unrelated session"));
    }
}
