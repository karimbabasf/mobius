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
  // Every session reaching the UI is a live process now; terminated ones are
  // gated out in the backend, so the strip just counts what it's given.
  it("counts active and working agents", () => {
    const html = renderTopBar([
      session({ status: "working" }),
      session({ status: "working" }),
      session({ status: "idle" }),
    ]);

    expect(html).toContain("active");
    expect(html).toContain("working");
    // 3 active, 2 working.
    expect(html).toContain("<strong>3</strong>");
    expect(html).toContain("<strong>2</strong>");
  });

  it("sums total tokens across the shown sessions", () => {
    const html = renderTopBar([
      session({ status: "working", tokens: { input: 1000, output: 500, cache: 200 } }),
      session({ status: "idle", tokens: { input: 300, output: 0, cache: 0 } }),
    ]);

    expect(html).toContain("tokens");
    expect(html).toContain("2.0k");
  });
});
