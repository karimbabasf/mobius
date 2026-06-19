import type { FileAction, FileEvent } from "../types";
import { basename, escapeHtml } from "./format";

const actionLabels: Record<FileAction, string> = {
  reading: "reading",
  writing: "writing",
  editing: "editing",
  appending: "appending",
  running: "running",
  searching: "searching",
};

export function renderFileLog(events: FileEvent[]): string {
  if (events.length === 0) {
    return `<p class="file-log__empty">no file activity yet</p>`;
  }

  const rows = events
    .map((event) => {
      const label = actionLabels[event.action];
      // "running" carries a shell command, not a path — show it whole; others basename.
      const name = event.action === "running" ? event.path : basename(event.path);
      return `<li class="file-row"><span class="file-tag file-tag--${event.action}">${escapeHtml(
        label,
      )}</span><span class="file-name">${escapeHtml(name)}</span></li>`;
    })
    .join("");

  return `<ul class="file-log">${rows}</ul>`;
}
