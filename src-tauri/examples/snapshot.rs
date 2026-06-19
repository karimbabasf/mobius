//! Dev helper: print the exact live snapshot the app's `get_sessions` returns.
//! Run with: cargo run --example snapshot

use agent_mission_control_lib::collector::Collector;

fn main() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let sessions = Collector::new().snapshot(now);
    println!("ACTIVE SESSIONS: {}", sessions.len());
    for s in &sessions {
        println!(
            "- [{:?}] tool={:?} title={:?}\n    id={} model={:?} branch={:?}\n    tokens in/out/cache={}/{}/{} files={} action={:?}",
            s.status,
            s.tool,
            s.title,
            s.id,
            s.model,
            s.branch,
            s.tokens.input,
            s.tokens.output,
            s.tokens.cache,
            s.recent_files.len(),
            s.current_action,
        );
        for f in s.recent_files.iter().take(4) {
            println!("      {:?}  {}", f.action, f.path);
        }
    }
}
