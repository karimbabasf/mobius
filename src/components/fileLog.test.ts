import { describe, expect, it } from "vitest";
import { renderFileLog } from "./fileLog";
import type { FileEvent } from "../types";

describe("renderFileLog", () => {
  it("shows an empty state when there is no activity", () => {
    expect(renderFileLog([], false)).toContain("no activity recorded");
  });

  it("renders a status tag and file name per event", () => {
    const events: FileEvent[] = [
      { path: "/a/b/styles.css", action: "editing", at: 0 },
      { path: "build.log", action: "appending", at: 0 },
    ];
    const html = renderFileLog(events, false);
    expect(html).toContain("log__tag--editing");
    expect(html).toContain("styles.css");
    expect(html).toContain("log__tag--appending");
    expect(html).toContain("build.log");
  });

  it("keeps shell commands behind an explicit reveal control", () => {
    const events: FileEvent[] = [
      { path: "cargo test --lib hermes -- --nocapture", action: "running", at: 0 },
    ];
    const html = renderFileLog(events, true);

    expect(html).toContain("shell command");
    expect(html).toContain("reveal raw");
    expect(html).toContain("<code>cargo test --lib hermes -- --nocapture</code>");
  });

  it("marks the newest event as the live 'now' row when the agent is working", () => {
    const events: FileEvent[] = [
      { path: "/a/now.ts", action: "editing", at: 0 },
      { path: "/a/past.ts", action: "reading", at: 0 },
    ];
    const live = renderFileLog(events, true);
    expect(live).toContain("log__row--now");
    expect(live).toContain("log__row--past");

    // Idle agent has no present tense — every row is past.
    const idle = renderFileLog(events, false);
    expect(idle).not.toContain("log__row--now");
  });

  it("wraps activity in a scrollable live stream with an action-specific now marker", () => {
    const events: FileEvent[] = [
      { path: "/a/now.ts", action: "editing", at: 0 },
      { path: "/a/past.ts", action: "reading", at: 0 },
    ];
    const html = renderFileLog(events, true);

    expect(html).toContain('class="log__viewport"');
    expect(html).toContain('aria-label="scrollable live activity"');
    expect(html).toContain("log__live-pulse");
    expect(html).toContain("changing now");
  });

  it("labels running commands as running now", () => {
    const html = renderFileLog(
      [{ path: "cargo test --lib", action: "running", at: 0 }],
      true,
    );

    expect(html).toContain("running now");
  });

  it("escapes file names to prevent HTML injection", () => {
    const events: FileEvent[] = [
      { path: "/x/<img src=x onerror=alert(1)>.ts", action: "reading", at: 0 },
    ];
    const html = renderFileLog(events, false);
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;img");
  });
});
