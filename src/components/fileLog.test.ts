import { describe, expect, it } from "vitest";
import { renderFileLog } from "./fileLog";
import type { FileEvent } from "../types";

describe("renderFileLog", () => {
  it("shows an empty state when there is no activity", () => {
    expect(renderFileLog([])).toContain("no file activity");
  });

  it("renders a status tag and file name per event", () => {
    const events: FileEvent[] = [
      { path: "/a/b/styles.css", action: "editing", at: 0 },
      { path: "build.log", action: "appending", at: 0 },
    ];
    const html = renderFileLog(events);
    expect(html).toContain("editing");
    expect(html).toContain("styles.css");
    expect(html).toContain("appending");
    expect(html).toContain("build.log");
    expect(html).toContain("file-tag--editing");
  });

  it("escapes file names to prevent HTML injection", () => {
    const events: FileEvent[] = [
      { path: "/x/<img src=x onerror=alert(1)>.ts", action: "reading", at: 0 },
    ];
    const html = renderFileLog(events);
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;img");
  });
});
