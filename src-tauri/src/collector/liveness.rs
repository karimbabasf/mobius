//! Authoritative process-liveness signals for local agents.
//!
//! Status used to be purely time-based: a session was "idle" until its log had
//! been silent for 10 minutes, then it vanished. Closing a terminal therefore
//! left a ghost card for up to 10 minutes. This module replaces that guess with
//! a real answer to "is the process still alive?", using signals each tool
//! already exposes on disk / to the OS:
//!
//! * **Claude Code** writes `~/.claude/sessions/<PID>.json` for every running
//!   instance. The filename is the OS PID and the body carries the `sessionId`
//!   (which is also the transcript filename), so we get a direct
//!   PID <-> sessionId bridge plus a real `status` field. A `kill(pid, 0)`
//!   confirms the process is alive. Matching on `sessionId` (not just PID)
//!   makes PID reuse a non-issue: a recycled PID's registry file would have to
//!   carry the exact same sessionId, which only the real owner writes.
//! * **Codex** keeps its rollout `.jsonl` open with a write fd for the whole
//!   session, so `lsof -c codex` maps the open transcript file -> owning PID.
//!
//! Everything here degrades gracefully: missing files, an absent `lsof`, or a
//! non-Unix target all yield empty maps (callers then show nothing rather than
//! a ghost).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

/// A live Claude session, keyed by `sessionId` in the returned map.
#[derive(Clone, Debug)]
pub struct LiveClaude {
    pub pid: i32,
    pub status: Option<ClaudeStatus>,
    pub entrypoint: Option<String>,
}

/// The real status Claude Code reports in its session registry file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaudeStatus {
    Idle,
    Busy,
}

#[derive(Debug, Deserialize)]
struct RegistryFile {
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    status: Option<String>,
    entrypoint: Option<String>,
}

/// Live Claude sessions, keyed by `sessionId`, for every registry file whose
/// PID is still alive.
pub fn live_claude_sessions() -> HashMap<String, LiveClaude> {
    let dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".claude")
        .join("sessions");
    let mut map = HashMap::new();
    for (pid, reg) in read_registry_dir(&dir) {
        if !pid_is_alive(pid) {
            continue;
        }
        let Some(session_id) = reg.session_id else {
            continue;
        };
        map.insert(
            session_id,
            LiveClaude {
                pid,
                status: reg.status.as_deref().and_then(parse_claude_status),
                entrypoint: reg.entrypoint,
            },
        );
    }
    map
}

/// Open Codex transcript files mapped to their owning PID, via `lsof`.
pub fn live_codex_pids_by_file() -> HashMap<PathBuf, i32> {
    // `-c codex` filters at the kernel level (~40x faster than scanning every
    // open file); `-F pn` emits machine-readable `p<pid>` / `n<name>` records.
    let output = Command::new("lsof")
        .args(["-c", "codex", "-F", "pn"])
        .output();
    match output {
        // lsof exits non-zero when some processes can't be inspected, even
        // though the rows we care about are present — trust stdout if non-empty.
        Ok(out) if !out.stdout.is_empty() => parse_lsof_pn(&String::from_utf8_lossy(&out.stdout)),
        _ => HashMap::new(),
    }
}

/// PID of the running Hermes daemon, if any.
///
/// Hermes runs a single background process whose argv contains
/// `.hermes/<...>/bin/hermes`. There is no per-session process (every session
/// lives in one SQLite DB), so this single PID gates all Hermes cards. Uses
/// `pgrep -f`; yields `None` where `pgrep` is missing, nothing matches, or on
/// non-Unix targets.
pub fn hermes_daemon_pid() -> Option<i32> {
    let output = Command::new("pgrep")
        .args(["-f", r"\.hermes/.*/bin/hermes"])
        .output()
        .ok()?;
    if !output.status.success() {
        // pgrep exits non-zero when nothing matched.
        return None;
    }
    first_pid(&String::from_utf8_lossy(&output.stdout))
}

/// First PID from `pgrep` output (one PID per line).
fn first_pid(output: &str) -> Option<i32> {
    output
        .lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
        .next()
}

