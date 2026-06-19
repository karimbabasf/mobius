import type { AgentSession, Status, Tool } from "../types";
import { basename, escapeHtml, formatTokens } from "./format";
import { renderFileLog } from "./fileLog";
import { renderToolLogo } from "./toolLogo";

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
  const name = session.title ?? basename(session.projectPath);
  const project = basename(session.projectPath);
  const branch = session.branch ?? "—";
  const model = session.model ?? "model unknown";
  const action = session.currentAction ?? "standing by";

  return `
    <article class="agent-card" data-tool="${session.tool}">
      <div class="agent-card__topline">
        <span class="tool-badge">${renderToolLogo(session.tool)}<span>${escapeHtml(
          toolLabels[session.tool],
        )}</span></span>
        <span class="status-pill status-pill--${session.status}">
          <span aria-hidden="true"></span>${escapeHtml(statusLabels[session.status])}
        </span>
      </div>
      <div class="agent-id-block">
        <h2>${escapeHtml(name)}</h2>
        <span class="agent-id">id ${escapeHtml(session.id)}</span>
        <p class="agent-meta">${escapeHtml(project)} · ${escapeHtml(branch)} · ${escapeHtml(model)}</p>
      </div>
      <dl class="token-row">
        <div><dt>In</dt><dd>${formatTokens(session.tokens.input)}</dd></div>
        <div><dt>Out</dt><dd>${formatTokens(session.tokens.output)}</dd></div>
        <div><dt>Cache</dt><dd>${formatTokens(session.tokens.cache)}</dd></div>
      </dl>
      <div class="file-log-block">
        <span class="file-log__header">file activity</span>
        ${renderFileLog(session.recentFiles)}
      </div>
      <p class="agent-action">${escapeHtml(action)}</p>
    </article>
  `;
}
