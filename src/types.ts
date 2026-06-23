export type Tool = "claude" | "codex" | "cursor" | "hermes" | "agent";

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

/**
 * Run-level telemetry for autonomous agents (Hermes/Fugu one-shot builds).
 * Present only on Hermes sessions; `null`/absent for Claude/Codex.
 *
 * Note: Fugu/Sakana does not report a dollar figure to `state.db`, so
 * `costUsd` is usually `null` — token burn (see `tokens`) is the real cost
 * signal, paired with `turns`/`maxTurns`.
 */
export interface RunStats {
  turns: number;
  maxTurns: number | null;
  toolCalls: number;
  messages: number;
  effort: string | null;
  costUsd: number | null;
  costStatus: string | null;
  endReason: string | null;
}

/**
 * A node in a live process subtree, attached to a card by the OS process
 * scanner. Mirrors the Rust `ProcessNode`.
 */
export interface ProcessNode {
  pid: number;
  command: string;
  children: ProcessNode[];
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
  titleSource: "provider" | "fallback";
  canRename: boolean;
  recentFiles: FileEvent[];
  parentSessionId?: string | null;
  connectionRole?: "orchestrator" | "subAgent" | null;
  childCount?: number;
  run?: RunStats | null;
  /** Live process subtree, present when the scanner matched this agent. */
  processTree?: ProcessNode | null;
  /** True for a synthesized card: an agent process with no session of its own. */
  untracked?: boolean;
}
