import { describe, expect, it } from "vitest";
import { renderToolLogo } from "./toolLogo";

describe("renderToolLogo", () => {
  it("returns a distinct svg mark per tool", () => {
    const claude = renderToolLogo("claude");
    const codex = renderToolLogo("codex");

    expect(claude).toContain("<svg");
    expect(claude).toContain("tool-logo--claude");
    expect(codex).toContain("tool-logo--codex");
    expect(claude).not.toEqual(codex);
  });
});
