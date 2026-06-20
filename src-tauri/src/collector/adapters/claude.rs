//! Claude Code adapter.
//!
//! Reads a single Claude Code session JSONL file (one event per line, as written
//! under `~/.claude/projects/**/*.jsonl`) and normalizes it into an `AgentSession`.
//! Parsing is pure (path in, session out) so it can be unit-tested against committed
//! fixtures without touching the real home directory. Liveness/status is applied
//! separately by the collector (AMC-052), which knows the current time.

use std::path::Path;

use serde_json::Value;

use crate::collector::session::{AgentSession, FileAction, FileEvent, Status, Tokens, Tool};

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
    let mut first_prompt: Option<String> = None;
    let mut tokens = Tokens::default();
    let mut files: Vec<FileEvent> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let event: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => continue,
        };

        if id.is_none() {
            id = event.get("sessionId").and_then(|v| v.as_str()).map(String::from);
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
        if event.get("type").and_then(|v| v.as_str()) == Some("summary") && summary.is_none() {
            summary = event
                .get("summary")
                .or_else(|| event.get("title"))
                .and_then(|v| v.as_str())
                .map(String::from);
        }

        let is_user = event.get("type").and_then(|v| v.as_str()) == Some("user");
        if let Some(message) = event.get("message") {
            if let Some(found) = message.get("model").and_then(|v| v.as_str()) {
                model = Some(found.to_string());
            }
            if let Some(usage) = message.get("usage") {
                let count = |key: &str| usage.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
                tokens.input += count("input_tokens");
                tokens.output += count("output_tokens");
                tokens.cache += count("cache_read_input_tokens") + count("cache_creation_input_tokens");
            }
            match message.get("content") {
                Some(Value::Array(blocks)) => {
                    for block in blocks {
                        let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if is_user && btype == "text" && first_prompt.is_none() {
                            if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                                if !is_command_wrapper(text) && !text.trim().is_empty() {
                                    first_prompt = Some(text.to_string());
                                }
                            }
                        }
                        if btype == "tool_use" {
                            if let (Some(name), Some(input)) =
                                (block.get("name").and_then(|v| v.as_str()), block.get("input"))
                            {
                                if let Some(fe) = file_event_for_tool(name, input, when) {
                                    files.push(fe);
                                }
                            }
                        }
                    }
                }
                Some(Value::String(text)) => {
                    if is_user
                        && first_prompt.is_none()
                        && !is_command_wrapper(text)
                        && !text.trim().is_empty()
                    {
                        first_prompt = Some(text.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    let id = id?;
    let project_path = cwd.unwrap_or_default();
    let title = derive_title(
        summary.as_deref(),
        first_prompt.as_deref(),
        slug.as_deref(),
        &project_path,
    );
    let current_action = files.last().map(action_phrase);

    files.reverse();
    files.truncate(12);

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
        title: Some(title),
        recent_files: files,
    })
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

/// True for synthetic slash-command / caveat prompts that are not real user text.
fn is_command_wrapper(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("<command-") || t.starts_with("<local-command-") || t.starts_with("<command_")
}

/// Collapse whitespace and trim a user prompt down to a one-line title.
fn clean_prompt(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(collapsed.trim(), 64)
}

fn dekebab(slug: &str) -> String {
    slug.trim().replace('-', " ")
}

/// Pick a human-readable session name: summary -> first real prompt -> slug -> folder.
fn derive_title(
    summary: Option<&str>,
    first_prompt: Option<&str>,
    slug: Option<&str>,
    project_path: &str,
) -> String {
    if let Some(s) = summary {
        let s = s.trim();
        if !s.is_empty() {
            return truncate(s, 64);
        }
    }
    if let Some(p) = first_prompt {
        let c = clean_prompt(p);
        if !c.is_empty() {
            return c;
        }
    }
    if let Some(sl) = slug {
        let d = dekebab(sl);
        if !d.is_empty() {
            return d;
        }
    }
    basename(project_path)
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
            let next = if i + 1 < bytes.len() { bytes[i + 1] } else { b' ' };
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
        "Read" => str_field("file_path").map(|p| FileEvent { path: p, action: FileAction::Reading, at }),
        "Write" => str_field("file_path").map(|p| FileEvent { path: p, action: FileAction::Writing, at }),
        "Edit" | "MultiEdit" => {
            str_field("file_path").map(|p| FileEvent { path: p, action: FileAction::Editing, at })
        }
        "NotebookEdit" => str_field("notebook_path")
            .or_else(|| str_field("file_path"))
            .map(|p| FileEvent { path: p, action: FileAction::Editing, at }),
        "Grep" | "Glob" => str_field("pattern")
            .or_else(|| str_field("path"))
            .map(|p| FileEvent { path: p, action: FileAction::Searching, at }),
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
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/claude")
            .join(name)
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
        assert_eq!(s.title.as_deref(), Some("Add a retry helper with backoff"));
        assert!(matches!(s.tool, Tool::Claude));
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
    fn parse_summary_prefers_summary_title_over_command_prompt() {
        let s = parse_session(&fixture("session-summary.jsonl")).unwrap();
        assert_eq!(s.title.as_deref(), Some("Refactor auth module"));
        assert_eq!(s.model.as_deref(), Some("claude-sonnet-4-6"));
        assert!(s
            .recent_files
            .iter()
            .any(|e| matches!(e.action, FileAction::Writing) && e.path.ends_with("login.ts")));
    }

    #[test]
    fn derive_title_follows_precedence() {
        assert_eq!(
            derive_title(Some("Real title"), Some("hi"), Some("a-b"), "/x/proj"),
            "Real title"
        );
        assert_eq!(
            derive_title(None, Some("Make it faster"), Some("a-b"), "/x/proj"),
            "Make it faster"
        );
        assert_eq!(
            derive_title(None, None, Some("my-cool-thing"), "/x/proj"),
            "my cool thing"
        );
        assert_eq!(derive_title(None, None, None, "/x/proj"), "proj");
    }

    #[test]
    fn classify_bash_detects_redirects() {
        assert!(matches!(classify_bash("echo hi >> out.log").0, FileAction::Appending));
        assert_eq!(classify_bash("echo hi >> out.log").1, "out.log");
        assert!(matches!(classify_bash("echo hi > result.txt").0, FileAction::Writing));
        assert_eq!(classify_bash("echo hi > result.txt").1, "result.txt");
        assert!(matches!(classify_bash("cargo test 2>&1").0, FileAction::Running));
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
        let running = FileEvent { path: "cargo test".into(), action: FileAction::Running, at: 0 };
        assert_eq!(action_phrase(&running), "Running cargo test");
        let editing = FileEvent { path: "/a/b/styles.css".into(), action: FileAction::Editing, at: 0 };
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
