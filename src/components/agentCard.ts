import type { AgentSession, Status, Tool } from "../types";
import { basename, escapeHtml, formatTokens } from "./format";

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

export function renderCard(session: AgentSession): string {
  const totalTokens =
    session.tokens.input + session.tokens.output + session.tokens.cache;
  const action = session.currentAction ?? "standing by";
  const branch = session.branch ?? "branch unknown";
  const model = session.model ?? "model unknown";
  const name = session.title ?? basename(session.projectPath);

  return `
    <article class="agent-card" data-tool="${session.tool}">
      <div class="agent-card__topline">
        <span class="tool-badge">${escapeHtml(toolLabels[session.tool])}</span>
        <span class="status-pill status-pill--${session.status}">
          <span aria-hidden="true"></span>${escapeHtml(statusLabels[session.status])}
        </span>
      </div>
      <div>
        <h2>${escapeHtml(name)}</h2>
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
      </dl>
      <p class="agent-action">${escapeHtml(action)}</p>
    </article>
  `;
}
