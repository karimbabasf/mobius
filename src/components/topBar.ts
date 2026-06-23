import type { AgentSession, Tool } from "../types";
import { formatTokens } from "./format";

const providerLabels: Record<Tool, string> = {
  claude: "Claude",
  codex: "Codex",
  cursor: "Cursor",
  hermes: "Hermes",
  agent: "Agent",
};

const providerOrder: Tool[] = ["claude", "codex", "cursor", "hermes", "agent"];

function totalTokens(sessions: AgentSession[]): number {
  return sessions.reduce(
    (sum, session) =>
      sum + session.tokens.input + session.tokens.output + session.tokens.cache,
    0,
  );
}

function providerBreakdown(sessions: AgentSession[]): string {
  return providerOrder
    .map((tool) => ({
      tool,
      count: sessions.filter((session) => session.tool === tool).length,
    }))
    .filter(({ count }) => count > 0)
    .map(
      ({ tool, count }) => `
    <span class="telemetry__provider" data-provider="${tool}">
      <strong>${count}</strong><span class="telemetry__label">${providerLabels[tool]}</span>
    </span>`,
    )
    .join("");
}

/**
 * The telemetry strip under the prompt. Every session shown is, by definition,
 * a live process now (terminated ones are gone), so "active" is just the count.
 */
export function renderTopBar(sessions: AgentSession[]): string {
  const working = sessions.filter(
    (session) => session.status === "working",
  ).length;
  const providers = providerBreakdown(sessions);

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
    ${
      providers
        ? `<span class="telemetry__sep" aria-hidden="true">·</span>
    <span class="telemetry__providers" aria-label="Visible agent providers">${providers}
    </span>`
        : ""
    }
  `;
}
