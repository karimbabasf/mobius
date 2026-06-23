import { getSessions, killProcesses, renameSession } from "./commands";
import { renderCard } from "./components/agentCard";
import { killPlan } from "./components/processTree";
import { renderTopBar } from "./components/topBar";
import { formatAgo } from "./components/format";
import type { AgentSession } from "./types";

const POLL_INTERVAL_MS = 1500;
const EXIT_ANIM_MS = 260;
const EXPAND_ANIM_MS = 460;

function requiredElement<T extends HTMLElement>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) {
    throw new Error(`Missing required element: ${selector}`);
  }
  return element;
}

// Keyed reconciliation: one wrapper element per session id. New agents animate
// in once, existing strips update in place (no flicker), terminated agents
// animate out then leave. Which strips are expanded is UI state the backend
// knows nothing about, so we track it here and feed it back into each render.
const wrappers = new Map<string, HTMLElement>();
const lastHtml = new Map<string, string>();
const expandedIds = new Set<string>();
const latest = new Map<string, AgentSession>();
let lastTopHtml = "";

// The card HTML is deliberately free of per-second values (relative times are
// empty `data-at` spans filled by `refreshTimes`). That keeps the rendered
// string stable across polls, so a strip's DOM is only rebuilt when its real
// data changes — no flashing the whole interface every tick.
function paint(wrapper: HTMLElement, session: AgentSession): void {
  const html = renderCard(session, expandedIds.has(session.id));
  wrapper.classList.toggle("card-wrapper--expanded", expandedIds.has(session.id));
  if (lastHtml.get(session.id) !== html) {
    wrapper.innerHTML = html;
    lastHtml.set(session.id, html);
    syncExpansionState(wrapper, expandedIds.has(session.id));
  }
  refreshTimes(wrapper);
}

function syncExpansionState(wrapper: HTMLElement, expanded: boolean): void {
  const body = wrapper.querySelector<HTMLElement>(".agent-block__body");
  if (!body) return;
  body.dataset.expansionState = expanded ? "open" : "closed";
  body.style.height = expanded ? "auto" : "0px";
}

