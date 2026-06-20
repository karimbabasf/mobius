import type { AgentSession, Status, Tool } from "../types";
import { basename, escapeHtml, formatTokens } from "./format";
import { renderContextBar, renderContextDetail } from "./contextGauge";
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
  waitingInput: "waiting",
  ended: "ended",
  dead: "dead",
};

/**
 * A collapsible agent "flight strip". The header is always visible and reads
 * like a process line (tool · name · status · current action · tokens); the
 * body drops down on click to reveal identity, the token split, and the
 * scrollable activity log. Open/closed state is driven entirely by
 * `aria-expanded`, so the CSS sibling selector handles the reveal and the
 * caller only has to remember which ids are open.
 */
export function renderCard(session: AgentSession, expanded: boolean): string {
  const name = session.title ?? basename(session.projectPath);
  const project = basename(session.projectPath);
  const branch = session.branch ?? "—";
  const model = session.model ?? "unknown model";
  const action = session.currentAction ?? "standing by";
  const live = session.status === "working";
  const pid = session.pid != null ? `pid ${session.pid}` : "pid —";

  return `
    <article class="strip" data-tool="${session.tool}" data-status="${session.status}">
      <button
        class="strip__head"
        type="button"
        aria-expanded="${expanded ? "true" : "false"}"
        data-session-toggle="${escapeHtml(session.id)}"
      >
        <span class="strip__caret" aria-hidden="true">▸</span>
        <span class="strip__tool">${renderToolLogo(session.tool)}</span>
        <span class="strip__name">${escapeHtml(name)}</span>
        <span class="strip__status"><i class="strip__dot" aria-hidden="true"></i>${escapeHtml(
          statusLabels[session.status],
        )}</span>
        <span class="strip__now">${escapeHtml(action)}</span>
        <span class="strip__tokens">${renderContextBar(session)}</span>
      </button>
      <div class="strip__body">
        <div class="strip__body-inner">
          <p class="strip__meta">
            <span class="strip__meta-tool">${escapeHtml(toolLabels[session.tool])}</span>
            <span>${escapeHtml(project)}</span>
            <span>${escapeHtml(branch)}</span>
            <span>${escapeHtml(model)}</span>
            <span>${escapeHtml(pid)}</span>
          </p>
          <span class="strip__id">${escapeHtml(session.id)}</span>
          <dl class="token-row">
            <div><dt>in</dt><dd>${formatTokens(session.tokens.input)}</dd></div>
            <div><dt>out</dt><dd>${formatTokens(session.tokens.output)}</dd></div>
            <div><dt>cache</dt><dd>${formatTokens(session.tokens.cache)}</dd></div>
          </dl>
          ${renderContextDetail(session.context)}
          <div class="log">
            <div class="log__head">
              <span>activity log</span>
              <span class="log__count">${session.recentFiles.length}</span>
            </div>
            ${renderFileLog(session.recentFiles, live)}
          </div>
        </div>
      </div>
    </article>
  `;
}
