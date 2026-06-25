//! Claude Code adapter.
//!
//! Reads a single Claude Code session JSONL file (one event per line, as written
//! under `~/.claude/projects/**/*.jsonl`) and normalizes it into an `AgentSession`.
//! Parsing is pure (path in, session out) so it can be unit-tested against committed
//! fixtures without touching the real home directory. Liveness/status is applied
//! separately by the collector (AMC-052), which knows the current time.

use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

use crate::collector::context::{self, OccupancyRaw, SegmentAccumulator, CLAUDE_TOOLDEF_ESTIMATE};
use crate::collector::session::{
    AgentSession, Compaction, ContextCategory, ContextSnapshot, FileAction, FileEvent, LimitSource,
    Status, TitleSource, Tokens, Tool,
};

/// Parse one Claude Code session log into a normalized session.
/// Returns `None` when the file cannot be read or carries no session id.
pub fn parse_session(path: &Path) -> Option<AgentSession> {
    let content = std::fs::read_to_string(path).ok()?;
    let when = file_mtime_ms(path);

    let mut id: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut model: Option<String> = None;
    let mut slug: Option<String> = None;
    let mut summary: Option<String> = None;
    let mut ai_title: Option<String> = None;
    let mut first_prompt: Option<String> = None;
    let mut tokens = Tokens::default();
    let mut files: Vec<FileEvent> = Vec::new();

    // Context-window reconstruction state (occupancy = the most recent call).
    let mut occ = OccupancyRaw::default();
    let mut seg = SegmentAccumulator::default();
    let mut history: Vec<ContextSnapshot> = Vec::new();
    let mut compactions: Vec<Compaction> = Vec::new();
    let mut tool_kinds: HashMap<String, String> = HashMap::new();

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

        if id.is_none() {
            id = event
                .get("sessionId")
                .and_then(|v| v.as_str())
                .map(String::from);
        }
        if cwd.is_none() {
            cwd = event.get("cwd").and_then(|v| v.as_str()).map(String::from);
        }
        if branch.is_none() {
            branch = event
                .get("gitBranch")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
        }
        if slug.is_none() {
            slug = event.get("slug").and_then(|v| v.as_str()).map(String::from);
        }
        if let Some(title) = event.get("aiTitle").and_then(|v| v.as_str()) {
            let title = title.trim();
            if !title.is_empty() {
                ai_title = Some(title.to_string());
            }
        }
        if event.get("type").and_then(|v| v.as_str()) == Some("summary") && summary.is_none() {
            summary = event
                .get("summary")
                .or_else(|| event.get("title"))
                .and_then(|v| v.as_str())
                .map(String::from);
        }

        if event.get("type").and_then(|v| v.as_str()) == Some("system") {
            if event.get("subtype").and_then(|v| v.as_str()) == Some("compact_boundary") {
                // Explicit compaction: record the drop so the sawtooth shows it
                // and occupancy rebases to the post-compaction size.
                let meta = event.get("compactMetadata");
                let pre = meta
                    .and_then(|m| m.get("preTokens"))
                    .and_then(|v| v.as_u64());
                let post = meta
                    .and_then(|m| m.get("postTokens"))
                    .and_then(|v| v.as_u64());
                compactions.push(Compaction {
                    at: ts,
                    pre_tokens: pre,
                    post_tokens: post,
                    explicit: true,
                });
                if let Some(p) = post {
                    occ.used = p;
                    history.push(ContextSnapshot { at: ts, used: p });
                }
            } else if let Some(text) = event.get("content").and_then(|v| v.as_str()) {
                seg.push(ContextCategory::SystemInstructions, text);
            }
        }

        let is_user = event.get("type").and_then(|v| v.as_str()) == Some("user");
        if let Some(message) = event.get("message") {
            if is_user && first_prompt.is_none() {
                first_prompt = first_prompt_from_claude_content(message.get("content"));
            }
            if let Some(found) = message.get("model").and_then(|v| v.as_str()) {
                model = Some(found.to_string());
            }
            if let Some(usage) = message.get("usage") {
                let count = |key: &str| usage.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
                tokens.input += count("input_tokens");
                tokens.output += count("output_tokens");
                tokens.cache +=
                    count("cache_read_input_tokens") + count("cache_creation_input_tokens");

                // Occupancy: the full prompt size of THIS call. The most recent
                // call wins (overwrite), and each call is a point on the sawtooth.
                let cache = count("cache_read_input_tokens") + count("cache_creation_input_tokens");
                let used = count("input_tokens") + cache;
                if used > 0 {
                    occ.used = used;
                    occ.cached = cache;
                    history.push(ContextSnapshot { at: ts, used });
                }
            }
            match message.get("content") {
                Some(Value::Array(blocks)) => {
                    for block in blocks {
                        let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if btype == "text" {
                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                seg.push(segment_for_text(text, is_user), text);
                            }
                        }
                        if btype == "tool_use" {
                            if let Some(id) = block.get("id").and_then(|v| v.as_str()) {
                                if let Some(name) = block.get("name").and_then(|v| v.as_str()) {
                                    tool_kinds.insert(id.to_string(), name.to_string());
                                }
                            }
                            if let (Some(name), Some(input)) = (
                                block.get("name").and_then(|v| v.as_str()),
                                block.get("input"),
                            ) {
                                // The tool call itself (paths, commands) is conversation.
                                seg.push(ContextCategory::Conversation, &input.to_string());
                                if let Some(fe) = file_event_for_tool(name, input, ts) {
                                    files.push(fe);
                                }
                            }
                        }
                        if btype == "tool_result" {
                            let kind = block
                                .get("tool_use_id")
                                .and_then(|v| v.as_str())
                                .and_then(|id| tool_kinds.get(id))
                                .map(String::as_str)
                                .unwrap_or("");
                            let cat = if matches!(kind, "Read" | "Grep" | "Glob") {
                                ContextCategory::FileReads
                            } else {
                                ContextCategory::Conversation
                            };
                            seg.push(cat, &result_text(block.get("content")));
                        }
                    }
                }
                Some(Value::String(text)) => {
                    seg.push(segment_for_text(text, is_user), text);
                }
                _ => {}
            }
        }
    }

    let id = id?;
    let project_path = cwd.unwrap_or_default();
    let can_rename = ai_title.as_ref().is_some_and(|s| !s.trim().is_empty());
    let (title, title_source) = derive_title(ai_title.as_deref().or(summary.as_deref()));
    let current_action = files.last().map(action_phrase);

    files.reverse();
    files.truncate(12);

    // Resolve the context limit from the model name (Claude never reports it).
    if let Some(m) = &model {
        occ.limit = context::claude_limit(m);
        occ.limit_source = if occ.limit.is_some() {
            LimitSource::ModelTable
        } else {
            LimitSource::Unknown
        };
    }
    let context_window = context::build(
        occ,
        &seg,
        history,
        compactions,
        CLAUDE_TOOLDEF_ESTIMATE,
        true,
    );

    Some(AgentSession {
        id,
        tool: Tool::Claude,
        pid: None,
        project_path,
        branch,
        model,
        status: Status::Working,
        current_action,
        started_at: when,
        last_event_at: when,
        tokens,
        context: Some(context_window),
        first_prompt,
        title,
        title_source,
        can_rename,
        recent_files: files,
        parent_session_id: None,
        connection_role: None,
        child_count: 0,
        run: None,
        process_tree: None,
        untracked: false,
    })
}

