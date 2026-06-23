import { describe, expect, it } from "vitest";
import { flattenPids, killPlan, renderProcessTree } from "./processTree";
import type { ProcessNode } from "../types";

const tree: ProcessNode = {
  pid: 200,
  command: "/Users/x/.hermes/v/bin/hermes -z build",
  children: [
    {
      pid: 300,
      command: "cargo build",
      children: [{ pid: 305, command: "rustc src/main.rs", children: [] }],
    },
    { pid: 301, command: "git status", children: [] },
  ],
};

describe("flattenPids", () => {
  it("returns every pid root-first, depth-first", () => {
    expect(flattenPids(tree)).toEqual([200, 300, 305, 301]);
  });

  it("handles a lone root", () => {
    expect(flattenPids({ pid: 7, command: "ollama serve", children: [] })).toEqual([7]);
  });
});

describe("renderProcessTree", () => {
  it("renders a nested list with each pid and command", () => {
    const html = renderProcessTree(tree);
    expect(html).toContain("process-tree");
    expect(html).toContain("200");
    expect(html).toContain("305");
    expect(html).toContain("cargo build");
    expect(html).toContain("rustc src/main.rs");
    // nesting: a child <ul> exists for the cargo subtree
    expect(html.match(/<ul/g)?.length).toBeGreaterThanOrEqual(2);
  });

  it("escapes commands to avoid HTML injection", () => {
    const html = renderProcessTree({
      pid: 1,
      command: "evil <script>alert(1)</script>",
      children: [],
    });
    expect(html).toContain("&lt;script&gt;");
    expect(html).not.toContain("<script>");
  });
});

describe("killPlan", () => {
  it("lists every pid and a confirm line per process, root-first", () => {
    const plan = killPlan(tree);
    expect(plan.pids).toEqual([200, 300, 305, 301]);
    expect(plan.text).toContain("This will send SIGTERM to 4 process");
    expect(plan.text).toContain("200  /Users/x/.hermes/v/bin/hermes -z build");
    expect(plan.text).toContain("305  rustc src/main.rs");
    expect(plan.text).toContain("301  git status");
  });

  it("uses singular phrasing for a lone process", () => {
    const plan = killPlan({ pid: 7, command: "ollama serve", children: [] });
    expect(plan.pids).toEqual([7]);
    expect(plan.text).toContain("1 process");
    expect(plan.text).not.toContain("processes");
  });
});