/// Parse `lsof -F pn` output into open-`.jsonl` -> PID.
fn parse_lsof_pn(output: &str) -> HashMap<PathBuf, i32> {
    let mut map = HashMap::new();
    let mut current: Option<i32> = None;
    for line in output.lines() {
        let mut chars = line.chars();
        let Some(tag) = chars.next() else {
            continue;
        };
        let rest = chars.as_str();
        match tag {
            'p' => current = rest.trim().parse::<i32>().ok(),
            'n' => {
                if let Some(pid) = current {
                    if rest.ends_with(".jsonl") {
                        map.insert(PathBuf::from(rest), pid);
                    }
                }
            }
            _ => {}
        }
    }
    map
}

/// Read `<PID>.json` registry files from `dir` (no liveness check).
fn read_registry_dir(dir: &Path) -> Vec<(i32, RegistryFile)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(pid) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<i32>().ok())
        else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(reg) = serde_json::from_str::<RegistryFile>(&text) {
            out.push((pid, reg));
        }
    }
    out
}

fn parse_claude_status(raw: &str) -> Option<ClaudeStatus> {
    match raw.to_ascii_lowercase().as_str() {
        "busy" | "running" | "working" => Some(ClaudeStatus::Busy),
        "idle" | "waiting" => Some(ClaudeStatus::Idle),
        _ => None,
    }
}

/// `kill(pid, 0)`: no signal sent, just an existence + permission probe.
/// `EPERM` means the process exists but isn't ours — still alive.
#[cfg(unix)]
fn pid_is_alive(pid: i32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    match kill(Pid::from_raw(pid), None) {
        Ok(()) => true,
        Err(Errno::EPERM) => true,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn pid_is_alive(_pid: i32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lsof_keeps_only_jsonl_and_maps_to_pid() {
        // Two codex processes; only the rollout .jsonl files should be kept,
        // each attributed to the PID of the `p` record that preceded it.
        let sample = "\
p75581
fcwd
n/Users/karimbaba
f90
n/Users/karimbaba/.codex/sessions/2026/06/19/rollout-a.jsonl
f131
n/Users/karimbaba/.codex-karim/sessions/2026/06/19/rollout-b.jsonl
f12
n/Users/karimbaba/.codex/logs_2.sqlite-shm
p80001
f7
n/Users/karimbaba/.codex/sessions/2026/06/19/rollout-c.jsonl
";
        let map = parse_lsof_pn(sample);
        assert_eq!(map.len(), 3);
        assert_eq!(
            map.get(Path::new(
                "/Users/karimbaba/.codex/sessions/2026/06/19/rollout-a.jsonl"
            )),
            Some(&75581)
        );
        assert_eq!(
            map.get(Path::new(
                "/Users/karimbaba/.codex-karim/sessions/2026/06/19/rollout-b.jsonl"
            )),
            Some(&75581)
        );
        assert_eq!(
            map.get(Path::new(
                "/Users/karimbaba/.codex/sessions/2026/06/19/rollout-c.jsonl"
            )),
            Some(&80001)
        );
    }

    #[test]
    fn parse_lsof_ignores_non_jsonl_and_orphan_names() {
        let sample = "n/orphan/before/any/pid.jsonl\np100\nn/tmp/notes.txt\n";
        assert!(parse_lsof_pn(sample).is_empty());
    }

    #[test]
    fn read_registry_dir_parses_pid_from_filename_and_body() {
        let dir = registry_fixture_dir();
        let entries = read_registry_dir(&dir);
        let busy = entries
            .iter()
            .find(|(pid, _)| *pid == 4242)
            .expect("4242.json should parse");
        assert_eq!(busy.1.session_id.as_deref(), Some("sess-busy"));
        assert_eq!(busy.1.status.as_deref(), Some("busy"));
        assert_eq!(busy.1.entrypoint.as_deref(), Some("cli"));
    }

    #[test]
    fn first_pid_picks_first_line_and_ignores_blanks() {
        assert_eq!(first_pid("83766\n83770\n"), Some(83766));
        assert_eq!(first_pid("\n  91234 \n"), Some(91234));
        assert_eq!(first_pid(""), None);
        assert_eq!(first_pid("not-a-pid\n"), None);
    }

    #[test]
    fn claude_status_mapping() {
        assert_eq!(parse_claude_status("busy"), Some(ClaudeStatus::Busy));
        assert_eq!(parse_claude_status("idle"), Some(ClaudeStatus::Idle));
        assert_eq!(parse_claude_status("whatever"), None);
    }

    fn registry_fixture_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/claude-sessions")
    }
}