pub fn rename_ai_title(path: &Path, new_title: &str) -> Result<(), String> {
    let new_title = new_title.trim();
    if new_title.is_empty() {
        return Err("Agent name cannot be empty.".into());
    }

    let content = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    let mut changed = false;
    let mut lines = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut value: Value = serde_json::from_str(line).map_err(|err| err.to_string())?;
        if value.get("aiTitle").and_then(|v| v.as_str()).is_some() {
            value["aiTitle"] = Value::String(new_title.to_string());
            changed = true;
        }
        lines.push(serde_json::to_string(&value).map_err(|err| err.to_string())?);
    }

    if !changed {
        return Err("This Claude session has no writable aiTitle.".into());
    }

    let mut next = lines.join("\n");
    next.push('\n');
    std::fs::write(path, next).map_err(|err| err.to_string())
}

/// Last-modified time of a file in epoch milliseconds (0 if unavailable).
pub(crate) fn file_mtime_ms(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Final path segment, e.g. "/a/b/c.rs" -> "c.rs".
pub(crate) fn basename(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

/// Truncate to `max` characters, adding an ellipsis when shortened.
fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let kept: String = value.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// Classify a free-text block: user text quoting a memory file is `Memory`,
/// everything else is `Conversation`.
fn segment_for_text(text: &str, is_user: bool) -> ContextCategory {
    if is_user
        && (text.contains("CLAUDE.md") || text.contains("AGENTS.md") || text.contains("# claudeMd"))
    {
        ContextCategory::Memory
    } else {
        ContextCategory::Conversation
    }
}

/// First real user text from a Claude message. Tool-result user messages are
/// provider plumbing, not the assignment Karim typed.
fn first_prompt_from_claude_content(content: Option<&Value>) -> Option<String> {
    let raw = match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => {
            let mut out = String::new();
            for block in blocks {
                if block.get("type").and_then(|v| v.as_str()) != Some("text") {
                    continue;
                }
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(text);
                }
            }
            out
        }
        _ => String::new(),
    };
    clean_prompt(raw)
}

