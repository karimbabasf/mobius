import type { FileAction, FileEvent } from "../types";
import { basename, escapeHtml } from "./format";

const actionLabels: Record<FileAction, string> = {
  reading: "read",
  writing: "write",
  editing: "edit",
  appending: "append",
  running: "run",
  searching: "search",
};

/**
 * The activity timeline. Events arrive newest-first, so the first row is the
 * agent's most recent step. When the agent is `live` (working), that row is the
 * present tense — highlighted with a caret; everything below it is past tense,
 * dimmed. When the agent is idle there is no "now", so every row reads as past.
 */
export function renderFileLog(events: FileEvent[], live: boolean): string {
  if (events.length === 0) {
    return `<p class="log__empty">no activity recorded yet</p>`;
  }

  const rows = events
    .map((event, index) => {
      const label = actionLabels[event.action];
      // "running" carries a shell command, not a path — show it whole; others basename.
      const name = event.action === "running" ? event.path : basename(event.path);
      const isNow = live && index === 0;
      const rowClass = isNow ? "log__row log__row--now" : "log__row log__row--past";
      const caret = isNow
        ? `<span class="log__caret" aria-hidden="true">█</span>`
        : "";
      return `<li class="${rowClass}">
        <span class="log__tag log__tag--${event.action}">${escapeHtml(label)}</span>
        <span class="log__name">${escapeHtml(name)}${caret}</span>
        <span class="log__time" data-at="${event.at}"></span>
      </li>`;
    })
    .join("");

  return `<ul class="log__list">${rows}</ul>`;
}
