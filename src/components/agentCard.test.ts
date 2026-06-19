import { describe, expect, it } from "vitest";
import { renderCard } from "./agentCard";
import type { AgentSession } from "../types";

function session(partial: Partial<AgentSession> = {}): AgentSession {
  return {
    id: "f89686b4-e4df-4d13-8fab-90267a9d08c1",
    tool: "claude",
    pid: 1234,
    projectPath: "/Users/you/project",
    branch: "main",
    model: "claude-opus-4-8",
    status: "working",
    currentAction: "Running cargo test",
    startedAt: 0,
    lastEventAt: 0,
    tokens: { input: 1200, output: 320, cache: 640 },
    title: "Build a local agent tracker",
    recentFiles: [{ path: "/Users/you/project/src/main.ts", action: "editing", at: 0 }],
    ...partial,
  };
}

describe("renderCard", () => {
  it("shows the title, full session id, model, branch, and split token counts", () => {
    const html = renderCard(session());

    expect(html).toContain("Build a local agent tracker");
    expect(html).toContain("f89686b4-e4df-4d13-8fab-90267a9d08c1");
    expect(html).toContain("claude-opus-4-8");
    expect(html).toContain("main");
    expect(html).toContain("1.2k");
  });

  it("includes the tool logo and the file activity log", () => {
    const html = renderCard(session());

    expect(html).toContain("<svg");
    expect(html).toContain("tool-logo--claude");
    expect(html).toContain("editing");
    expect(html).toContain("main.ts");
  });

  it("renders no dollar cost", () => {
    const html = renderCard(session({ tokens: { input: 0, output: 0, cache: 0 } }));

    expect(html).not.toContain("$");
  });

  it("falls back to the folder name and shows an empty file log", () => {
    const html = renderCard(session({ title: null, recentFiles: [] }));

    expect(html).toContain("project");
    expect(html).toContain("no file activity");
  });
});
