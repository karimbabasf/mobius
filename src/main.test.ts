// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderSessions } from "./main";
import type { AgentSession } from "./types";

function session(id: string, title: string): AgentSession {
  return {
    id,
    tool: "codex",
    pid: 1234,
    projectPath: `/Users/you/${id}`,
    branch: "main",
    model: "gpt-5",
    status: "working",
    currentAction: "Editing app.ts",
    startedAt: 0,
    lastEventAt: 0,
    tokens: { input: 10, output: 5, cache: 0 },
    context: null,
    title,
    recentFiles: [{ path: `/Users/you/${id}/app.ts`, action: "editing", at: 0 }],
  };
}

describe("renderSessions", () => {
  beforeEach(() => {
    document.body.innerHTML = `
      <section id="metrics"></section>
      <section id="empty-state"></section>
      <section id="session-grid"></section>
    `;
  });

  it("does not reappend unchanged cards on refresh", () => {
    const sessions = [session("a", "Alpha"), session("b", "Beta")];
    renderSessions(sessions);

    const grid = document.querySelector<HTMLElement>("#session-grid");
    expect(grid).not.toBeNull();
    const append = vi.spyOn(grid!, "appendChild");

    renderSessions(sessions);

    expect(append).not.toHaveBeenCalled();
    expect(Array.from(grid!.children).map((el) => el.getAttribute("data-session-id"))).toEqual([
      "a",
      "b",
    ]);
  });
});
