export type Tool = "claude" | "codex" | "cursor";

export type Status =
  | "starting"
  | "working"
  | "idle"
  | "waitingInput"
  | "ended"
  | "dead";

export interface Tokens {
  input: number;
  output: number;
  cache: number;
}

export interface AgentSession {
  id: string;
  tool: Tool;
  pid: number | null;
  projectPath: string;
  branch: string | null;
  model: string | null;
  status: Status;
  currentAction: string | null;
  startedAt: number;
  lastEventAt: number;
  tokens: Tokens;
  costUsd: number | null;
}
