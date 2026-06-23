//! Hermes adapter (Nous Research desktop agent).
//!
//! Unlike Claude/Codex (one process + one JSONL file per session), Hermes runs
//! a single long-lived daemon that records every session in one SQLite database
//! at `~/.hermes/state.db` (WAL mode). This adapter reads that DB **read-only**
//! and returns the local coding sessions (`source='cli'` with a working
//! directory). DB timestamps are epoch *seconds*; we convert to milliseconds.
//!
//! Hermes records *every* CLI exchange as a session row — including internal
//! probes (e.g. "Reply exactly: AUTOSTART_DISABLED_OK") and one-line greetings,
//! which it leaves open, untitled, and with zero tool calls. Those are not
//! agents, so we keep only sessions with real work: a tool call, an
//! auto-generated title (Hermes only titles substantive conversations), or
//! substantial generated output.
//!
//! That last clause matters for *live* runs: Hermes writes a session's message
//! and tool-call rows only when a turn completes, but the row's token counters
//! climb in real time. So an agent mid-turn shows `tool_call_count = 0` and no
//! title yet — while already having burned hundreds of thousands of input
//! tokens. Probes top out around ~25 output tokens (a canned acknowledgment);
//! a real agent emits thousands, so `output_tokens > PROBE_OUTPUT_CEILING`
//! admits the working session without resurrecting the greetings.
//!
//! Status is left at `Idle` and `pid` unset here: the collector applies daemon
//! liveness, the recency window, and the working/idle split, exactly as it does
//! for Codex.
//!
//! [`snapshot_sessions`] returns the lightweight session rows. The per-session
//! *activity* — live context-window occupancy and the files an agent has touched —
//! lives in the `messages` table and is reconstructed lazily by
//! [`reconstruct_activity`], which the collector calls only for the handful of
//! Hermes cards it is actually about to show (tokenizing every historical session
//! on every poll would be wasteful).

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use rusqlite::{params_from_iter, Connection, OpenFlags};
use serde_json::Value;

use crate::collector::context::{self, OccupancyRaw, SegmentAccumulator};
use crate::collector::session::{
    AgentSession, ContextCategory, ContextWindow, FileAction, FileEvent, LimitSource, RunStats,
    Status, TitleSource, Tokens, Tool,
};

/// Output-token ceiling for a no-work probe/greeting. A canned probe reply
/// ("AUTOSTART_DISABLED_OK") is a handful of tokens; observed greetings top out
/// near ~25. A real agent's first turn emits thousands, so this floor (with a
/// generous margin) separates a live working session from a greeting even before
/// it has logged a tool call or earned a title.
const PROBE_OUTPUT_CEILING: i64 = 200;

/// Read all local Hermes coding sessions from the state DB (read-only).
/// Returns an empty vec if the DB is missing or unreadable.
pub fn snapshot_sessions(db_path: &Path) -> Vec<AgentSession> {
    let conn = match Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(conn) => conn,
        Err(_) => return Vec::new(),
    };

    let mut stmt = match conn.prepare(
        "SELECT s.id, s.title, s.model, s.cwd, s.started_at, s.ended_at, \
                s.input_tokens, s.output_tokens, \
                s.cache_read_tokens, s.cache_write_tokens, \
                s.message_count, s.tool_call_count, s.api_call_count, \
                s.estimated_cost_usd, s.actual_cost_usd, s.cost_status, \
                s.end_reason, s.model_config, \
                (SELECT MAX(m.timestamp) FROM messages m WHERE m.session_id = s.id) \
         FROM sessions s \
         WHERE s.source = 'cli' AND s.cwd IS NOT NULL AND s.cwd <> '' AND s.archived = 0 \
           AND (s.tool_call_count > 0 OR s.title IS NOT NULL OR s.output_tokens > ?1) \
         ORDER BY s.started_at DESC",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return Vec::new(),
    };

    let rows = stmt.query_map([PROBE_OUTPUT_CEILING], |row| {
        let run = build_run_stats(
            row.get::<_, Option<i64>>(10)?.unwrap_or(0),
            row.get::<_, Option<i64>>(11)?.unwrap_or(0),
            row.get::<_, Option<i64>>(12)?.unwrap_or(0),
            row.get::<_, Option<f64>>(13)?,
            row.get::<_, Option<f64>>(14)?,
            row.get::<_, Option<String>>(15)?,
            row.get::<_, Option<String>>(16)?,
            row.get::<_, Option<String>>(17)?,
        );
        Ok(build_session(
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, f64>(4)?,
            row.get::<_, Option<f64>>(5)?,
            row.get::<_, Option<i64>>(6)?.unwrap_or(0),
            row.get::<_, Option<i64>>(7)?.unwrap_or(0),
            row.get::<_, Option<i64>>(8)?.unwrap_or(0),
            row.get::<_, Option<i64>>(9)?.unwrap_or(0),
            row.get::<_, Option<f64>>(18)?,
            run,
        ))
    });

    match rows {
        Ok(iter) => iter.filter_map(Result::ok).collect(),
        Err(_) => Vec::new(),
    }
}

