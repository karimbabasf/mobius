import type { AgentSession } from "../types";
import { formatTokens } from "./format";

function liveSessions(sessions: AgentSession[]): AgentSession[] {
  return sessions.filter(
    (session) => session.status !== "ended" && session.status !== "dead",
  );
}

function totalTokens(sessions: AgentSession[]): number {
  return sessions.reduce(
    (sum, session) =>
      sum + session.tokens.input + session.tokens.output + session.tokens.cache,
    0,
  );
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
      <span class="metric-label">Total tokens (live)</span>
      <strong>${formatTokens(totalTokens(live))}</strong>
    </article>
  `;
}
