pub mod app_paths;
pub mod collector;

use collector::registry::Registry;
use collector::session::{AgentSession, Status, Tokens, Tool};

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

#[tauri::command]
fn get_sessions(registry: tauri::State<'_, Registry>) -> Vec<AgentSession> {
    registry.get_all()
}

fn seeded_sessions() -> Vec<AgentSession> {
    vec![AgentSession {
        id: "demo-claude-session".into(),
        tool: Tool::Claude,
        pid: Some(std::process::id() as i32),
        project_path: "/Users/you/project".into(),
        branch: Some("main".into()),
        model: Some("claude-opus-4-8".into()),
        status: Status::Working,
        current_action: Some("running Bash".into()),
        started_at: 1_820_000_000_000,
        last_event_at: 1_820_000_005_000,
        tokens: Tokens {
            input: 1200,
            output: 320,
            cache: 640,
        },
        title: Some("demo session".into()),
        recent_files: vec![],
    }]
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Registry::with_seed(seeded_sessions()))
        .invoke_handler(tauri::generate_handler![greet, get_sessions])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