fn secs_to_ms(secs: f64) -> i64 {
    (secs * 1000.0) as i64
}

/// Build the run-telemetry block for one session row.
///
/// `cost_usd` is reported only when the provider gives a positive figure
/// (prefer `actual`, fall back to `estimated`); Fugu/Sakana writes `0.0`, which
/// we treat as "unknown" rather than "free" — token burn is the real signal.
/// `max_turns` and `effort` are parsed out of the `model_config` JSON blob.
#[allow(clippy::too_many_arguments)]
fn build_run_stats(
    message_count: i64,
    tool_call_count: i64,
    api_call_count: i64,
    estimated_cost: Option<f64>,
    actual_cost: Option<f64>,
    cost_status: Option<String>,
    end_reason: Option<String>,
    model_config: Option<String>,
) -> RunStats {
    let (max_turns, effort) = parse_model_config(model_config.as_deref());
    let cost_usd = actual_cost
        .filter(|c| *c > 0.0)
        .or_else(|| estimated_cost.filter(|c| *c > 0.0));
    RunStats {
        turns: api_call_count.max(0) as u32,
        max_turns,
        tool_calls: tool_call_count.max(0) as u32,
        messages: message_count.max(0) as u32,
        effort,
        cost_usd,
        cost_status: cost_status.filter(|s| !s.trim().is_empty()),
        end_reason: end_reason.filter(|s| !s.trim().is_empty()),
    }
}

