import type { AgentSession, Status, Tool } from "../types";

const toolLabels: Record<Tool, string> = {
  claude: "Claude Code",
  codex: "Codex",
  cursor: "Cursor",
};

const statusLabels: Record<Status, string> = {
  starting: "starting",
  working: "working",
  idle: "idle",
  waitingInput: "waiting input",
  ended: "ended",
  dead: "dead",
};

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function basename(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  return trimmed.split("/").pop() || trimmed || "unknown project";
}

function formatTokens(total: number): string {
  if (total >= 1000) {
    return `${(total / 1000).toFixed(1)}k`;
  }
  return String(total);
}

function formatCost(value: number | null): string {
  return value === null ? "n/a (subscription)" : `$${value.toFixed(2)}`;
}

export function renderCard(session: AgentSession): string {
  const totalTokens =
    session.tokens.input + session.tokens.output + session.tokens.cache;
  const action = session.currentAction ?? "standing by";
  const branch = session.branch ?? "branch unknown";
  const model = session.model ?? "model unknown";

  return `
    <article class="agent-card" data-tool="${session.tool}">
      <div class="agent-card__topline">
        <span class="tool-badge">${escapeHtml(toolLabels[session.tool])}</span>
        <span class="status-pill status-pill--${session.status}">
          <span aria-hidden="true"></span>${escapeHtml(statusLabels[session.status])}
        </span>
      </div>
      <div>
        <h2>${escapeHtml(basename(session.projectPath))}</h2>
        <p class="agent-path">${escapeHtml(session.projectPath)}</p>
      </div>
      <dl class="agent-facts">
        <div>
          <dt>Branch</dt>
          <dd>${escapeHtml(branch)}</dd>
        </div>
        <div>
          <dt>Model</dt>
          <dd>${escapeHtml(model)}</dd>
        </div>
        <div>
          <dt>Tokens</dt>
          <dd>${formatTokens(totalTokens)}</dd>
        </div>
        <div>
          <dt>Cost</dt>
          <dd>${escapeHtml(formatCost(session.costUsd))}</dd>
        </div>
      </dl>
      <p class="agent-action">${escapeHtml(action)}</p>
    </article>
  `;
}