fn clean_prompt(raw: String) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// Flatten a `tool_result` block's `content` (string or array of text blocks)
/// into plain text for tokenizing.
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => {
            let mut out = String::new();
            for block in blocks {
                if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                    out.push_str(t);
                    out.push('\n');
                }
            }
            out
        }
        _ => String::new(),
    }
}

/// Pick the real provider title when one exists; otherwise leave title absent.
fn derive_title(provider_title: Option<&str>) -> (Option<String>, TitleSource) {
    if let Some(s) = provider_title {
        let s = s.trim();
        if !s.is_empty() {
            return (Some(truncate(s, 64)), TitleSource::Provider);
        }
    }
    (None, TitleSource::Fallback)
}

/// First whitespace-delimited token of a redirect target, ignoring fd dups (`>&1`).
fn redirect_target(rest: &str) -> Option<String> {
    let file = rest.trim().split_whitespace().next().unwrap_or("");
    if file.is_empty() || file.starts_with('&') {
        None
    } else {
        Some(file.to_string())
    }
}

/// Index of a real single `>` redirect (preceded by a space, not part of `>>`).
fn find_single_redirect(cmd: &str) -> Option<usize> {
    let bytes = cmd.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'>' {
            let prev = if i == 0 { b' ' } else { bytes[i - 1] };
            let next = if i + 1 < bytes.len() {
                bytes[i + 1]
            } else {
                b' '
            };
            if next != b'>' && prev == b' ' {
                return Some(i);
            }
        }
    }
    None
}

/// Classify a shell command into a file action: append/write on redirect, else run.
pub(crate) fn classify_bash(cmd: &str) -> (FileAction, String) {
    if let Some(idx) = cmd.find(">>") {
        if let Some(file) = redirect_target(&cmd[idx + 2..]) {
            return (FileAction::Appending, file);
        }
    }
    if let Some(idx) = find_single_redirect(cmd) {
        if let Some(file) = redirect_target(&cmd[idx + 1..]) {
            return (FileAction::Writing, file);
        }
    }
    (FileAction::Running, truncate(cmd.trim(), 48))
}

