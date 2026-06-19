import { invoke } from "@tauri-apps/api/core";
import { renderCard } from "./components/agentCard";
import { renderTopBar } from "./components/topBar";
import type { AgentSession } from "./types";

function requiredElement<T extends HTMLElement>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) {
    throw new Error(`Missing required element: ${selector}`);
  }
  return element;
}

export function renderSessions(sessions: AgentSession[]): void {
  const metrics = requiredElement<HTMLElement>("#metrics");
  const emptyState = requiredElement<HTMLElement>("#empty-state");
  const grid = requiredElement<HTMLElement>("#session-grid");

  metrics.innerHTML = renderTopBar(sessions);
  grid.innerHTML = sessions.map(renderCard).join("");
  emptyState.hidden = sessions.length > 0;
  grid.hidden = sessions.length === 0;
}

async function loadSessions(): Promise<void> {
  const sessions = await invoke<AgentSession[]>("get_sessions");
  renderSessions(sessions);
  document.body.dataset.ready = "true";
}

window.addEventListener("DOMContentLoaded", () => {
  void loadSessions();
});
