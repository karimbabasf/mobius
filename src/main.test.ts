// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { handleSessionGridClick, handleSessionGridSubmit, renderSessions } from "./main";
import type { AgentSession } from "./types";

const renameSessionMock = vi.hoisted(() => vi.fn());
const getSessionsMock = vi.hoisted(() => vi.fn());

vi.mock("./commands", () => ({
  renameSession: renameSessionMock,
  getSessions: getSessionsMock,
}));

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
    firstPrompt: null,
    title,
    titleSource: "provider",
    canRename: true,
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

  it("expands a clicked block inline", () => {
    const sessions = [session("a", "Alpha"), session("b", "Beta")];
    renderSessions(sessions);

    const grid = document.querySelector<HTMLElement>("#session-grid")!;
    const articleBefore = grid.querySelector<HTMLElement>('[data-session-id="a"] .agent-block')!;
    const firstToggle = grid.querySelector<HTMLElement>('[data-session-toggle="a"]')!;
    handleSessionGridClick(new MouseEvent("click", { bubbles: true, cancelable: true }), firstToggle);

    const articleAfter = grid.querySelector<HTMLElement>('[data-session-id="a"] .agent-block')!;
    expect(articleAfter).toBe(articleBefore);
    expect(articleAfter.classList.contains("agent-block--expanded")).toBe(true);
    expect(firstToggle.getAttribute("aria-expanded")).toBe("true");
  });

  it("uses a measured height transition when expanding a block", async () => {
    renderSessions([session("motion", "Motion")]);

    const grid = document.querySelector<HTMLElement>("#session-grid")!;
    const scrollHeight = vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(320);

    const toggle = grid.querySelector<HTMLElement>('[data-session-toggle="motion"]')!;
    handleSessionGridClick(new MouseEvent("click", { bubbles: true, cancelable: true }), toggle);

    const bodyAfter = grid.querySelector<HTMLElement>(
      '[data-session-id="motion"] .agent-block__body',
    )!;
    expect(bodyAfter.style.height).toBe("320px");
    expect(bodyAfter.dataset.expansionState).toBe("opening");

    scrollHeight.mockRestore();
  });

  it("submits supported rename through the Tauri command", async () => {
    renameSessionMock.mockResolvedValueOnce(undefined);
    getSessionsMock.mockResolvedValueOnce([session("a", "Renamed Alpha")]);
    renderSessions([session("a", "Alpha")]);

    const grid = document.querySelector<HTMLElement>("#session-grid")!;
    const form = grid.querySelector<HTMLFormElement>('[data-session-rename="a"]')!;
    const input = form.querySelector<HTMLInputElement>('input[name="newTitle"]')!;
    input.value = "Renamed Alpha";

    await handleSessionGridSubmit(
      new SubmitEvent("submit", { bubbles: true, cancelable: true }),
      form,
    );

    expect(renameSessionMock).toHaveBeenCalledWith("a", "Renamed Alpha");
  });
});
