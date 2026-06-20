export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function basename(path: string): string {
  const trimmed = path.replace(/\/+$/, "");
  return trimmed.split("/").pop() || trimmed || "unknown project";
}

export function formatTokens(total: number): string {
  if (total >= 1_000_000) {
    return `${(total / 1_000_000).toFixed(2)}M`;
  }
  if (total >= 1000) {
    return `${(total / 1000).toFixed(1)}k`;
  }
  return String(total);
}

/** Whole-percent label for a context-fill ratio. */
export function formatPct(pct: number): string {
  return `${Math.round(pct)}%`;
}

/** Compact, terminal-style relative time: "now", "8s", "3m", "2h", "1d". */
export function formatAgo(at: number, now: number): string {
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  if (seconds < 5) return "now";
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.round(hours / 24)}d`;
}
