import type { AgentSession } from "../types";

function liveSessions(sessions: AgentSession[]): AgentSession[] {
  return sessions.filter(
    (session) => session.status !== "ended" && session.status !== "dead",
  );
}

function totalCost(sessions: AgentSession[]): number {
  return sessions.reduce((sum, session) => sum + (session.costUsd ?? 0), 0);
}

export function renderTopBar(sessions: AgentSession[]): string {
  const live = liveSessions(sessions);
  const working = live.filter((session) => session.status === "working").length;

  return `
    <article class="metric">
      <span class="metric-label">Active agents</span>
      <strong>${live.length}</strong>
    </article>
    <article class="metric">
      <span class="metric-label">Working</span>
      <strong>${working}</strong>
    </article>
    <article class="metric">
      <span class="metric-label">24h estimated spend</span>
      <strong>$${totalCost(live).toFixed(2)}</strong>
    </article>
  `;
}