/// Map a `tool_use` block to a file-activity event, or `None` if it touches no file.
fn file_event_for_tool(name: &str, input: &Value, at: i64) -> Option<FileEvent> {
    let str_field = |key: &str| {
        input
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    match name {
        "Read" => str_field("file_path").map(|p| FileEvent {
            path: p,
            action: FileAction::Reading,
            at,
        }),
        "Write" => str_field("file_path").map(|p| FileEvent {
            path: p,
            action: FileAction::Writing,
            at,
        }),
        "Edit" | "MultiEdit" => str_field("file_path").map(|p| FileEvent {
            path: p,
            action: FileAction::Editing,
            at,
        }),
        "NotebookEdit" => str_field("notebook_path")
            .or_else(|| str_field("file_path"))
            .map(|p| FileEvent {
                path: p,
                action: FileAction::Editing,
                at,
            }),
        "Grep" | "Glob" => str_field("pattern")
            .or_else(|| str_field("path"))
            .map(|p| FileEvent {
                path: p,
                action: FileAction::Searching,
                at,
            }),
        "Bash" => input.get("command").and_then(|v| v.as_str()).map(|cmd| {
            let (action, path) = classify_bash(cmd);
            FileEvent { path, action, at }
        }),
        _ => None,
    }
}

/// One-line "Editing styles.css" style phrase for the most recent activity.
pub(crate) fn action_phrase(event: &FileEvent) -> String {
    let verb = match event.action {
        FileAction::Reading => "Reading",
        FileAction::Writing => "Writing",
        FileAction::Editing => "Editing",
        FileAction::Appending => "Appending to",
        FileAction::Running => "Running",
        FileAction::Searching => "Searching",
    };
    let short = if matches!(event.action, FileAction::Running) {
        event.path.clone()
    } else {
        basename(&event.path)
    };
    format!("{verb} {short}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::session::TitleSource;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/claude")
            .join(name)
    }

    fn temp_session_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mobi-board-{}-{}", name, std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("session.jsonl")
    }

    #[test]
    fn parse_basic_extracts_core_fields() {
        let s = parse_session(&fixture("sess-basic.jsonl")).expect("should parse");
        assert_eq!(s.id, "sess-basic");
        assert_eq!(s.project_path, "/Users/demo/proj");
        assert_eq!(s.branch.as_deref(), Some("feature/x"));
        assert_eq!(s.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(s.tokens.input, 300);
        assert_eq!(s.tokens.output, 60);
        assert_eq!(s.tokens.cache, 120);
        assert!(s.title.is_none());
        assert!(matches!(s.title_source, TitleSource::Fallback));
        assert!(!s.can_rename);
        assert!(matches!(s.tool, Tool::Claude));
        assert_eq!(
            s.first_prompt.as_deref(),
            Some("Add a retry helper with backoff")
        );
    }

    #[test]
    fn parse_basic_builds_file_activity_log_newest_first() {
        let s = parse_session(&fixture("sess-basic.jsonl")).unwrap();
        assert_eq!(s.recent_files.len(), 4);
        assert!(matches!(s.recent_files[0].action, FileAction::Running));
        assert!(s.recent_files[0].path.contains("cargo test"));
        assert!(matches!(s.recent_files[1].action, FileAction::Appending));
        assert_eq!(s.recent_files[1].path, "build.log");
        assert!(s
            .recent_files
            .iter()
            .any(|e| matches!(e.action, FileAction::Editing) && e.path.ends_with("util.ts")));
        assert!(s
            .recent_files
            .iter()
            .any(|e| matches!(e.action, FileAction::Reading)));
        assert_eq!(s.current_action.as_deref(), Some("Running cargo test"));
    }

    #[test]
    fn activity_events_keep_their_transcript_timestamps() {
        let s = parse_session(&fixture("sess-basic.jsonl")).unwrap();
        let newest = context::parse_iso8601_ms("2026-06-19T10:00:20.000Z").unwrap();
        let appended = context::parse_iso8601_ms("2026-06-19T10:00:15.000Z").unwrap();

        assert_eq!(s.recent_files[0].at, newest);
        assert_eq!(s.recent_files[1].at, appended);
    }

    #[test]
    fn context_occupancy_uses_last_call_not_spend_sum() {
        let s = parse_session(&fixture("sess-context.jsonl")).unwrap();
        let ctx = s.context.expect("context window present");
        // Occupancy is the most recent call's prompt size (2 + 60000 + 500),
        // NOT the cumulative spend sum that `tokens` accumulates.
        assert_eq!(ctx.used, 60_502);
        assert_eq!(ctx.cached, 60_500);
        assert_eq!(ctx.fresh, 2);
        assert_eq!(ctx.limit, Some(200_000));
        assert!(matches!(ctx.limit_source, LimitSource::ModelTable));
        assert!((ctx.fill_pct.unwrap() - 30.251).abs() < 0.1);
    }

    #[test]
    fn context_history_shows_sawtooth_across_compaction() {
        let s = parse_session(&fixture("sess-context.jsonl")).unwrap();
        let ctx = s.context.unwrap();
        // rise (145000), rise (151002), compaction drop (60000), rise (60502)
        let used: Vec<u64> = ctx.history.iter().map(|h| h.used).collect();
        assert_eq!(used, vec![145_000, 151_002, 60_000, 60_502]);
        assert_eq!(ctx.compactions.len(), 1);
        assert!(ctx.compactions[0].explicit);
        assert_eq!(ctx.compactions[0].pre_tokens, Some(151_002));
        assert_eq!(ctx.compactions[0].post_tokens, Some(60_000));
    }

    #[test]
    fn context_breakdown_reconciles_to_used() {
        let s = parse_session(&fixture("sess-context.jsonl")).unwrap();
        let ctx = s.context.unwrap();
        let sum: u64 = ctx.categories.iter().map(|c| c.tokens).sum();
        assert_eq!(
            sum, ctx.used,
            "categories must sum exactly to the authoritative total"
        );
        assert!(ctx
            .categories
            .iter()
            .any(|c| matches!(c.name, ContextCategory::Memory)));
        assert!(ctx
            .categories
            .iter()
            .any(|c| matches!(c.name, ContextCategory::ToolDefinitions) && c.estimated));
    }

    #[test]
    fn parse_summary_prefers_summary_title_over_command_prompt() {
        let s = parse_session(&fixture("session-summary.jsonl")).unwrap();
        assert_eq!(s.title.as_deref(), Some("Refactor auth module"));
        assert!(matches!(s.title_source, TitleSource::Provider));
        assert!(!s.can_rename);
        assert_eq!(s.model.as_deref(), Some("claude-sonnet-4-6"));
        assert!(s
            .recent_files
            .iter()
            .any(|e| matches!(e.action, FileAction::Writing) && e.path.ends_with("login.ts")));
    }

    #[test]
    fn parse_prefers_ai_title_and_does_not_use_first_prompt_as_name() {
        let s = parse_session(&fixture("session-ai-title.jsonl")).unwrap();
        assert_eq!(s.title.as_deref(), Some("Real Claude Chat Name"));
        assert!(matches!(s.title_source, TitleSource::Provider));
        assert!(s.can_rename);
        assert_ne!(
            s.title.as_deref(),
            Some("This is the first prompt and should not be the agent name")
        );
        assert_eq!(
            s.first_prompt.as_deref(),
            Some("This is the first prompt and should not be the agent name")
        );
    }

    #[test]
    fn parse_without_provider_title_falls_back_to_slug_not_first_prompt() {
        let s = parse_session(&fixture("sess-basic.jsonl")).unwrap();
        assert!(s.title.is_none());
        assert!(matches!(s.title_source, TitleSource::Fallback));
        assert!(!s.can_rename);
    }

    #[test]
    fn rename_ai_title_updates_existing_provider_name_only() {
        let path = temp_session_path("claude-rename-supported");
        std::fs::copy(fixture("session-ai-title.jsonl"), &path).unwrap();

        rename_ai_title(&path, "Renamed Claude Chat").unwrap();

        let parsed = parse_session(&path).unwrap();
        assert_eq!(parsed.title.as_deref(), Some("Renamed Claude Chat"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("\"aiTitle\":\"Renamed Claude Chat\""));
        assert!(content
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok()));
    }

    #[test]
    fn rename_ai_title_rejects_files_without_provider_name() {
        let path = temp_session_path("claude-rename-unsupported");
        std::fs::copy(fixture("sess-basic.jsonl"), &path).unwrap();

        assert!(rename_ai_title(&path, "Should not write").is_err());
        let parsed = parse_session(&path).unwrap();
        assert!(parsed.title.is_none());
    }

    #[test]
    fn derive_title_follows_precedence_without_prompt_fallback() {
        assert_eq!(
            derive_title(Some("Real title")).0.as_deref(),
            Some("Real title")
        );
        assert!(derive_title(None).0.is_none());
    }

    #[test]
    fn classify_bash_detects_redirects() {
        assert!(matches!(
            classify_bash("echo hi >> out.log").0,
            FileAction::Appending
        ));
        assert_eq!(classify_bash("echo hi >> out.log").1, "out.log");
        assert!(matches!(
            classify_bash("echo hi > result.txt").0,
            FileAction::Writing
        ));
        assert_eq!(classify_bash("echo hi > result.txt").1, "result.txt");
        assert!(matches!(
            classify_bash("cargo test 2>&1").0,
            FileAction::Running
        ));
    }

    #[test]
    fn file_event_skips_non_file_tools() {
        let read = file_event_for_tool("Read", &serde_json::json!({"file_path": "/a/b.rs"}), 0);
        assert!(matches!(read.unwrap().action, FileAction::Reading));
        let skill = file_event_for_tool("Skill", &serde_json::json!({"skill": "x"}), 0);
        assert!(skill.is_none());
    }

    #[test]
    fn action_phrase_reads_like_a_sentence() {
        let running = FileEvent {
            path: "cargo test".into(),
            action: FileAction::Running,
            at: 0,
        };
        assert_eq!(action_phrase(&running), "Running cargo test");
        let editing = FileEvent {
            path: "/a/b/styles.css".into(),
            action: FileAction::Editing,
            at: 0,
        };
        assert_eq!(action_phrase(&editing), "Editing styles.css");
    }

    #[test]
    #[ignore = "reads the real ~/.claude tree; run locally with -- --ignored --nocapture"]
    fn smoke_parse_newest_real_session() {
        fn walk(dir: &Path, newest: &mut Option<(std::time::SystemTime, PathBuf)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, newest);
                } else if path.extension().and_then(|x| x.to_str()) == Some("jsonl") {
                    if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                        if newest.as_ref().map_or(true, |(t, _)| modified > *t) {
                            *newest = Some((modified, path));
                        }
                    }
                }
            }
        }

        let home = std::env::var("HOME").expect("HOME");
        let base = Path::new(&home).join(".claude/projects");
        let mut newest = None;
        walk(&base, &mut newest);
        let (_, path) = newest.expect("at least one real session file");
        let s = parse_session(&path).expect("parse real session");
        eprintln!(
            "REAL id={} title={:?} model={:?} branch={:?} in/out/cache={}/{}/{} files={} action={:?}",
            s.id,
            s.title,
            s.model,
            s.branch,
            s.tokens.input,
            s.tokens.output,
            s.tokens.cache,
            s.recent_files.len(),
            s.current_action,
        );
        assert!(!s.id.is_empty());
        assert!(!s.project_path.is_empty());
    }
}
