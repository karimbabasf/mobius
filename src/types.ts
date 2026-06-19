export type Tool = "claude" | "codex" | "cursor";

export type Status =
  | "starting"
  | "working"
  | "idle"
  | "waitingInput"
  | "ended"
  | "dead";

export type FileAction =
  | "reading"
  | "writing"
  | "editing"
  | "appending"
  | "running"
  | "searching";

export interface Tokens {
  input: number;
  output: number;
  cache: number;
}

export interface FileEvent {
  path: string;
  action: FileAction;
  at: number;
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
  title: string | null;
  recentFiles: FileEvent[];
}
