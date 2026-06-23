//! Dev helper: print the exact live snapshot the app's `get_sessions` returns.
//! Run with: cargo run --example snapshot

use mobius_lib::collector::scanner::ProcessNode;
use mobius_lib::collector::Collector;

/// Print a process subtree as an indented list.
fn print_tree(node: &ProcessNode, depth: usize) {
    println!("      {}{} {}", "  ".repeat(depth), node.pid, node.command);
    for child in &node.children {
        print_tree(child, depth + 1);
    }
}

fn main() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let sessions = Collector::new().snapshot(now);
    println!("ACTIVE SESSIONS: {}", sessions.len());
    for s in &sessions {
        println!(
            "- [{:?}] tool={:?} pid={:?} title={:?}\n    id={} model={:?} branch={:?}\n    tokens in/out/cache={}/{}/{} files={} action={:?}",
            s.status,
            s.tool,
            s.pid,
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
        if let Some(ctx) = &s.context {
            let pct = ctx
                .fill_pct
                .map(|p| format!("{p:.1}%"))
                .unwrap_or_else(|| "—".into());
            let limit = ctx
                .limit
                .map(|l| l.to_string())
                .unwrap_or_else(|| "?".into());
            println!(
                "    context: {}/{} ({}) cached={} fresh={} history={} compactions={}",
                ctx.used,
                limit,
                pct,
                ctx.cached,
                ctx.fresh,
                ctx.history.len(),
                ctx.compactions.len(),
            );
            let cats: Vec<String> = ctx
                .categories
                .iter()
                .map(|c| {
                    format!(
                        "{:?}={}{}",
                        c.name,
                        c.tokens,
                        if c.estimated { "~" } else { "" }
                    )
                })
                .collect();
            if !cats.is_empty() {
                println!("      breakdown: {}", cats.join(" "));
            }
        }
        if let Some(run) = &s.run {
            let cost = run
                .cost_usd
                .map(|c| format!("${c:.4}"))
                .unwrap_or_else(|| "unreported".into());
            let cap = run
                .max_turns
                .map(|m| m.to_string())
                .unwrap_or_else(|| "?".into());
            println!(
                "    run: turns={}/{} tools={} msgs={} effort={:?} cost={} end={:?}",
                run.turns, cap, run.tool_calls, run.messages, run.effort, cost, run.end_reason,
            );
        }
        for f in s.recent_files.iter().take(4) {
            println!("      {:?}  {}", f.action, f.path);
        }
        if s.untracked {
            println!("    UNTRACKED (no session card of its own)");
        }
        if let Some(tree) = &s.process_tree {
            println!("    process tree:");
            print_tree(tree, 0);
        }
    }
}
