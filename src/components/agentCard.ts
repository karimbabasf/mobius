import type { AgentSession, RunStats, Status, Tool } from "../types";
import { basename, escapeHtml, formatBurnRate, formatTokens } from "./format";
import { renderContextBar, renderContextDetail } from "./contextGauge";
import { renderFileLog } from "./fileLog";
import { renderToolLogo } from "./toolLogo";
import { flattenPids, renderProcessTree } from "./processTree";

const toolLabels: Record<Tool, string> = {
  claude: "Claude",
  codex: "Codex",
  cursor: "Cursor",
  hermes: "Hermes",
  agent: "Agent",
};

const providerBadges: Record<Tool, string> = {
  claude: "CLAUDE",
  codex: "CODEX",
  cursor: "CURSOR",
  hermes: "HERMES",
  agent: "AGENT",
};

const statusLabels: Record<Status, string> = {
  starting: "starting",
  working: "working",
  idle: "idle",
  waitingInput: "waiting",
  ended: "ended",
  dead: "dead",
};

function shortId(id: string, chars = 12): string {
  return id.length <= chars ? id : id.slice(0, chars);
}

function modelFamily(model: string | null | undefined): string {
  const value = model?.toLowerCase() ?? "";
  if (value.includes("fugu")) {
    return "Fugu";
  }
  return "Hermes";
}

function fallbackName(session: AgentSession): string {
  if (session.tool === "hermes" && session.connectionRole === "subAgent") {
    return `${modelFamily(session.model)} sub-agent`;
  }
  if (session.tool === "hermes" && session.connectionRole === "orchestrator") {
    return `${modelFamily(session.model)} orchestrator`;
  }
  return `${toolLabels[session.tool]} · ${session.id}`;
}

function titleStatus(session: AgentSession): string {
  if (providerTitle(session)) {
    return "real name";
  }
  return "provider title unavailable";
}

function infoTip(text: string): string {
  return `<span class="info-tip" tabindex="0" aria-label="${escapeHtml(
    text,
  )}"><span aria-hidden="true">i</span><span class="info-tip__bubble" role="tooltip">${escapeHtml(
    text,
  )}</span></span>`;
}

function panelHeading(label: string, tip: string): string {
  return `<h3><span>${escapeHtml(label)}</span>${infoTip(tip)}</h3>`;
}

function connectionLabel(session: AgentSession): string | null {
  if (session.tool !== "hermes" || !session.connectionRole) {
    return null;
  }
  if (session.connectionRole === "subAgent") {
    return "sub-agent";
  }
  const count = session.childCount ?? 0;
  if (count > 0) {
    const noun = count === 1 ? "sub-agent" : "sub-agents";
    return `orchestrator · ${count} ${noun}`;
  }
  return "orchestrator";
}

function renderConnectionBadge(session: AgentSession): string {
  const label = connectionLabel(session);
  if (!label || !session.connectionRole) {
    return "";
  }
  return `<span class="agent-block__connection" data-connection-role="${escapeHtml(
    session.connectionRole,
  )}">${escapeHtml(label)}</span>`;
}

function providerTitle(session: AgentSession): string | null {
  const title = session.title?.trim();
  if (session.titleSource === "provider" && title) {
    return title;
  }
  return null;
}

