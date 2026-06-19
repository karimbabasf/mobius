import { invoke } from "@tauri-apps/api/core";
import { renderCard } from "./components/agentCard";
import { renderTopBar } from "./components/topBar";
import type { AgentSession } from "./types";

const POLL_INTERVAL_MS = 1500;

function requiredElement<T extends HTMLElement>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) {
    throw new Error(`Missing required element: ${selector}`);
  }
  return element;
}

// Keyed reconciliation: keep one wrapper element per session id so new agents
// animate in once, existing cards update in place (no flicker), and ended
// sessions are removed. Order follows the backend's newest-first sort.
const wrappers = new Map<string, HTMLElement>();
const lastHtml = new Map<string, string>();

export function renderSessions(sessions: AgentSession[]): void {
  const metrics = requiredElement<HTMLElement>("#metrics");
  const emptyState = requiredElement<HTMLElement>("#empty-state");
  const grid = requiredElement<HTMLElement>("#session-grid");

  metrics.innerHTML = renderTopBar(sessions);

  const liveIds = new Set(sessions.map((session) => session.id));
  for (const [id, element] of wrappers) {
    if (!liveIds.has(id)) {
      element.remove();
      wrappers.delete(id);
      lastHtml.delete(id);
    }
  }

  for (const session of sessions) {
    let wrapper = wrappers.get(session.id);
    if (!wrapper) {
      wrapper = document.createElement("div");
      wrapper.className = "card-wrapper card-wrapper--enter";
      wrapper.dataset.sessionId = session.id;
      wrappers.set(session.id, wrapper);
    }
    const html = renderCard(session);
    if (lastHtml.get(session.id) !== html) {
      wrapper.innerHTML = html;
      lastHtml.set(session.id, html);
    }
    grid.appendChild(wrapper);
  }

  emptyState.hidden = sessions.length > 0;
  grid.hidden = sessions.length === 0;
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
  void loadSessions();
  window.setInterval(() => void loadSessions(), POLL_INTERVAL_MS);
});
