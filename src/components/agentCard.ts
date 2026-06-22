import type { AgentSession, Status, Tool } from "../types";
import { basename, escapeHtml, formatTokens } from "./format";
import { renderContextBar, renderContextDetail } from "./contextGauge";
import { renderFileLog } from "./fileLog";
import { renderToolLogo } from "./toolLogo";

const toolLabels: Record<Tool, string> = {
  claude: "Claude",
  codex: "Codex",
  cursor: "Cursor",
  hermes: "Hermes",
};

const providerBadges: Record<Tool, string> = {
  claude: "CLAUDE",
  codex: "CODEX",
  cursor: "CURSOR",
  hermes: "HERMES",
};

const statusLabels: Record<Status, string> = {
  starting: "starting",
  working: "working",
  idle: "idle",
  waitingInput: "waiting",
  ended: "ended",
  dead: "dead",
};

function renderRenameControl(session: AgentSession, name: string): string {
  if (!session.canRename) {
    return `
      <div class="rename-box rename-box--readonly">
        <span class="rename-box__label">name</span>
        <span>Name is read-only for this session</span>
      </div>
    `;
  }

  return `
    <form class="rename-box" data-session-rename="${escapeHtml(session.id)}">
      <label class="rename-box__label" for="rename-${escapeHtml(session.id)}">name</label>
      <input
        id="rename-${escapeHtml(session.id)}"
        class="rename-box__input"
        name="newTitle"
        value="${escapeHtml(name)}"
        autocomplete="off"
      />
      <button class="rename-box__button" type="submit">Rename actual chat</button>
    </form>
  `;
}

/**
 * A compact operations block. The provider lane comes first so mixed Claude,
 * Codex, and Cursor sessions can be scanned before reading session details.
 */
export function renderCard(session: AgentSession, expanded: boolean): string {
  const name = session.title ?? basename(session.projectPath);
  const project = basename(session.projectPath);
  const branch = session.branch ?? "—";
  const model = session.model ?? "unknown model";
  const action = session.currentAction ?? "standing by";
  const live = session.status === "working";
  const pid = session.pid != null ? `pid ${session.pid}` : "pid —";
  const expandedClass = expanded ? " agent-block--expanded" : "";
  const titleSource = session.titleSource === "provider" ? "real name" : "fallback";

  return `
    <article class="agent-block${expandedClass}" data-tool="${session.tool}" data-provider="${session.tool}" data-status="${session.status}">
      <button
        class="agent-block__head"
        type="button"
        aria-expanded="${expanded ? "true" : "false"}"
        data-session-toggle="${escapeHtml(session.id)}"
      >
        <span class="agent-block__provider" data-provider="${session.tool}" aria-label="${escapeHtml(
          toolLabels[session.tool],
        )} provider">
          <span class="agent-block__provider-mark">${renderToolLogo(session.tool)}</span>
          <span class="agent-block__provider-badge">${escapeHtml(providerBadges[session.tool])}</span>
        </span>
        <span class="agent-block__identity">
          <span class="agent-block__tool"><span>${escapeHtml(
            toolLabels[session.tool],
          )}</span><i>${escapeHtml(titleSource)}</i></span>
          <span class="agent-block__name">${escapeHtml(name)}</span>
          ${
            session.canRename
              ? `<span class="agent-block__rename-hint"><i aria-hidden="true">✎</i>click to rename</span>`
              : ""
          }
        </span>
        <span class="agent-block__status" aria-label="status ${escapeHtml(
          statusLabels[session.status],
        )}"><i class="agent-block__dot" aria-hidden="true"></i><span>${escapeHtml(
          statusLabels[session.status],
        )}</span></span>
        <span class="agent-block__action"><b>now</b>${escapeHtml(action)}</span>
        <span class="agent-block__tokens"><b>context</b>${renderContextBar(session)}</span>
        <span class="agent-block__meta">
          <span><b>project</b>${escapeHtml(project)}</span>
          <span><b>started</b><span class="agent-block__uptime" data-at="${session.startedAt}"></span></span>
          <span><b>model</b>${escapeHtml(model)}</span>
        </span>
      </button>
      <div class="agent-block__body">
        <div class="agent-block__body-inner">
          <div class="agent-block__expanded-grid">
            <section class="agent-block__panel agent-block__panel--session">
              <h3>session</h3>
              ${renderRenameControl(session, name)}
              <div class="agent-block__details">
                <span><b>project</b>${escapeHtml(project)}</span>
                <span><b>branch</b>${escapeHtml(branch)}</span>
                <span><b>model</b>${escapeHtml(model)}</span>
                <span><b>process</b>${escapeHtml(pid)}</span>
                <span><b>title</b>${escapeHtml(session.titleSource)}</span>
              </div>
              <span class="agent-block__id">${escapeHtml(session.id)}</span>
            </section>
            <section class="agent-block__panel agent-block__panel--capacity">
              <h3>capacity</h3>
              <dl class="token-row">
                <div><dt>in</dt><dd>${formatTokens(session.tokens.input)}</dd></div>
                <div><dt>out</dt><dd>${formatTokens(session.tokens.output)}</dd></div>
                <div><dt>cache</dt><dd>${formatTokens(session.tokens.cache)}</dd></div>
              </dl>
              ${renderContextDetail(session.context)}
            </section>
            <section class="agent-block__panel agent-block__panel--activity">
              <h3>activity</h3>
              <div class="log">
                <div class="log__head">
                  <span>recent files</span>
                  <span class="log__count">${session.recentFiles.length}</span>
                </div>
                ${renderFileLog(session.recentFiles, live)}
              </div>
            </section>
          </div>
        </div>
      </div>
    </article>
  `;
}
