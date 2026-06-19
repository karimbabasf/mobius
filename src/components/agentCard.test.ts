import { describe, expect, it } from "vitest";
import { renderCard } from "./agentCard";
import type { AgentSession } from "../types";

describe("renderCard", () => {
  it("shows project, status, model, branch, tokens, and cost", () => {
    const html = renderCard({
      id: "demo",
      tool: "claude",
      pid: 1234,
      projectPath: "/Users/you/project",
      branch: "main",
      model: "claude-opus-4-8",
      status: "working",
      currentAction: "running Bash",
      startedAt: 1_820_000_000_000,
      lastEventAt: 1_820_000_005_000,
      tokens: { input: 1200, output: 320, cache: 640 },
      costUsd: 0.08,
    });

    expect(html).toContain("project");
    expect(html).toContain("working");
    expect(html).toContain("claude-opus-4-8");
    expect(html).toContain("main");
    expect(html).toContain("2.2k");
    expect(html).toContain("$0.08");
  });

  it("renders unavailable cost as n/a", () => {
    const html = renderCard({
      id: "cursor-demo",
      tool: "cursor",
      pid: null,
      projectPath: "/Users/you/cursor-project",
      branch: null,
      model: null,
      status: "idle",
      currentAction: null,
      startedAt: 1_820_000_000_000,
      lastEventAt: 1_820_000_005_000,
      tokens: { input: 0, output: 0, cache: 0 },
      costUsd: null,
    } satisfies AgentSession);

    expect(html).toContain("n/a");
  });
});
