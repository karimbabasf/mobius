import { invoke } from "@tauri-apps/api/core";
import { renderCard } from "./components/agentCard";
import { renderTopBar } from "./components/topBar";
import { formatAgo } from "./components/format";
import type { AgentSession } from "./types";

const POLL_INTERVAL_MS = 1500;
const EXIT_ANIM_MS = 260;

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
  if (lastHtml.get(session.id) !== html) {
    wrapper.innerHTML = html;
    lastHtml.set(session.id, html);
  }
  refreshTimes(wrapper);
}

/** Update activity-log relative times in place, touching only text nodes — no
 *  DOM teardown, so this can run every second without any visible flicker. */
function refreshTimes(scope: ParentNode): void {
  const now = Date.now();
  scope.querySelectorAll<HTMLElement>(".log__time[data-at]").forEach((el) => {
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
    paint(wrapper, session);
  }
}

async function loadSessions(): Promise<void> {
  try {
    const sessions = await invoke<AgentSession[]>("get_sessions");
    renderSessions(sessions);
    document.body.dataset.ready = "true";
  } catch (error) {
    console.error("Failed to load sessions", error);
  }
}

window.addEventListener("DOMContentLoaded", () => {
  // Delegate strip toggles once; wrappers come and go, the grid stays.
  const grid = requiredElement<HTMLElement>("#session-grid");
  grid.addEventListener("click", (event) => {
    const head = (event.target as HTMLElement).closest<HTMLElement>(
      "[data-session-toggle]",
    );
    if (!head) return;
    const id = head.getAttribute("data-session-toggle");
    if (id) toggleExpanded(id);
  });

  void loadSessions();
  window.setInterval(() => void loadSessions(), POLL_INTERVAL_MS);
  // Keep relative times current without re-rendering any strips.
  window.setInterval(() => refreshTimes(grid), 1000);
});
