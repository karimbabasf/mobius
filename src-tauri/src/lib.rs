pub mod app_paths;
pub mod collector;

use collector::session::AgentSession;
use collector::Collector;

/// Current wall-clock time in epoch milliseconds.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[tauri::command]
fn get_sessions(collector: tauri::State<'_, Collector>) -> Vec<AgentSession> {
    collector.snapshot(now_ms())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Collector::new())
        .invoke_handler(tauri::generate_handler![get_sessions])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
