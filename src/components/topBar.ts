import type { AgentSession } from "../types";
import { formatTokens } from "./format";

function totalTokens(sessions: AgentSession[]): number {
  return sessions.reduce(
    (sum, session) =>
      sum + session.tokens.input + session.tokens.output + session.tokens.cache,
    0,
  );
}

/**
 * The telemetry strip under the prompt. Every session shown is, by definition,
 * a live process now (terminated ones are gone), so "active" is just the count.
 */
export function renderTopBar(sessions: AgentSession[]): string {
  const working = sessions.filter(
    (session) => session.status === "working",
  ).length;

  return `
    <span class="telemetry__item">
      <strong>${sessions.length}</strong><span class="telemetry__label">active</span>
    </span>
    <span class="telemetry__sep" aria-hidden="true">·</span>
    <span class="telemetry__item">
      <strong>${working}</strong><span class="telemetry__label">working</span>
    </span>
    <span class="telemetry__sep" aria-hidden="true">·</span>
    <span class="telemetry__item">
      <strong>${formatTokens(totalTokens(sessions))}</strong><span class="telemetry__label">tokens</span>
    </span>
  `;
}
