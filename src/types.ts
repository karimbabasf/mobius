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

export type LimitSource = "reported" | "modelTable" | "cacheFile" | "unknown";

export type ContextCategory =
  | "systemInstructions"
  | "toolDefinitions"
  | "memory"
  | "fileReads"
  | "conversation"
  | "other";

export interface CategorySlice {
  name: ContextCategory;
  tokens: number;
  estimated: boolean;
}

export interface Compaction {
  at: number;
  preTokens: number | null;
  postTokens: number | null;
  explicit: boolean;
}

export interface ContextSnapshot {
  at: number;
  used: number;
}

export interface ContextWindow {
  used: number;
  limit: number | null;
  fillPct: number | null;
  limitSource: LimitSource;
  cached: number;
  fresh: number;
  categories: CategorySlice[];
  residual: number;
  history: ContextSnapshot[];
  compactions: Compaction[];
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
  context: ContextWindow | null;
  title: string | null;
  recentFiles: FileEvent[];
}
