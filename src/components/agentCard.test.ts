import { describe, expect, it } from "vitest";
import { renderCard } from "./agentCard";
import type { AgentSession } from "../types";

function session(partial: Partial<AgentSession> = {}): AgentSession {
  return {
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
    title: "Build a local agent tracker",
    recentFiles: [],
    ...partial,
  };
}

describe("renderCard", () => {
  it("shows the session title, status, model, branch, and tokens", () => {
    const html = renderCard(session());

    expect(html).toContain("Build a local agent tracker");
    expect(html).toContain("working");
    expect(html).toContain("claude-opus-4-8");
    expect(html).toContain("main");
    expect(html).toContain("2.2k");
  });

  it("renders no dollar cost anywhere", () => {
    const html = renderCard(session({ tokens: { input: 0, output: 0, cache: 0 } }));

    expect(html).not.toContain("$");
    expect(html).not.toContain("n/a");
  });

  it("falls back to the project folder name when there is no title", () => {
    const html = renderCard(session({ title: null }));

    expect(html).toContain("project");
  });
});