function prefersReducedMotion(): boolean {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

function finishMeasuredExpansion(
  body: HTMLElement,
  expanded: boolean,
  token: string,
): void {
  if (body.dataset.expansionRun !== token) return;
  body.dataset.expansionState = expanded ? "open" : "closed";
  body.style.height = expanded ? "auto" : "0px";
  delete body.dataset.expansionRun;
}

function animateMeasuredExpansion(article: HTMLElement, expanded: boolean): void {
  const body = article.querySelector<HTMLElement>(".agent-block__body");
  if (!body) return;

  if (prefersReducedMotion()) {
    body.dataset.expansionState = expanded ? "open" : "closed";
    body.style.height = expanded ? "auto" : "0px";
    delete body.dataset.expansionRun;
    return;
  }

  const token = `${Date.now()}-${Math.random()}`;
  body.dataset.expansionRun = token;
  body.dataset.expansionState = expanded ? "opening" : "closing";

  const settle = (event?: TransitionEvent): void => {
    if (event && (event.target !== body || event.propertyName !== "height")) return;
    body.removeEventListener("transitionend", settle);
    finishMeasuredExpansion(body, expanded, token);
  };
  body.addEventListener("transitionend", settle);
  window.setTimeout(() => settle(), EXPAND_ANIM_MS + 120);

  if (expanded) {
    body.style.height = "0px";
    void body.offsetHeight;
    const targetHeight = body.scrollHeight;
    body.style.height = `${targetHeight}px`;
    return;
  }

  const startHeight = body.getBoundingClientRect().height || body.scrollHeight;
  body.style.height = `${startHeight}px`;
  void body.offsetHeight;
  body.style.height = "0px";
}

function setExpandedDom(wrapper: HTMLElement, session: AgentSession, expanded: boolean): void {
  const article = wrapper.querySelector<HTMLElement>(".agent-block");
  const toggle = wrapper.querySelector<HTMLElement>("[data-session-toggle]");
  if (!article || !toggle) {
    paint(wrapper, session);
    return;
  }

  wrapper.classList.toggle("card-wrapper--expanded", expanded);
  article.classList.toggle("agent-block--expanded", expanded);
  toggle.setAttribute("aria-expanded", expanded ? "true" : "false");
  animateMeasuredExpansion(article, expanded);
  lastHtml.set(session.id, renderCard(session, expanded));
}

/** Update activity-log relative times in place, touching only text nodes — no
 *  DOM teardown, so this can run every second without any visible flicker. */
function refreshTimes(scope: ParentNode): void {
  const now = Date.now();
  scope
    .querySelectorAll<HTMLElement>(".log__time[data-at], .agent-block__uptime[data-at]")
    .forEach((el) => {
      const at = Number(el.dataset.at);
      if (!Number.isNaN(at)) {
        el.textContent = formatAgo(at, now);
      }
    });
}

export function renderSessions(sessions: AgentSession[]): void {
  const metrics = requiredElement<HTMLElement>("#metrics");
  const emptyState = requiredElement<HTMLElement>("#empty-state");
  const grid = requiredElement<HTMLElement>("#session-grid");

  // Only touch the telemetry strip when its content actually changed.
  const topHtml = renderTopBar(sessions);
  if (topHtml !== lastTopHtml) {
    metrics.innerHTML = topHtml;
    lastTopHtml = topHtml;
  }

  latest.clear();
  for (const session of sessions) {
    latest.set(session.id, session);
  }

  const liveIds = new Set(sessions.map((session) => session.id));
  for (const [id, element] of wrappers) {
    if (!liveIds.has(id)) {
      // Process is gone — fade the strip out instead of yanking it, so a
      // termination reads as a deliberate exit rather than a glitch.
      element.classList.add("card-wrapper--exit");
      window.setTimeout(() => element.remove(), EXIT_ANIM_MS);
      wrappers.delete(id);
      lastHtml.delete(id);
      expandedIds.delete(id);
    }
  }

  let nextNode: ChildNode | null = grid.firstChild;
  for (const session of sessions) {
    let wrapper = wrappers.get(session.id);
    if (!wrapper) {
      wrapper = document.createElement("div");
      wrapper.className = "card-wrapper card-wrapper--enter";
      wrapper.dataset.sessionId = session.id;
      wrappers.set(session.id, wrapper);
    }
    paint(wrapper, session);
    if (wrapper === nextNode) {
      nextNode = wrapper.nextSibling;
    } else {
      grid.insertBefore(wrapper, nextNode);
    }
  }

  emptyState.hidden = sessions.length > 0;
  grid.hidden = sessions.length === 0;
}

function toggleExpanded(id: string): void {
  if (expandedIds.has(id)) {
    expandedIds.delete(id);
  } else {
    expandedIds.add(id);
  }
  const session = latest.get(id);
  const wrapper = wrappers.get(id);
  if (session && wrapper) {
    const expanded = expandedIds.has(id);
    setExpandedDom(wrapper, session, expanded);
    // Opening a renameable card drops the cursor straight into the name field,
    // so the prominent title doubles as the entry point for editing it.
    if (expanded && session.canRename) {
      const input = wrapper.querySelector<HTMLInputElement>('input[name="newTitle"]');
      if (input) {
        window.setTimeout(() => input.focus(), 0);
      }
    }
  }
}

export function handleSessionGridClick(event: Event, explicitTarget?: HTMLElement): void {
  const target = explicitTarget ?? (event.target as HTMLElement);

  // A Stop control lives in the (expanded) card body, outside the head toggle.
  const kill = target.closest<HTMLElement>("[data-kill-session]");
  if (kill) {
    const id = kill.getAttribute("data-kill-session");
    if (id) void handleKill(id);
    return;
  }

  const head = target.closest<HTMLElement>("[data-session-toggle]");
  if (!head) return;
  const id = head.getAttribute("data-session-toggle");
  if (id) {
    toggleExpanded(id);
  }
}

/** Confirm exactly which processes will be signalled, then send SIGTERM. */
async function handleKill(id: string): Promise<void> {
  const session = latest.get(id);
  if (!session?.processTree) return;
  const plan = killPlan(session.processTree);
  if (!window.confirm(plan.text)) return;
  try {
    await killProcesses(plan.pids);
  } catch (error) {
    console.error("Failed to stop processes", error);
  }
  void loadSessions();
}

export async function handleSessionGridSubmit(
  event: Event,
  explicitForm?: HTMLFormElement,
): Promise<void> {
  const target = explicitForm ?? (event.target as HTMLElement);
  const form = target.closest<HTMLFormElement>("[data-session-rename]");
  if (!form) return;
  event.preventDefault();

  const sessionId = form.getAttribute("data-session-rename");
  const input = form.querySelector<HTMLInputElement>('input[name="newTitle"]');
  const newTitle = input?.value.trim() ?? "";
  if (!sessionId || !newTitle) return;

  await renameSession(sessionId, newTitle);
  const session = latest.get(sessionId);
  if (session) {
    session.title = newTitle;
    const wrapper = wrappers.get(sessionId);
    if (wrapper) {
      paint(wrapper, session);
    }
  }
  void loadSessions();
}

async function loadSessions(): Promise<void> {
  try {
    const sessions = await getSessions();
    renderSessions(sessions);
    document.body.dataset.ready = "true";
  } catch (error) {
    console.error("Failed to load sessions", error);
  }
}

window.addEventListener("DOMContentLoaded", () => {
  // Delegate strip toggles once; wrappers come and go, the grid stays.
  const grid = requiredElement<HTMLElement>("#session-grid");
  grid.addEventListener("click", handleSessionGridClick);
  grid.addEventListener("submit", (event) => void handleSessionGridSubmit(event));

  void loadSessions();
  window.setInterval(() => void loadSessions(), POLL_INTERVAL_MS);
  // Keep relative times current without re-rendering any strips.
  window.setInterval(() => refreshTimes(grid), 1000);
});
