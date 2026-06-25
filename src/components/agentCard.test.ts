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
    context: null,
    firstPrompt: null,
    title: "Build a local agent tracker",
    titleSource: "provider",
    canRename: true,
    recentFiles: [{ path: "/Users/you/project/src/main.ts", action: "editing", at: 0 }],
    ...partial,
  };
}

describe("renderCard", () => {
  it("shows the title, full session id, model, pid, and split token counts", () => {
    const html = renderCard(session(), false);

    expect(html).toContain("Build a local agent tracker");
    expect(html).toContain("f89686b4-e4df-4d13-8fab-90267a9d08c1");
    expect(html).toContain("claude-opus-4-8");
    expect(html).toContain("pid 1234");
    expect(html).toContain("1.2k");
  });

  it("shows uptime (not branch) in the collapsed header meta, with branch kept in the expanded panel", () => {
    const html = renderCard(session(), false);

    // The header meta row now carries a live 'started' uptime instead of branch.
    expect(html).toContain("<b>started</b>");
    expect(html).toContain('class="agent-block__uptime" data-at="0"');

    // Branch is no longer duplicated onto the header — it lives once, in the
    // expanded session panel.
    const branchLabels = html.match(/<b>branch<\/b>/g) ?? [];
    expect(branchLabels.length).toBe(1);
    expect(html).toContain("main");
  });

  it("surfaces a click-to-rename hint for renameable sessions only", () => {
    expect(renderCard(session(), false)).toContain("agent-block__rename-hint");
    expect(renderCard(session({ canRename: false }), false)).not.toContain(
      "agent-block__rename-hint",
    );
  });

  it("reflects the expanded state via aria-expanded", () => {
    expect(renderCard(session(), false)).toContain('aria-expanded="false"');
    expect(renderCard(session(), true)).toContain('aria-expanded="true"');
    expect(renderCard(session(), true)).toContain("agent-block--expanded");
  });

  it("renders a supported real-name rename form", () => {
    const html = renderCard(session(), true);

    expect(html).toContain('data-session-rename="f89686b4-e4df-4d13-8fab-90267a9d08c1"');
    expect(html).toContain('name="newTitle"');
    expect(html).toContain("Rename actual chat");
    expect(html).toContain("provider");
  });

  it("disables rename when the provider has no writable name field", () => {
    const html = renderCard(
      session({ canRename: false, titleSource: "fallback", title: null }),
      true,
    );

    expect(html).not.toContain("Rename actual chat");
    expect(html).toContain("Provider title unavailable");
    expect(html).toContain("Claude · f89686b4-e4df-4d13-8fab-90267a9d08c1");
  });

  it("includes the tool logo and the file activity log", () => {
    const html = renderCard(session(), true);

    expect(html).toContain("<svg");
    expect(html).toContain("tool-logo--claude");
    expect(html).toContain("live activity");
    expect(html).toContain("log__tag--editing");
    expect(html).toContain("main.ts");
  });

  it("renders the first prompt in an expanded prompt panel", () => {
    const html = renderCard(
      session({
        firstPrompt: "My Hermes sub-agents aren't getting tracked. Show what each one is doing.",
      }),
      true,
    );

    expect(html).toContain("agent-block__panel--prompt");
    expect(html).toContain("first prompt");
    expect(html).toContain("My Hermes sub-agents aren&#39;t getting tracked");
    expect(html).toContain("Show what each one is doing.");
  });

  it("shows an unavailable prompt fallback when the provider has no first prompt", () => {
    const html = renderCard(session({ firstPrompt: null }), true);

    expect(html).toContain("agent-block__panel--prompt");
    expect(html).toContain("first prompt unavailable");
  });

  it("renders clear provider badges without the old shell prompt sigil", () => {
    const claudeHtml = renderCard(session({ tool: "claude" }), false);
    const codexHtml = renderCard(
      session({
        tool: "codex",
        model: "gpt-5-codex",
        title: "Wire the tray launcher",
      }),
      false,
    );

    expect(claudeHtml).toContain('data-provider="claude"');
    expect(claudeHtml).toContain("CLAUDE");
    expect(codexHtml).toContain('data-provider="codex"');
    expect(codexHtml).toContain("CODEX");
    expect(claudeHtml).not.toContain("agent-block__prompt");
    expect(codexHtml).not.toContain("agent-block__prompt");
  });

  it("falls back to provider plus session id and shows an empty activity log", () => {
    const html = renderCard(session({ title: null, recentFiles: [] }), true);

    expect(html).toContain(
      '<span class="agent-block__name">Claude · f89686b4-e4df-4d13-8fab-90267a9d08c1</span>',
    );
    expect(html).not.toContain('<span class="agent-block__name">project</span>');
    expect(html).toContain("provider title unavailable");
    expect(html).toContain("no activity recorded");
  });

  it("omits the run panel for non-Hermes sessions", () => {
    expect(renderCard(session(), true)).not.toContain("agent-block__panel--run");
  });

  it("renders a Hermes run panel with burn, turns, effort, and live outcome", () => {
    const html = renderCard(
      session({
        tool: "hermes",
        model: "fugu-ultra",
        startedAt: 0,
        lastEventAt: 60_000, // 1 min elapsed
        tokens: { input: 300_000, output: 100_000, cache: 50_000 },
        run: {
          turns: 110,
          maxTurns: 800,
          toolCalls: 63,
          messages: 220,
          effort: "xhigh",
          costUsd: null,
          costStatus: "unknown",
          endReason: null,
        },
      }),
      true,
    );

    expect(html).toContain("agent-block__panel--run");
    // 450k total tokens burned over 1 min
    expect(html).toContain("450.0k");
    expect(html).toContain("/min");
    // turns vs cap and the progress bar
    expect(html).toContain("110 / 800");
    expect(html).toContain("run-bar");
    // effort badge + live state
    expect(html).toContain("xhigh");
    expect(html).toContain('data-state="live"');
    // cost is unreported for Fugu/Sakana -> honest fallback
    expect(html).toContain("token burn only");
  });

  it("infers a Hermes thinking state when no current tool action is flushed yet", () => {
    const html = renderCard(
      session({
        tool: "hermes",
        model: "fugu-ultra",
        status: "working",
        currentAction: null,
        run: {
          turns: 33,
          maxTurns: 90,
          toolCalls: 0,
          messages: 1,
          effort: null,
          costUsd: null,
          costStatus: "unknown",
          endReason: null,
        },
      }),
      false,
    );

    expect(html).toContain("thinking via fugu-ultra");
    expect(html).toContain("33 calls");
  });

  it("labels Hermes orchestrator and sub-agent connections", () => {
    const orchestrator = renderCard(
      session({
        tool: "hermes",
        model: "fugu-ultra",
        connectionRole: "orchestrator",
        childCount: 2,
      }),
      false,
    );
    expect(orchestrator).toContain('data-connection-role="orchestrator"');
    expect(orchestrator).toContain("orchestrator");
    expect(orchestrator).toContain("2 sub-agents");

    const child = renderCard(
      session({
        id: "20260622_182940_8a25",
        tool: "hermes",
        model: "fugu-ultra",
        title: null,
        connectionRole: "subAgent",
        parentSessionId: "20260622_181033_0e11",
      }),
      true,
    );
    expect(child).toContain('data-connection-role="subAgent"');
    expect(child).toContain("Fugu sub-agent");
    expect(child).toContain("parent");
    expect(child).toContain("20260622_181");
  });

  it("flags an untracked process card with a badge and data attribute", () => {
    const html = renderCard(
      session({
        id: "proc:59421:200",
        tool: "hermes",
        untracked: true,
        title: "hermes",
        currentAction: "/Users/x/.hermes/v/bin/hermes -z build the thing",
        processTree: { pid: 59421, command: "/Users/x/.hermes/v/bin/hermes -z build", children: [] },
      }),
      false,
    );
    expect(html).toContain('data-untracked="true"');
    expect(html).toContain("untracked");
    expect(html).toContain("no session");
  });

  it("renders a process-tree panel and a Stop control when a tree is attached", () => {
    const html = renderCard(
      session({
        id: "proc:59421:200",
        untracked: true,
        processTree: {
          pid: 59421,
          command: "/Users/x/.hermes/v/bin/hermes -z build",
          children: [{ pid: 63318, command: "cargo build", children: [] }],
        },
      }),
      true,
    );
    expect(html).toContain("process-tree");
    expect(html).toContain("63318");
    expect(html).toContain("cargo build");
    expect(html).toContain('data-kill-session="proc:59421:200"');
    expect(html).toContain("Stop");
  });

  it("adds practical info hovers to every expanded section that renders", () => {
    const html = renderCard(
      session({
        tool: "hermes",
        model: "fugu-ultra",
        run: {
          turns: 44,
          maxTurns: 150,
          toolCalls: 63,
          messages: 110,
          effort: "medium",
          costUsd: 0.42,
          costStatus: "unknown",
          endReason: "compression",
        },
        processTree: {
          pid: 59421,
          command: "/Users/x/.hermes/v/bin/hermes -z build",
          children: [],
        },
      }),
      true,
    );

    expect(html.match(/class="info-tip"/g)?.length).toBe(6);
    expect(html).toContain("This is the provider title Mobius read from the agent");
    expect(html).toContain("Shows the original user assignment Mobius found in the provider log");
    expect(html).toContain("Shows autonomous run progress");
    expect(html).toContain("Shows the live OS process tree");
    expect(html).toContain("Shows token usage and context-window pressure");
    expect(html).toContain("Shows recent files and commands touched by the agent");
  });

  it("omits the process panel and Stop control when there is no tree", () => {
    const html = renderCard(session(), true);
    expect(html).not.toContain("agent-block__panel--process");
    expect(html).not.toContain("data-kill-session");
  });

  it("renders a generic AGENT provider badge and logo for scanner-detected agents", () => {
    const html = renderCard(
      session({ tool: "agent", untracked: true, title: "ollama", processTree: { pid: 9, command: "ollama serve", children: [] } }),
      false,
    );
    expect(html).toContain('data-provider="agent"');
    expect(html).toContain("AGENT");
    expect(html).toContain("tool-logo--agent");
  });

  it("labels chained (compaction) and ended Hermes runs distinctly", () => {
    const base = {
      tool: "hermes" as const,
      run: {
        turns: 44,
        maxTurns: 150,
        toolCalls: 63,
        messages: 110,
        effort: "medium",
        costUsd: 0.42,
        costStatus: "unknown",
        endReason: "compression",
      },
    };
    const chained = renderCard(session(base), true);
    expect(chained).toContain('data-state="chained"');
    expect(chained).toContain("compaction");
    // a reported positive cost is shown as a dollar figure
    expect(chained).toContain("$0.4200");

    const ended = renderCard(
      session({ ...base, run: { ...base.run, endReason: "cli_close" } }),
      true,
    );
    expect(ended).toContain('data-state="ended"');
  });
});