/// Pull `max_iterations` and `reasoning_config.effort` out of the `model_config`
/// JSON, e.g. `{"max_iterations": 800, "reasoning_config": {"effort": "xhigh"}}`.
/// Returns `(None, None)` for missing/malformed config rather than failing.
fn parse_model_config(raw: Option<&str>) -> (Option<u32>, Option<String>) {
    let Some(raw) = raw else {
        return (None, None);
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return (None, None);
    };
    let max_turns = value
        .get("max_iterations")
        .and_then(serde_json::Value::as_u64)
        .map(|n| n as u32);
    let effort = value
        .get("reasoning_config")
        .and_then(|rc| rc.get("effort"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string);
    (max_turns, effort)
}

/// Pure mapping of one `sessions` row to an `AgentSession`. `last_event_at`
/// prefers the newest message timestamp, then `ended_at`, then `started_at`.
#[allow(clippy::too_many_arguments)]
fn build_session(
    id: String,
    title: Option<String>,
    model: Option<String>,
    cwd: String,
    started_at: f64,
    ended_at: Option<f64>,
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    last_msg: Option<f64>,
    run: RunStats,
) -> AgentSession {
    let last_secs = last_msg.or(ended_at).unwrap_or(started_at);
    let (title, title_source) = match title {
        Some(t) if !t.trim().is_empty() => (Some(t), TitleSource::Provider),
        _ => (None, TitleSource::Fallback),
    };
    AgentSession {
        id,
        tool: Tool::Hermes,
        pid: None,
        project_path: cwd,
        branch: None,
        model,
        status: Status::Idle,
        current_action: None,
        started_at: secs_to_ms(started_at),
        last_event_at: secs_to_ms(last_secs),
        tokens: Tokens {
            input: input.max(0) as u64,
            output: output.max(0) as u64,
            cache: (cache_read.max(0) + cache_write.max(0)) as u64,
        },
        context: None,
        title,
        title_source,
        can_rename: false,
        recent_files: Vec::new(),
        run: Some(run),
        process_tree: None,
        untracked: false,
    }
}

/// Per-session activity reconstructed from the `messages` table: the live
/// context-window occupancy and the recent file/command touches.
#[derive(Clone, Debug, Default)]
pub struct HermesActivity {
    pub context: Option<ContextWindow>,
    pub recent_files: Vec<FileEvent>,
}

/// Reconstruct one session's live context window and file activity from its
/// messages, using an already-open read-only connection.
///
/// Hermes never writes per-turn token usage (`messages.token_count` is always
/// NULL), so occupancy is reconstructed by tokenizing the **active** (in-context)
/// messages with the same `o200k_base` encoder the Claude/Codex adapters use.
/// The `active` flag is Hermes' own "in context / pruned" marker, so inactive
/// rows are excluded from `used` — that is literally the out-of-context split.
/// The window limit comes from the model table (Fugu = 1M).
///
/// File touches come from each assistant turn's `tool_calls` (recorded for every
/// turn, active or not). Returns defaults (no context, no files) if the messages
/// query fails — e.g. an older/minimal schema — so enrichment can never break a
/// snapshot.
pub fn reconstruct_activity(
    conn: &Connection,
    session_id: &str,
    model: Option<&str>,
) -> HermesActivity {
    let session_ids = related_session_ids(conn, session_id);

    // The system prompt counts toward occupancy but lives on the session row.
    let system_prompt: Option<String> = conn
        .query_row(
            "SELECT system_prompt FROM sessions WHERE id = ?1",
            [session_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten();

    let placeholders = vec!["?"; session_ids.len()].join(",");
    let sql = format!(
        "SELECT role, content, tool_calls, timestamp, active \
         FROM messages WHERE session_id IN ({placeholders}) ORDER BY timestamp ASC"
    );
    let mut stmt = match conn.prepare(&sql) {
        Ok(stmt) => stmt,
        Err(_) => return HermesActivity::default(),
    };

    let rows = stmt.query_map(params_from_iter(session_ids.iter()), |row| {
        Ok((
            row.get::<_, Option<String>>(0)?,           // role
            row.get::<_, Option<String>>(1)?,           // content
            row.get::<_, Option<String>>(2)?,           // tool_calls
            row.get::<_, f64>(3)?,                       // timestamp (epoch secs)
            row.get::<_, Option<i64>>(4)?.unwrap_or(1),  // active
        ))
    });
    let rows = match rows {
        Ok(rows) => rows,
        Err(_) => return HermesActivity::default(),
    };

    let mut seg = SegmentAccumulator::default();
    if let Some(sys) = system_prompt.as_deref() {
        seg.push(ContextCategory::SystemInstructions, sys);
    }
    let mut files: Vec<FileEvent> = Vec::new();

    for (role, content, tool_calls, ts, active) in rows.flatten() {
        let at = secs_to_ms(ts);

        // File/command touches: extracted from every assistant tool call, whether
        // or not the message is still in context.
        if let Some(tc) = tool_calls.as_deref() {
            extract_file_events(tc, at, &mut files);
        }

        // Occupancy: only *active* (in-context) text counts toward `used`.
        if active == 0 {
            continue;
        }
        if let Some(text) = content.as_deref() {
            // Tool results are overwhelmingly file reads / search output; user and
            // assistant turns are the conversation.
            let cat = match role.as_deref() {
                Some("tool") => ContextCategory::FileReads,
                _ => ContextCategory::Conversation,
            };
            seg.push(cat, text);
        }
        if let Some(tc) = tool_calls.as_deref() {
            // The assistant's tool-call JSON is part of the prompt it sent.
            seg.push(ContextCategory::Conversation, tc);
        }
    }

    // Newest touch first, capped to match the Claude adapter's window.
    files.sort_by(|a, b| b.at.cmp(&a.at));
    files.truncate(12);

    HermesActivity {
        context: build_context(&seg, model),
        recent_files: files,
    }
}

/// Hermes may split one visible CLI run across child/subagent rows. The visible
/// root keeps the working directory, while child rows often hold the real tool
/// activity. Keep this defensive: older schemas simply return the requested id.
fn related_session_ids(conn: &Connection, session_id: &str) -> Vec<String> {
    let mut stmt = match conn.prepare("SELECT id, parent_session_id, model_config FROM sessions") {
        Ok(stmt) => stmt,
        Err(_) => return vec![session_id.to_string()],
    };
    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(_) => return vec![session_id.to_string()],
    };

    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows.flatten() {
        let (id, parent, model_config) = row;
        if let Some(parent) = parent.filter(|p| !p.trim().is_empty()) {
            children.entry(parent).or_default().push(id.clone());
        }
        if let Some(delegate) = delegate_from(model_config.as_deref()) {
            children.entry(delegate).or_default().push(id.clone());
        }
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = VecDeque::from([session_id.to_string()]);
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id.clone()) {
            continue;
        }
        out.push(id.clone());
        if let Some(kids) = children.get(&id) {
            for child in kids {
                queue.push_back(child.clone());
            }
        }
    }

    if out.is_empty() {
        vec![session_id.to_string()]
    } else {
        out
    }
}

fn delegate_from(raw: Option<&str>) -> Option<String> {
    let value = serde_json::from_str::<Value>(raw?).ok()?;
    value
        .get("_delegate_from")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
}

/// Assemble a [`ContextWindow`] from the tokenized in-context segments. `used` is
/// the segment total, so [`context::build`]'s normalization is an identity and
/// the category breakdown sums to it exactly. Returns `None` for an empty window
/// (nothing to show).
fn build_context(seg: &SegmentAccumulator, model: Option<&str>) -> Option<ContextWindow> {
    let used = seg.total_tokens();
    if used == 0 {
        return None;
    }
    let limit = hermes_context_limit(model);
    let occ = OccupancyRaw {
        used,
        // Hermes gives no honest per-turn cache split, so the whole window reads
        // as fresh rather than inventing a cached portion.
        cached: 0,
        limit,
        limit_source: if limit.is_some() {
            LimitSource::ModelTable
        } else {
            LimitSource::Unknown
        },
    };
    Some(context::build(occ, seg, Vec::new(), Vec::new(), 0, true))
}

/// Static context-window limit for a Hermes model. The Fugu family (Sakana) is
/// 1M tokens; anything else is unknown.
fn hermes_context_limit(model: Option<&str>) -> Option<u64> {
    let model = model?.to_ascii_lowercase();
    if model.contains("fugu") {
        Some(1_000_000)
    } else {
        None
    }
}

/// Pull file/command touches out of one message's `tool_calls` JSON.
///
/// Hermes records tool calls as `[{"function":{"name":..,"arguments":"{json}"}}]`
/// where `arguments` is itself a JSON *string*. Only the tools that name a target
/// become events; bookkeeping tools (todo, memory, skill_view) are skipped.
fn extract_file_events(tool_calls_json: &str, at: i64, out: &mut Vec<FileEvent>) {
    let Ok(calls) = serde_json::from_str::<Value>(tool_calls_json) else {
        return;
    };
    let Some(arr) = calls.as_array() else {
        return;
    };
    for call in arr {
        let func = call.get("function");
        let name = func
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let args: Value = func
            .and_then(|f| f.get("arguments"))
            .and_then(Value::as_str)
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or(Value::Null);
        let arg = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_string);

        let event = match name {
            "read_file" => arg("path").map(|p| (FileAction::Reading, p)),
            "write_file" => arg("path").map(|p| (FileAction::Writing, p)),
            "patch" => arg("path").map(|p| (FileAction::Editing, p)),
            "search_files" => arg("path")
                .or_else(|| arg("pattern"))
                .map(|p| (FileAction::Searching, p)),
            "terminal" => arg("command").map(|c| (FileAction::Running, truncate(&c, 48))),
            _ => None,
        };
        if let Some((action, path)) = event {
            if !path.trim().is_empty() {
                out.push(FileEvent { path, action, at });
            }
        }
    }
}

