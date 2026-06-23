pub mod app_paths;
pub mod collector;

use collector::session::AgentSession;
use collector::Collector;
use serde::Serialize;

/// Current wall-clock time in epoch milliseconds.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Outcome of attempting to signal one process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KillResult {
    pub pid: i32,
    /// True when the signal was delivered (process existed and we had
    /// permission). False for an already-dead or reused PID — a reported no-op,
    /// never an error.
    pub ok: bool,
}

/// Signal each PID via `killer`, collecting per-PID outcomes. Pure over the
/// killer so the command can be tested without signalling real processes.
fn signal_pids<F: FnMut(i32) -> bool>(pids: &[i32], mut killer: F) -> Vec<KillResult> {
    pids.iter()
        .map(|&pid| KillResult {
            pid,
            ok: killer(pid),
        })
        .collect()
}

/// Send `SIGTERM` to one PID. `false` on any failure (dead PID, no permission,
/// non-Unix) rather than surfacing an error.
#[cfg(unix)]
fn send_sigterm(pid: i32) -> bool {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    kill(Pid::from_raw(pid), Signal::SIGTERM).is_ok()
}

#[cfg(not(unix))]
fn send_sigterm(_pid: i32) -> bool {
    false
}

#[tauri::command]
fn get_sessions(collector: tauri::State<'_, Collector>) -> Vec<AgentSession> {
    collector.snapshot(now_ms())
}

#[tauri::command]
fn rename_session(
    collector: tauri::State<'_, Collector>,
    session_id: String,
    new_title: String,
) -> Result<(), String> {
    collector.rename_session(&session_id, &new_title)
}

/// Send SIGTERM to each PID (a process root and, optionally, its subtree). The
/// frontend computes the PID list from the tree it rendered and confirms with
/// the user first.
#[tauri::command]
fn kill_processes(pids: Vec<i32>) -> Vec<KillResult> {
    signal_pids(&pids, send_sigterm)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Collector::new())
        .invoke_handler(tauri::generate_handler![
            get_sessions,
            rename_session,
            kill_processes
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_pids_calls_killer_once_per_pid_and_collects_results() {
        let mut seen = Vec::new();
        let results = signal_pids(&[10, 20, 30], |pid| {
            seen.push(pid);
            pid != 20 // pretend 20 was already dead
        });
        assert_eq!(
            seen,
            vec![10, 20, 30],
            "killer invoked once per pid, in order"
        );
        assert_eq!(
            results,
            vec![
                KillResult { pid: 10, ok: true },
                KillResult { pid: 20, ok: false },
                KillResult { pid: 30, ok: true },
            ]
        );
    }

    #[test]
    fn signal_pids_empty_is_noop() {
        let results = signal_pids(&[], |_| true);
        assert!(results.is_empty());
    }
}