function renderRenameControl(session: AgentSession, realTitle: string | null): string {
  if (!session.canRename || !realTitle) {
    const value = realTitle ?? "Provider title unavailable";
    const note = realTitle ? "Read-only provider title" : "Provider title unavailable";
    return `
      <div class="rename-box rename-box--readonly">
        <span class="rename-box__label">provider title</span>
        <span>${escapeHtml(note)}</span>
        <strong>${escapeHtml(value)}</strong>
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
        value="${escapeHtml(realTitle)}"
        autocomplete="off"
      />
      <button class="rename-box__button" type="submit">Rename actual chat</button>
    </form>
  `;
}

function renderPromptPanel(session: AgentSession): string {
  const prompt = session.firstPrompt?.trim();
  const hasPrompt = Boolean(prompt);
  const visible = hasPrompt ? prompt! : "first prompt unavailable";
  return `
    <section class="agent-block__panel agent-block__panel--prompt">
      ${panelHeading("first prompt", "Shows the original user assignment Mobius found in the provider log")}
      <div class="prompt-card" data-empty="${hasPrompt ? "false" : "true"}">
        <p>${escapeHtml(visible)}</p>
        ${
          hasPrompt
            ? `<details class="prompt-card__raw"><summary>reveal full prompt</summary><code>${escapeHtml(
                prompt!,
              )}</code></details>`
            : ""
        }
      </div>
    </section>
  `;
}

/** Human label + state class for a Hermes run's `endReason`. */
function runOutcome(endReason: string | null): { label: string; state: string } {
  switch (endReason) {
    case null:
    case undefined:
    case "":
      return { label: "live", state: "live" };
    case "compression":
      return { label: "chained · compaction", state: "chained" };
    case "cli_close":
      return { label: "ended", state: "ended" };
    default:
      return { label: endReason, state: "ended" };
  }
}

/**
 * Run telemetry for autonomous Hermes/Fugu builds — the burn picture. Empty for
 * Claude/Codex (no `run` block). Tokens are the de-facto cost signal because
 * Fugu/Sakana does not write a dollar figure to `state.db`; we surface the cost
 * line only when a positive figure is actually reported, and otherwise say so.
 */
export function renderRunPanel(session: AgentSession): string {
  const run: RunStats | null | undefined = session.run;
  if (!run) {
    return "";
  }

  const totalTokens = session.tokens.input + session.tokens.output + session.tokens.cache;
  const burnRate = formatBurnRate(totalTokens, session.lastEventAt - session.startedAt);
  const outcome = runOutcome(run.endReason ?? null);
  const turnsPct =
    run.maxTurns && run.maxTurns > 0
      ? Math.min(100, Math.round((run.turns / run.maxTurns) * 100))
      : null;
  const turnsLabel = run.maxTurns ? `${run.turns} / ${run.maxTurns}` : `${run.turns}`;
  const costLine =
    run.costUsd != null
      ? `$${run.costUsd.toFixed(run.costUsd < 1 ? 4 : 2)}`
      : `not reported · token burn only`;

  return `
    <section class="agent-block__panel agent-block__panel--run">
      ${panelHeading("run", "Shows autonomous run progress")}
      <div class="run-outcome" data-state="${escapeHtml(outcome.state)}">
        <i class="run-outcome__dot" aria-hidden="true"></i>
        <span>${escapeHtml(outcome.label)}</span>
        ${run.effort ? `<em class="run-outcome__effort">${escapeHtml(run.effort)}</em>` : ""}
      </div>
      <dl class="run-grid">
        <div><dt>burn</dt><dd>${formatTokens(totalTokens)}<small>${escapeHtml(burnRate)}</small></dd></div>
        <div><dt>turns</dt><dd>${escapeHtml(turnsLabel)}</dd></div>
        <div><dt>tools</dt><dd>${formatTokens(run.toolCalls)}</dd></div>
        <div><dt>messages</dt><dd>${formatTokens(run.messages)}</dd></div>
      </dl>
      ${
        turnsPct != null
          ? `<div class="run-bar" role="img" aria-label="turns ${turnsPct}% of cap"><span style="width:${turnsPct}%"></span></div>`
          : ""
      }
      <div class="run-cost"><b>cost</b>${escapeHtml(costLine)}</div>
    </section>
  `;
}

/**
 * Live process subtree for a scanned agent, plus a Stop control. Rendered on any
 * card the scanner attached a tree to — both untracked cards and enriched
 * session cards. The Stop button carries the session id; `main.ts` looks up the
 * tree, builds the kill list, and confirms before signalling.
 */
export function renderProcessPanel(session: AgentSession): string {
  const tree = session.processTree;
  if (!tree) {
    return "";
  }
  const pidCount = flattenPids(tree).length;
  const killLabel =
    pidCount > 1 ? `Stop process tree (${pidCount})` : "Stop process";
  return `
    <section class="agent-block__panel agent-block__panel--process">
      ${panelHeading("process", "Shows the live OS process tree")}
      ${renderProcessTree(tree)}
      <button
        class="process-kill"
        type="button"
        data-kill-session="${escapeHtml(session.id)}"
      >${escapeHtml(killLabel)}</button>
    </section>
  `;
}

function inferredAction(session: AgentSession): string {
  if (session.currentAction && session.currentAction.trim().length > 0) {
    return session.currentAction;
  }
  if (session.tool === "hermes" && session.status === "working" && session.run) {
    const model = session.model ?? "Fugu";
    const calls = session.run.turns === 1 ? "1 call" : `${session.run.turns} calls`;
    return `thinking via ${model} · ${calls}`;
  }
  return "standing by";
}

/**
 * A compact operations block. The provider lane comes first so mixed Claude,
 * Codex, and Cursor sessions can be scanned before reading session details.
 */
export function renderCard(session: AgentSession, expanded: boolean): string {
  const realTitle = providerTitle(session);
  const name = realTitle ?? fallbackName(session);
  const project = basename(session.projectPath);
  const branch = session.branch ?? "—";
  const model = session.model ?? "unknown model";
  const action = inferredAction(session);
  const live = session.status === "working";
  const pid = session.pid != null ? `pid ${session.pid}` : "pid —";
  const expandedClass = expanded ? " agent-block--expanded" : "";
  const titleSource = titleStatus(session);
  const untracked = session.untracked === true;
  const connection = connectionLabel(session);
  const parentId = session.parentSessionId ? shortId(session.parentSessionId) : null;

  return `
    <article class="agent-block${expandedClass}" data-tool="${session.tool}" data-provider="${session.tool}" data-status="${session.status}" data-untracked="${untracked}" data-connection-role="${escapeHtml(
      session.connectionRole ?? "",
    )}">
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
        ${
          untracked
            ? `<span class="agent-block__untracked" title="A signature-matched process with no session card of its own">untracked · no session</span>`
            : ""
        }
        <span class="agent-block__identity">
          <span class="agent-block__tool"><span>${escapeHtml(
            toolLabels[session.tool],
          )}</span><i>${escapeHtml(titleSource)}</i>${renderConnectionBadge(session)}</span>
          <span class="agent-block__name">${escapeHtml(name)}</span>
          ${
            session.canRename && realTitle
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
              ${panelHeading("session", "This is the provider title Mobius read from the agent")}
              ${renderRenameControl(session, realTitle)}
              <div class="agent-block__details">
                <span><b>project</b>${escapeHtml(project)}</span>
                <span><b>branch</b>${escapeHtml(branch)}</span>
                <span><b>model</b>${escapeHtml(model)}</span>
                <span><b>process</b>${escapeHtml(pid)}</span>
                <span><b>provider title</b>${escapeHtml(
                  realTitle ? "available" : "provider title unavailable",
                )}</span>
                ${
                  connection
                    ? `<span><b>connection</b>${escapeHtml(connection)}</span>`
                    : ""
                }
                ${parentId ? `<span><b>parent</b>${escapeHtml(parentId)}</span>` : ""}
              </div>
              <span class="agent-block__id">${escapeHtml(session.id)}</span>
            </section>
            ${renderPromptPanel(session)}
            ${renderRunPanel(session)}
            ${renderProcessPanel(session)}
            <section class="agent-block__panel agent-block__panel--capacity">
              ${panelHeading("capacity", "Shows token usage and context-window pressure")}
              <dl class="token-row">
                <div><dt>in</dt><dd>${formatTokens(session.tokens.input)}</dd></div>
                <div><dt>out</dt><dd>${formatTokens(session.tokens.output)}</dd></div>
                <div><dt>cache</dt><dd>${formatTokens(session.tokens.cache)}</dd></div>
              </dl>
              ${renderContextDetail(session.context)}
            </section>
            <section class="agent-block__panel agent-block__panel--activity">
              ${panelHeading("activity", "Shows recent files and commands touched by the agent")}
              <div class="log">
                <div class="log__head">
                  <span>live activity</span>
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
