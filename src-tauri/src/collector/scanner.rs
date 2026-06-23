//! OS-level agent-process scanner: a safety net for agents running on the
//! machine that the per-provider session stores never surface.
//!
//! Each provider adapter only knows about its own session store (Claude's
//! registry files, Codex's open transcripts, Hermes' `state.db`). A headless
//! launch — `hermes -z "$(cat prompt)" &` from a backgrounded shell — can be
//! doing real work while the dashboard shows nothing, along with the child
//! process tree it spawned (wrapper shell -> orchestrator -> subagent/tool).
//!
//! This module enumerates the process table once via `ps`, matches roots
//! against a curated **signature table** (known agent executables), and
//! assembles each match's child subtree. It cannot detect a never-seen agent by
//! magic — at the OS level a process is a name + argv + a place in the tree — so
//! the guarantee is "any process matching a known agent shape," not "any
//! conceivable agent." New tools are one signature line away.
//!
//! Everything degrades gracefully: a missing `ps`, a parse failure, or a
//! non-Unix target all yield an empty scan (callers then show nothing rather
//! than inventing a card).

use std::collections::{HashMap, HashSet};
use std::process::Command;

use crate::collector::session::Tool;

/// One parsed `ps` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcRecord {
    pub pid: i32,
    pub ppid: i32,
    /// Epoch ms the process started (`now_ms - etime`).
    pub started_at: i64,
    pub command: String,
}

/// A node in a process subtree.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessNode {
    pub pid: i32,
    pub command: String,
    pub children: Vec<ProcessNode>,
}

/// A signature-matched root with its assembled child subtree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessRoot {
    pub pid: i32,
    pub ppid: i32,
    pub tool: Tool,
    pub started_at: i64,
    pub tree: ProcessNode,
}

/// Scan the process table for agent roots. Empty on any failure / non-Unix.
pub fn scan_processes(now_ms: i64) -> Vec<ProcessRoot> {
    // `-axo ...=` suppresses headers; `command` is last because it has spaces.
    let output = Command::new("ps")
        .args(["-axo", "pid=,ppid=,etime=,command="])
        .output();
    match output {
        Ok(out) if !out.stdout.is_empty() => {
            let records = parse_ps(&String::from_utf8_lossy(&out.stdout), now_ms);
            build_roots(&records)
        }
        _ => Vec::new(),
    }
}

