use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Claude,
    Codex,
    Cursor,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Starting,
    Working,
    Idle,
    WaitingInput,
    Ended,
    Dead,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSession {
    pub id: String,
    pub tool: Tool,
    pub pid: Option<i32>,
    pub project_path: String,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub status: Status,
    pub current_action: Option<String>,
    pub started_at: i64,
    pub last_event_at: i64,
    pub tokens: Tokens,
    pub cost_usd: Option<f64>,
}

impl AgentSession {
    pub fn touch(&mut self, now_ms: i64) {
        self.last_event_at = now_ms;
    }

    pub fn mark_idle_if_stale(&mut self, now_ms: i64, idle_after_ms: i64) {
        if matches!(self.status, Status::Working) && now_ms - self.last_event_at >= idle_after_ms {
            self.status = Status::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AgentSession {
        AgentSession {
            id: "s1".into(),
            tool: Tool::Claude,
            pid: Some(123),
            project_path: "/Users/you/project".into(),
            branch: Some("main".into()),
            model: Some("claude-opus-4-8".into()),
            status: Status::Working,
            current_action: Some("running Bash".into()),
            started_at: 0,
            last_event_at: 1000,
            tokens: Tokens::default(),
            cost_usd: Some(0.0),
        }
    }

    #[test]
    fn working_session_becomes_idle_when_stale() {
        let mut session = sample();
        session.mark_idle_if_stale(6000, 4000);
        assert!(matches!(session.status, Status::Idle));
    }

    #[test]
    fn working_session_stays_working_when_recent() {
        let mut session = sample();
        session.mark_idle_if_stale(2000, 4000);
        assert!(matches!(session.status, Status::Working));
    }

    #[test]
    fn touch_updates_last_event_timestamp_without_changing_status() {
        let mut session = sample();
        session.touch(2500);
        assert_eq!(session.last_event_at, 2500);
        assert!(matches!(session.status, Status::Working));
    }

    #[test]
    fn serializes_to_camel_case_for_ui() {
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(json.contains("\"projectPath\""));
        assert!(json.contains("\"currentAction\""));
        assert!(json.contains("\"lastEventAt\""));
        assert!(json.contains("\"costUsd\""));
        assert!(json.contains("\"tool\":\"claude\""));
        assert!(json.contains("\"status\":\"working\""));
    }
}