/// Trim `s` to at most `max` characters, appending an ellipsis when shortened.
fn truncate(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::path::PathBuf;

    /// Create a throwaway Hermes-shaped DB at a unique temp path and run `body`
    /// to populate it. Returns the path.
    fn make_db(tag: &str, body: impl FnOnce(&Connection)) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("mobius-hermes-{}-{}.db", std::process::id(), tag));
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
        body(&conn);
        path
    }

    fn insert_cli_session(conn: &Connection, id: &str, started: f64, cwd: Option<&str>) {
        conn.execute(
            "INSERT INTO sessions
                (id, source, model, model_config, started_at, ended_at, end_reason,
                 input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                 message_count, tool_call_count, api_call_count,
                 estimated_cost_usd, actual_cost_usd, cost_status,
                 cwd, title, archived)
             VALUES (?1,'cli','fugu-ultra',
                 '{\"max_iterations\": 800, \"reasoning_config\": {\"effort\": \"xhigh\"}}',
                 ?2,NULL,'compression',
                 100,20,5,3,
                 42,7,11,
                 0.0,NULL,'unknown',
                 ?3,'Demo Session',0)",
            rusqlite::params![id, started, cwd],
        )
        .unwrap();
    }

    #[test]
    fn returns_cli_session_with_mapped_fields() {
        let db = make_db("basic", |conn| {
            insert_cli_session(conn, "s-1", 1000.0, Some("/work/proj"));
            conn.execute(
                "INSERT INTO messages (session_id, timestamp) VALUES ('s-1', 1500.0)",
                [],
            )
            .unwrap();
        });
        let sessions = snapshot_sessions(&db);
        let s = sessions
            .iter()
            .find(|s| s.id == "s-1")
            .expect("cli session present");
        assert!(matches!(s.tool, Tool::Hermes));
        assert_eq!(s.project_path, "/work/proj");
        assert_eq!(s.model.as_deref(), Some("fugu-ultra"));
        // seconds -> milliseconds
        assert_eq!(s.started_at, 1_000_000);
        // last_event_at from MAX(messages.timestamp)
        assert_eq!(s.last_event_at, 1_500_000);
        // tokens: input/output direct, cache = read + write
        assert_eq!(s.tokens.input, 100);
        assert_eq!(s.tokens.output, 20);
        assert_eq!(s.tokens.cache, 8);
        assert_eq!(s.title.as_deref(), Some("Demo Session"));
        assert!(matches!(s.title_source, TitleSource::Provider));
        assert!(!s.can_rename);
        assert!(matches!(s.status, Status::Idle)); // collector overrides later
        assert!(s.pid.is_none());
    }

    #[test]
    fn maps_run_telemetry_from_session_columns_and_model_config() {
        let db = make_db("run", |conn| {
            insert_cli_session(conn, "r-1", 1000.0, Some("/work/proj"));
        });
        let sessions = snapshot_sessions(&db);
        let run = sessions
            .iter()
            .find(|s| s.id == "r-1")
            .expect("session present")
            .run
            .as_ref()
            .expect("run telemetry present");
        // turns/tool_calls/messages come straight from the count columns
        assert_eq!(run.turns, 11); // api_call_count
        assert_eq!(run.tool_calls, 7);
        assert_eq!(run.messages, 42);
        // model_config JSON yields the iteration cap and effort
        assert_eq!(run.max_turns, Some(800));
        assert_eq!(run.effort.as_deref(), Some("xhigh"));
        // Fugu/Sakana writes 0.0 cost -> treated as unknown, not free
        assert_eq!(run.cost_usd, None);
        assert_eq!(run.cost_status.as_deref(), Some("unknown"));
        assert_eq!(run.end_reason.as_deref(), Some("compression"));
    }

    #[test]
    fn run_cost_prefers_positive_actual_then_estimated() {
        let db = make_db("cost", |conn| {
            // estimated set, actual null -> use estimated
            conn.execute(
                "INSERT INTO sessions (id, source, started_at, cwd, title, tool_call_count,
                     estimated_cost_usd, actual_cost_usd, archived)
                 VALUES ('est','cli',1000.0,'/p','Est',1,1.25,NULL,0)",
                [],
            )
            .unwrap();
            // both set -> actual wins
            conn.execute(
                "INSERT INTO sessions (id, source, started_at, cwd, title, tool_call_count,
                     estimated_cost_usd, actual_cost_usd, archived)
                 VALUES ('act','cli',1000.0,'/p','Act',1,1.25,3.50,0)",
                [],
            )
            .unwrap();
        });
        let sessions = snapshot_sessions(&db);
        let cost = |id: &str| {
            sessions
                .iter()
                .find(|s| s.id == id)
                .unwrap()
                .run
                .as_ref()
                .unwrap()
                .cost_usd
        };
        assert_eq!(cost("est"), Some(1.25));
        assert_eq!(cost("act"), Some(3.50));
    }

    #[test]
    fn excludes_no_work_sessions_but_keeps_real_ones() {
        // Hermes logs internal probes ("Reply exactly: ...") and one-line
        // greetings as open cli sessions with no tools and no title. Those are
        // not agents; real work has either a tool call or an auto-title.
        let db = make_db("nowork", |conn| {
            conn.execute(
                "INSERT INTO sessions (id, source, started_at, cwd, title, tool_call_count, archived)
                 VALUES ('ghost','cli',1000.0,'/work/proj',NULL,0,0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, source, started_at, cwd, title, tool_call_count, archived)
                 VALUES ('worker','cli',1000.0,'/work/proj',NULL,3,0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, source, started_at, cwd, title, tool_call_count, archived)
                 VALUES ('titled','cli',1000.0,'/work/proj','Real Chat',0,0)",
                [],
            )
            .unwrap();
        });
        let ids: Vec<String> = snapshot_sessions(&db).into_iter().map(|s| s.id).collect();
        assert!(
            !ids.contains(&"ghost".to_string()),
            "no-work ghost must be hidden"
        );
        assert!(
            ids.contains(&"worker".to_string()),
            "tool-using session must show"
        );
        assert!(
            ids.contains(&"titled".to_string()),
            "titled session must show"
        );
    }

    #[test]
    fn includes_live_run_burning_tokens_before_it_flushes_work() {
        // A live agent mid-turn: token counters have climbed (Hermes updates them
        // in real time) but the row still shows zero tool calls and no title
        // because message/tool rows flush only at turn end. It must show; a probe
        // (tiny output) sitting next to it must not.
        let db = make_db("liverun", |conn| {
            conn.execute(
                "INSERT INTO sessions (id, source, started_at, cwd, title, tool_call_count,
                     input_tokens, output_tokens, archived)
                 VALUES ('live','cli',1000.0,'/work/proj',NULL,0,287031,16150,0)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, source, started_at, cwd, title, tool_call_count,
                     input_tokens, output_tokens, archived)
                 VALUES ('probe','cli',1000.0,'/work/proj',NULL,0,32432,18,0)",
                [],
            )
            .unwrap();
        });
        let ids: Vec<String> = snapshot_sessions(&db).into_iter().map(|s| s.id).collect();
        assert!(
            ids.contains(&"live".to_string()),
            "token-burning live run must show even before it flushes tool rows"
        );
        assert!(
            !ids.contains(&"probe".to_string()),
            "a tiny-output probe must stay hidden"
        );
    }

    #[test]
    fn excludes_non_cli_sources() {
        let db = make_db("source", |conn| {
            conn.execute(
                "INSERT INTO sessions
                    (id, source, started_at, cwd, archived)
                 VALUES ('tg-1','telegram',1000.0,'/work/proj',0)",
                [],
            )
            .unwrap();
        });
        assert!(snapshot_sessions(&db).is_empty());
    }

    #[test]
    fn excludes_sessions_without_cwd() {
        let db = make_db("nocwd", |conn| {
            insert_cli_session(conn, "s-null", 1000.0, None);
            insert_cli_session(conn, "s-empty", 1000.0, Some(""));
        });
        assert!(snapshot_sessions(&db).is_empty());
    }

    #[test]
    fn excludes_archived_sessions() {
        let db = make_db("arch", |conn| {
            conn.execute(
                "INSERT INTO sessions
                    (id, source, started_at, cwd, archived)
                 VALUES ('a-1','cli',1000.0,'/work/proj',1)",
                [],
            )
            .unwrap();
        });
        assert!(snapshot_sessions(&db).is_empty());
    }

    #[test]
    fn last_event_falls_back_to_started_at_without_messages() {
        let db = make_db("fallback", |conn| {
            insert_cli_session(conn, "s-2", 2000.0, Some("/work/proj"));
        });
        let sessions = snapshot_sessions(&db);
        let s = sessions.iter().find(|s| s.id == "s-2").unwrap();
        assert_eq!(s.last_event_at, 2_000_000);
    }

    #[test]
    fn missing_db_returns_empty() {
        let path = std::env::temp_dir().join("mobius-hermes-does-not-exist.db");
        let _ = std::fs::remove_file(&path);
        assert!(snapshot_sessions(&path).is_empty());
    }

    // ---- activity reconstruction (context window + files touched) ----

    /// A DB with the full schema the activity reconstruction reads: a
    /// `system_prompt` column on sessions and `role`/`content`/`tool_calls`/
    /// `active` columns on messages.
    fn make_activity_db(tag: &str, body: impl FnOnce(&Connection)) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "mobius-hermes-act-{}-{}.db",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_file(&path);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (
                 id TEXT PRIMARY KEY, system_prompt TEXT,
                 parent_session_id TEXT, model_config TEXT
             );
             CREATE TABLE messages (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 session_id TEXT NOT NULL, role TEXT, content TEXT,
                 tool_calls TEXT, timestamp REAL NOT NULL,
                 active INTEGER NOT NULL DEFAULT 1
             );",
        )
        .unwrap();
        body(&conn);
        path
    }

    fn tool_call(name: &str, args_json: &str) -> String {
        // Mirror Hermes' shape: `arguments` is a JSON *string* inside the array.
        let escaped = args_json.replace('"', "\\\"");
        format!(r#"[{{"function": {{"name": "{name}", "arguments": "{escaped}"}}}}]"#)
    }

    #[test]
    fn reconstructs_context_window_from_active_messages() {
        let db = make_activity_db("ctx", |conn| {
            conn.execute(
                "INSERT INTO sessions (id, system_prompt) VALUES ('s1', 'You are a helpful coding agent.')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (session_id, role, content, timestamp, active)
                 VALUES ('s1','user','please refactor the parser for me today',1000.0,1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (session_id, role, content, timestamp, active)
                 VALUES ('s1','tool','fn parse() { /* a chunk of file contents here */ }',1001.0,1)",
                [],
            )
            .unwrap();
        });
        let conn = Connection::open(&db).unwrap();
        let act = reconstruct_activity(&conn, "s1", Some("fugu-ultra"));
        let ctx = act.context.expect("context reconstructed");
        assert!(ctx.used > 0, "tokenized occupancy should be positive");
        assert_eq!(ctx.limit, Some(1_000_000), "Fugu window is 1M");
        assert!(matches!(ctx.limit_source, LimitSource::ModelTable));
        // fill_pct = used / 1M * 100, a small positive number
        let pct = ctx.fill_pct.expect("fill pct known");
        assert!(pct > 0.0 && pct < 1.0, "got {pct}");
        // No invented cache split.
        assert_eq!(ctx.cached, 0);
        assert_eq!(ctx.fresh, ctx.used);
        // Category breakdown sums exactly to used.
        let sum: u64 = ctx.categories.iter().map(|c| c.tokens).sum();
        assert_eq!(sum, ctx.used);
        assert!(ctx
            .categories
            .iter()
            .any(|c| matches!(c.name, ContextCategory::SystemInstructions)));
    }

    #[test]
    fn inactive_messages_are_out_of_context_but_still_logged() {
        // An active read + a pruned (active=0) read: the pruned one must not add
        // to occupancy, but both files are still surfaced in the activity log.
        let db = make_activity_db("split", |conn| {
            conn.execute(
                "INSERT INTO sessions (id, system_prompt) VALUES ('s1', NULL)",
                [],
            )
            .unwrap();
            let live = tool_call("read_file", r#"{"path": "/proj/live.rs"}"#);
            let pruned = tool_call("read_file", r#"{"path": "/proj/pruned.rs"}"#);
            conn.execute(
                "INSERT INTO messages (session_id, role, content, tool_calls, timestamp, active)
                 VALUES ('s1','assistant','in context here', ?1, 2000.0, 1)",
                rusqlite::params![live],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (session_id, role, content, tool_calls, timestamp, active)
                 VALUES ('s1','assistant','pruned out of window', ?1, 1000.0, 0)",
                rusqlite::params![pruned],
            )
            .unwrap();
        });
        let conn = Connection::open(&db).unwrap();
        let act = reconstruct_activity(&conn, "s1", Some("fugu"));
        // Both reads logged, newest first.
        let paths: Vec<&str> = act.recent_files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["/proj/live.rs", "/proj/pruned.rs"]);
        // Occupancy is derived only from the active row's text.
        let ctx = act.context.expect("context present");
        assert!(ctx.used > 0);
    }

    #[test]
    fn reconstructs_activity_from_child_sessions() {
        let db = make_activity_db("children", |conn| {
            conn.execute(
                "INSERT INTO sessions (id, system_prompt, parent_session_id, model_config)
                 VALUES ('root', 'You are running the local agent.', NULL, NULL)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, system_prompt, parent_session_id, model_config)
                 VALUES ('child', NULL, 'root', NULL)",
                [],
            )
            .unwrap();
            let read = tool_call("read_file", r#"{"path": "/work/src/lib.rs"}"#);
            let terminal = tool_call("terminal", r#"{"command": "cargo test --lib hermes"}"#);
            conn.execute(
                "INSERT INTO messages (session_id, role, content, tool_calls, timestamp, active)
                 VALUES ('root','assistant','checking the root run', ?1, 1000.0, 1)",
                rusqlite::params![read],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO messages (session_id, role, content, tool_calls, timestamp, active)
                 VALUES ('child','assistant','running the child worker', ?1, 2000.0, 1)",
                rusqlite::params![terminal],
            )
            .unwrap();
        });
        let conn = Connection::open(&db).unwrap();
        let act = reconstruct_activity(&conn, "root", Some("fugu-ultra"));
        let paths: Vec<&str> = act.recent_files.iter().map(|f| f.path.as_str()).collect();

        assert!(
            paths.iter().any(|p| p.contains("cargo test --lib hermes")),
            "child terminal event should appear in root activity: {paths:?}"
        );
        assert!(
            paths.contains(&"/work/src/lib.rs"),
            "root file event should still appear beside child activity: {paths:?}"
        );
        assert!(
            act.context.as_ref().map_or(0, |ctx| ctx.used) > 0,
            "active child text should contribute to reconstructed context"
        );
    }

    #[test]
    fn extracts_file_events_for_each_tool_kind() {
        let db = make_activity_db("files", |conn| {
            conn.execute(
                "INSERT INTO sessions (id, system_prompt) VALUES ('s1', NULL)",
                [],
            )
            .unwrap();
            let calls = [
                (tool_call("read_file", r#"{"path": "/p/read.rs", "limit": 10}"#), 10.0),
                (tool_call("write_file", r#"{"path": "/p/new.rs", "content": "x"}"#), 20.0),
                (tool_call("patch", r#"{"path": "/p/edit.rs", "mode": "replace"}"#), 30.0),
                (tool_call("search_files", r#"{"path": "/p", "pattern": "*.rs"}"#), 40.0),
                (tool_call("terminal", r#"{"command": "cargo test --all --workspace --verbose --color=always"}"#), 50.0),
                (tool_call("todo", r#"{"todos": []}"#), 60.0),
            ];
            for (i, (tc, ts)) in calls.iter().enumerate() {
                conn.execute(
                    "INSERT INTO messages (session_id, role, tool_calls, timestamp, active)
                     VALUES ('s1','assistant', ?1, ?2, 1)",
                    rusqlite::params![tc, ts],
                )
                .unwrap();
                let _ = i;
            }
        });
        let conn = Connection::open(&db).unwrap();
        let act = reconstruct_activity(&conn, "s1", Some("fugu-ultra"));
        // todo is skipped; the other five become events, newest first.
        assert_eq!(act.recent_files.len(), 5);
        let by_path = |p: &str| act.recent_files.iter().find(|f| f.path.contains(p)).cloned();
        assert!(matches!(
            by_path("read.rs").unwrap().action,
            FileAction::Reading
        ));
        assert!(matches!(
            by_path("new.rs").unwrap().action,
            FileAction::Writing
        ));
        assert!(matches!(
            by_path("edit.rs").unwrap().action,
            FileAction::Editing
        ));
        // search maps to Searching on the searched directory
        let search = act
            .recent_files
            .iter()
            .find(|f| matches!(f.action, FileAction::Searching))
            .unwrap();
        assert_eq!(search.path, "/p");
        // terminal command is Running and truncated with an ellipsis
        let term = act
            .recent_files
            .iter()
            .find(|f| matches!(f.action, FileAction::Running))
            .unwrap();
        assert!(term.path.starts_with("cargo test"));
        assert!(term.path.ends_with('…'), "long command truncated: {}", term.path);
        // newest first
        assert_eq!(act.recent_files.first().unwrap().at, secs_to_ms(50.0));
    }

    #[test]
    fn reconstruct_is_defensive_on_minimal_schema() {
        // The collector's older test DBs lack the columns we read; reconstruction
        // must degrade to empty, never panic or error.
        let path =
            std::env::temp_dir().join(format!("mobius-hermes-min-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY);
             CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT, timestamp REAL);",
        )
        .unwrap();
        let act = reconstruct_activity(&conn, "whatever", Some("fugu"));
        assert!(act.context.is_none());
        assert!(act.recent_files.is_empty());
    }

    #[test]
    #[ignore = "reads the real ~/.hermes/state.db; run locally with -- --ignored --nocapture"]
    fn reconstruct_real_hermes_session() {
        let db = dirs::home_dir().unwrap().join(".hermes/state.db");
        let conn = Connection::open_with_flags(&db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        // Newest session that has actually flushed tool calls.
        let id: String = conn
            .query_row(
                "SELECT id FROM sessions WHERE source='cli' AND tool_call_count > 0 \
                 ORDER BY started_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let model: Option<String> = conn
            .query_row("SELECT model FROM sessions WHERE id=?1", [&id], |row| {
                row.get(0)
            })
            .unwrap();
        let act = reconstruct_activity(&conn, &id, model.as_deref());
        let ctx = act.context.expect("real session has context");
        println!(
            "session {id}: used={} limit={:?} fill={:?}% files={}",
            ctx.used,
            ctx.limit,
            ctx.fill_pct,
            act.recent_files.len()
        );
        for c in &ctx.categories {
            println!("  {:?} = {}", c.name, c.tokens);
        }
        for f in act.recent_files.iter().take(12) {
            println!("  {:?} {}", f.action, f.path);
        }
        assert!(ctx.used > 0);
        assert!(!act.recent_files.is_empty(), "a tool-using session has files");
    }

    #[test]
    fn hermes_context_limit_known_for_fugu_only() {
        assert_eq!(hermes_context_limit(Some("fugu")), Some(1_000_000));
        assert_eq!(hermes_context_limit(Some("fugu-ultra")), Some(1_000_000));
        assert_eq!(hermes_context_limit(Some("gpt-5.5")), None);
        assert_eq!(hermes_context_limit(None), None);
    }
}
