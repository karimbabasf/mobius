use std::collections::HashMap;
use std::sync::Mutex;

use crate::collector::session::AgentSession;

pub struct Registry {
    sessions: Mutex<HashMap<String, AgentSession>>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    pub fn with_seed(seed: Vec<AgentSession>) -> Self {
        let registry = Self::new();
        for session in seed {
            registry.upsert(session);
        }
        registry
    }

    pub fn upsert(&self, session: AgentSession) {
        self.sessions
            .lock()
            .expect("registry lock should not be poisoned")
            .insert(session.id.clone(), session);
    }

    pub fn get_all(&self) -> Vec<AgentSession> {
        let mut sessions: Vec<AgentSession> = self
            .sessions
            .lock()
            .expect("registry lock should not be poisoned")
            .values()
            .cloned()
            .collect();
        sessions.sort_by(|a, b| a.id.cmp(&b.id));
        sessions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::session::{AgentSession, Status, TitleSource, Tokens, Tool};

    fn sample(id: &str) -> AgentSession {
        AgentSession {
            id: id.into(),
            tool: Tool::Claude,
            pid: Some(1234),
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
            context: None,
            title: Some("demo".into()),
            title_source: TitleSource::Provider,
            can_rename: false,
            recent_files: vec![],
            parent_session_id: None,
            connection_role: None,
            child_count: 0,
            run: None,
            process_tree: None,
            untracked: false,
        }
    }

    #[test]
    fn upsert_replaces_duplicate_ids() {
        let registry = Registry::new();
        registry.upsert(sample("demo"));

        let mut updated = sample("demo");
        updated.status = Status::Idle;
        updated.current_action = Some("waiting for next turn".into());
        registry.upsert(updated);

        let sessions = registry.get_all();
        assert_eq!(sessions.len(), 1);
        assert!(matches!(sessions[0].status, Status::Idle));
        assert_eq!(
            sessions[0].current_action.as_deref(),
            Some("waiting for next turn")
        );
    }

    #[test]
    fn with_seed_returns_seeded_sessions() {
        let registry = Registry::with_seed(vec![sample("one"), sample("two")]);

        let sessions = registry.get_all();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().any(|session| session.id == "one"));
        assert!(sessions.iter().any(|session| session.id == "two"));
    }

    #[test]
    fn seeded_sessions_serialize_for_the_ui() {
        let registry = Registry::with_seed(vec![sample("demo")]);
        let json = serde_json::to_string(&registry.get_all()).unwrap();

        assert!(json.contains("\"projectPath\""));
        assert!(json.contains("\"currentAction\""));
        assert!(json.contains("\"status\":\"working\""));
    }
}