/// Parse `ps -axo pid=,ppid=,etime=,command=` output into records. Lines that
/// don't have all four fields (or whose pid/ppid don't parse) are skipped.
pub fn parse_ps(output: &str, now_ms: i64) -> Vec<ProcRecord> {
    let mut records = Vec::new();
    for line in output.lines() {
        let mut parts = line.split_whitespace();
        let (Some(pid), Some(ppid), Some(etime)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let (Ok(pid), Ok(ppid)) = (pid.parse::<i32>(), ppid.parse::<i32>()) else {
            continue;
        };
        let command = parts.collect::<Vec<_>>().join(" ");
        if command.is_empty() {
            continue;
        }
        let started_at = parse_etime(etime).map(|ms| now_ms - ms).unwrap_or(now_ms);
        records.push(ProcRecord {
            pid,
            ppid,
            started_at,
            command,
        });
    }
    records
}

/// Parse a BSD `etime` (`[[dd-]hh:]mm:ss`) into elapsed milliseconds.
pub fn parse_etime(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    let (days, rest) = match raw.split_once('-') {
        Some((d, r)) => (d.parse::<i64>().ok()?, r),
        None => (0, raw),
    };
    let mut secs: [i64; 3] = [0, 0, 0]; // h, m, s
    let parts: Vec<&str> = rest.split(':').collect();
    // Right-align: ss, then mm:ss, then hh:mm:ss.
    let offset = 3usize.checked_sub(parts.len())?;
    for (i, part) in parts.iter().enumerate() {
        secs[offset + i] = part.parse::<i64>().ok()?;
    }
    let total = days * 86_400 + secs[0] * 3_600 + secs[1] * 60 + secs[2];
    Some(total * 1_000)
}

/// Match a command line against the agent signature table.
///
/// Deliberately narrow, because the live process table is a minefield: the
/// Claude and Codex *desktop apps* ship as `/Applications/*.app` Electron trees
/// (whose framework paths even contain spaces, so a naive argv[0] split yields
/// `.../Codex`), and the real headless agents often run *through* an
/// interpreter (`python .../venv/bin/hermes`), so argv[0] is `Python`, not the
/// agent. Two rules survive that reality:
///
/// * **Hermes** — its install lives under `~/.hermes/` (daemon at
///   `.hermes/<v>/bin/hermes`, agent venv at
///   `.hermes/hermes-agent/venv/bin/hermes`). The path, not argv[0], is the
///   reliable signal and never collides with a GUI app.
/// * **Generic CLI agents** — `ollama`/`aider`/`goose` matched on argv[0]'s
///   basename only (not every token, so a stray `goose` argument can't trip it).
///
/// Claude and Codex are intentionally **absent**: the session adapters already
/// track their CLIs with real cards, so matching them here would only duplicate
/// those — and, as the desktop apps prove, invent false "untracked" ones.
pub fn match_tool(command: &str) -> Option<Tool> {
    if command.contains("/.hermes/") {
        return Some(Tool::Hermes);
    }
    let exe = command.split_whitespace().next()?;
    let base = exe.rsplit('/').next().unwrap_or(exe).to_ascii_lowercase();
    match base.as_str() {
        "ollama" | "aider" | "goose" => Some(Tool::Agent),
        _ => None,
    }
}

/// Assemble signature-matched roots with their subtrees. A match that descends
/// from another match is *not* its own root — it belongs to the ancestor's
/// subtree — so the same agent never produces two cards.
pub fn build_roots(records: &[ProcRecord]) -> Vec<ProcessRoot> {
    let by_pid: HashMap<i32, &ProcRecord> = records.iter().map(|r| (r.pid, r)).collect();
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    for r in records {
        if r.ppid != r.pid {
            children.entry(r.ppid).or_default().push(r.pid);
        }
    }

    let mut roots = Vec::new();
    for r in records {
        let Some(tool) = match_tool(&r.command) else {
            continue;
        };
        if has_matching_ancestor(r, &by_pid) {
            continue;
        }
        let tree = build_node(r.pid, &by_pid, &children, &mut HashSet::new());
        roots.push(ProcessRoot {
            pid: r.pid,
            ppid: r.ppid,
            tool,
            started_at: r.started_at,
            tree,
        });
    }
    roots.sort_by_key(|root| root.pid);
    roots
}

/// True if any ancestor in the ppid chain is itself a signature match.
fn has_matching_ancestor(rec: &ProcRecord, by_pid: &HashMap<i32, &ProcRecord>) -> bool {
    let mut seen = HashSet::new();
    let mut cur = rec.ppid;
    while cur != 0 && seen.insert(cur) {
        let Some(parent) = by_pid.get(&cur) else {
            break;
        };
        if match_tool(&parent.command).is_some() {
            return true;
        }
        cur = parent.ppid;
    }
    false
}

/// Build the subtree rooted at `pid`. `visited` guards against pid-reuse cycles.
fn build_node(
    pid: i32,
    by_pid: &HashMap<i32, &ProcRecord>,
    children: &HashMap<i32, Vec<i32>>,
    visited: &mut HashSet<i32>,
) -> ProcessNode {
    let command = by_pid
        .get(&pid)
        .map(|r| r.command.clone())
        .unwrap_or_default();
    let mut kids = Vec::new();
    if visited.insert(pid) {
        if let Some(child_pids) = children.get(&pid) {
            let mut sorted = child_pids.clone();
            sorted.sort_unstable();
            for child in sorted {
                kids.push(build_node(child, by_pid, children, visited));
            }
        }
    }
    ProcessNode {
        pid,
        command,
        children: kids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_etime_seconds_only() {
        assert_eq!(parse_etime("05"), Some(5_000));
    }

    #[test]
    fn parse_etime_minutes_seconds() {
        assert_eq!(parse_etime("01:23"), Some(83_000));
    }

    #[test]
    fn parse_etime_hours_minutes_seconds() {
        assert_eq!(parse_etime("02:03:04"), Some((2 * 3600 + 3 * 60 + 4) * 1000));
    }

    #[test]
    fn parse_etime_days() {
        assert_eq!(
            parse_etime("1-02:03:04"),
            Some((86_400 + 2 * 3600 + 3 * 60 + 4) * 1000)
        );
    }

    #[test]
    fn parse_etime_rejects_junk() {
        assert_eq!(parse_etime("not-a-time"), None);
        assert_eq!(parse_etime(""), None);
        assert_eq!(parse_etime("1:2:3:4"), None);
    }

    #[test]
    fn parse_ps_splits_fields_and_keeps_command_spaces() {
        let now = 1_000_000;
        let sample = "59421 59417    01:00 /Users/x/.hermes/abc/bin/hermes -z build the thing\n";
        let records = parse_ps(sample, now);
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.pid, 59421);
        assert_eq!(r.ppid, 59417);
        assert_eq!(r.command, "/Users/x/.hermes/abc/bin/hermes -z build the thing");
        assert_eq!(r.started_at, now - 60_000);
    }

    #[test]
    fn parse_ps_skips_unparseable_lines() {
        let records = parse_ps("header junk\n\nxx yy 01:00 cmd\n", 0);
        assert!(records.is_empty());
    }

    #[test]
    fn match_tool_matches_hermes_by_install_path() {
        // Daemon binary path.
        assert_eq!(
            match_tool("/Users/x/.hermes/v/bin/hermes -z go"),
            Some(Tool::Hermes)
        );
        // Run through the agent venv's Python interpreter: argv[0] is Python, so
        // only the path signal catches it.
        assert_eq!(
            match_tool(
                "/opt/homebrew/.../MacOS/Python /Users/x/.hermes/hermes-agent/venv/bin/hermes --continue -z prompt"
            ),
            Some(Tool::Hermes)
        );
    }

    #[test]
    fn match_tool_matches_generic_agents_by_argv0_only() {
        assert_eq!(match_tool("ollama serve"), Some(Tool::Agent));
        assert_eq!(match_tool("/usr/local/bin/aider --model x"), Some(Tool::Agent));
        // A stray agent-named *argument* must not trip the match.
        assert_eq!(match_tool("git commit -m goose"), None);
    }

    #[test]
    fn match_tool_ignores_claude_and_codex_clis() {
        // Owned by the session adapters; matching here only duplicates / misfires.
        assert_eq!(match_tool("claude --resume"), None);
        assert_eq!(match_tool("/usr/local/bin/codex"), None);
    }

    #[test]
    fn match_tool_ignores_desktop_apps() {
        // The regression that live data exposed: the Claude/Codex *desktop* apps
        // (note the space in the Codex framework path) must never be flagged.
        assert_eq!(
            match_tool("/Applications/Claude.app/Contents/MacOS/Claude"),
            None
        );
        assert_eq!(
            match_tool(
                "/Applications/Codex.app/Contents/Frameworks/Codex Framework.framework/Versions/1/Helpers/browser_crashpad_handler --monitor-self"
            ),
            None
        );
    }

    #[test]
    fn match_tool_ignores_non_agents() {
        assert_eq!(match_tool("cargo run --example snapshot"), None);
        assert_eq!(match_tool("node /app/server.js"), None);
        assert_eq!(match_tool("/Applications/MOBIUS.app/Contents/MacOS/MOBIUS"), None);
        assert_eq!(match_tool(""), None);
    }

    #[test]
    fn build_roots_assembles_subtree_under_matched_root() {
        // zsh wrapper (not a match) -> hermes (match) -> cargo + git children.
        let now = 0;
        let records = vec![
            ProcRecord { pid: 100, ppid: 1, started_at: now, command: "-zsh".into() },
            ProcRecord { pid: 200, ppid: 100, started_at: now, command: "/x/.hermes/v/bin/hermes -z go".into() },
            ProcRecord { pid: 300, ppid: 200, started_at: now, command: "cargo build".into() },
            ProcRecord { pid: 301, ppid: 200, started_at: now, command: "git status".into() },
        ];
        let roots = build_roots(&records);
        assert_eq!(roots.len(), 1);
        let root = &roots[0];
        assert_eq!(root.pid, 200);
        assert_eq!(root.tool, Tool::Hermes);
        let child_pids: Vec<i32> = root.tree.children.iter().map(|c| c.pid).collect();
        assert_eq!(child_pids, vec![300, 301]);
    }

    #[test]
    fn build_roots_excludes_matches_descending_from_another_match() {
        // A hermes subagent that is itself a hermes process must not be a second
        // root — it belongs to the parent hermes' subtree.
        let records = vec![
            ProcRecord { pid: 200, ppid: 1, started_at: 0, command: "/x/.hermes/v/bin/hermes -z go".into() },
            ProcRecord { pid: 250, ppid: 200, started_at: 0, command: "/x/.hermes/v/bin/hermes --sub".into() },
        ];
        let roots = build_roots(&records);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].pid, 200);
        assert_eq!(roots[0].tree.children.len(), 1);
        assert_eq!(roots[0].tree.children[0].pid, 250);
    }

    #[test]
    fn build_roots_returns_multiple_independent_roots_sorted() {
        let records = vec![
            ProcRecord { pid: 800, ppid: 1, started_at: 0, command: "ollama serve".into() },
            ProcRecord { pid: 200, ppid: 1, started_at: 0, command: "/x/.hermes/v/bin/hermes".into() },
        ];
        let roots = build_roots(&records);
        assert_eq!(roots.iter().map(|r| r.pid).collect::<Vec<_>>(), vec![200, 800]);
        assert_eq!(roots[0].tool, Tool::Hermes);
        assert_eq!(roots[1].tool, Tool::Agent);
    }

    #[test]
    fn build_roots_empty_when_no_matches() {
        let records = vec![ProcRecord {
            pid: 1,
            ppid: 0,
            started_at: 0,
            command: "/sbin/launchd".into(),
        }];
        assert!(build_roots(&records).is_empty());
    }
}
