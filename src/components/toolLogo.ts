import type { Tool } from "../types";

// Simple, original geometric marks (not the vendors' trademarked logos), tinted
// per tool by CSS. currentColor lets the card's data-tool theme drive the hue.
const marks: Record<Tool, string> = {
  claude:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M8 1.5v13M1.5 8h13M3.4 3.4l9.2 9.2M12.6 3.4l-9.2 9.2"/></svg>',
  codex:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"><path d="M8 1.3l5.8 3.35v6.7L8 14.7l-5.8-3.35v-6.7z"/></svg>',
  cursor:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="currentColor"><path d="M3 2l9.2 5.3-4.1.9-1.7 3.9z"/></svg>',
  hermes:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round"><path d="M8 2l5.5 9.5h-11z"/><path d="M8 6.5v4"/></svg>',
  // Generic mark for a scanner-detected process that isn't a first-class
  // provider: a radar blip, fitting the "caught something running" framing.
  agent:
    '<svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round"><circle cx="8" cy="8" r="2"/><circle cx="8" cy="8" r="6"/></svg>',
};

export function renderToolLogo(tool: Tool): string {
  return `<span class="tool-logo tool-logo--${tool}" aria-hidden="true">${marks[tool]}</span>`;
}
