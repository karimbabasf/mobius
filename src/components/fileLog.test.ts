import { describe, expect, it } from "vitest";
import { renderFileLog } from "./fileLog";
import type { FileEvent } from "../types";

describe("renderFileLog", () => {
  it("shows an empty state when there is no activity", () => {
    expect(renderFileLog([], false, 0)).toContain("no activity recorded");
  });

  it("renders a status tag and file name per event", () => {
    const events: FileEvent[] = [
      { path: "/a/b/styles.css", action: "editing", at: 0 },
      { path: "build.log", action: "appending", at: 0 },
    ];
    const html = renderFileLog(events, false, 0);
    expect(html).toContain("log__tag--editing");
    expect(html).toContain("styles.css");
    expect(html).toContain("log__tag--appending");
    expect(html).toContain("build.log");
  });

  it("marks the newest event as the live 'now' row when the agent is working", () => {
    const events: FileEvent[] = [
      { path: "/a/now.ts", action: "editing", at: 0 },
      { path: "/a/past.ts", action: "reading", at: 0 },
    ];
    const live = renderFileLog(events, true, 0);
    expect(live).toContain("log__row--now");
    expect(live).toContain("log__row--past");

    // Idle agent has no present tense — every row is past.
    const idle = renderFileLog(events, false, 0);
    expect(idle).not.toContain("log__row--now");
  });

  it("escapes file names to prevent HTML injection", () => {
    const events: FileEvent[] = [
      { path: "/x/<img src=x onerror=alert(1)>.ts", action: "reading", at: 0 },
    ];
    const html = renderFileLog(events, false, 0);
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;img");
  });
});
