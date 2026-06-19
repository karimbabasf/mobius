import { describe, expect, it } from "vitest";
import { renderTopBar } from "./topBar";
import type { AgentSession } from "../types";

function session(partial: Partial<AgentSession>): AgentSession {
  return {
    id: "s",
    tool: "claude",
    pid: null,
    projectPath: "/p",
    branch: null,
    model: null,
    status: "working",
    currentAction: null,
    startedAt: 0,
    lastEventAt: 0,
    tokens: { input: 0, output: 0, cache: 0 },
    title: null,
    recentFiles: [],
    ...partial,
  };
}

describe("renderTopBar", () => {
  it("counts active and working agents, excluding ended", () => {
    const html = renderTopBar([
      session({ status: "working" }),
      session({ status: "idle" }),
      session({ status: "ended" }),
    ]);

    expect(html).toContain("Active agents");
    expect(html).toContain("Working");
  });

  it("sums total live tokens across active sessions only", () => {
    const html = renderTopBar([
      session({ status: "working", tokens: { input: 1000, output: 500, cache: 200 } }),
      session({ status: "idle", tokens: { input: 300, output: 0, cache: 0 } }),
      session({ status: "ended", tokens: { input: 9999, output: 9999, cache: 9999 } }),
    ]);

    expect(html).toContain("Total tokens");
    expect(html).toContain("2.0k");
  });
});
