import type { AgentSession, ContextCategory, ContextWindow } from "../types";
import { escapeHtml, formatPct, formatTokens } from "./format";

const categoryLabels: Record<ContextCategory, string> = {
  systemInstructions: "system",
  toolDefinitions: "tools",
  memory: "memory",
  fileReads: "files",
  conversation: "chat",
  other: "other",
};

/**
 * Color zone for an occupancy percentage. Aligned with Claude Code's own
 * ~95% auto-compaction behavior: green well under, amber/orange as it climbs,
 * red once compaction is imminent. `null` (unknown limit) gets a neutral zone.
 */
function zone(pct: number | null): string {
  if (pct == null) return "unknown";
  if (pct < 60) return "ok";
  if (pct < 80) return "warn";
  if (pct < 90) return "high";
  return "crit";
}

/**
 * The glanceable header gauge: a thin occupancy bar plus a `used / limit` label.
 * Falls back to the plain spend total when no context data is available, so a
 * session without usage still renders something familiar.
 */
export function renderContextBar(session: AgentSession): string {
  const ctx = session.context;
  if (!ctx || ctx.used <= 0) {
    const total =
      session.tokens.input + session.tokens.output + session.tokens.cache;
    return `<span class="strip__tokens-num" title="total tokens">${formatTokens(total)}</span>`;
  }

  const pct = ctx.fillPct;
  const z = zone(pct);
  // The bar clamps at 100% for display, but the raw number (which can exceed
  // the limit just after a compaction) is preserved in the tooltip.
  const width = pct == null ? 100 : Math.min(100, Math.max(0, pct));
  const limitLabel = ctx.limit != null ? formatTokens(ctx.limit) : "—";
  const pctText =
    pct == null
      ? "limit unknown"
      : `${formatPct(Math.min(100, pct))}${pct > 100 ? " (over limit)" : ""}`;
  const title = `context ${ctx.used.toLocaleString()} / ${
    ctx.limit != null ? ctx.limit.toLocaleString() : "unknown"
  } tokens · ${pctText}`;

  return `
    <span class="ctx" data-zone="${z}" title="${escapeHtml(title)}">
      <span class="ctx__bar"><span class="ctx__fill" style="width:${width.toFixed(
        1,
      )}%"></span></span>
      <span class="ctx__label">${formatTokens(ctx.used)} / ${limitLabel}</span>
    </span>
  `;
}

/** A 220×32 sawtooth of per-turn occupancy; the drop after a compaction is the
 *  whole point, so no axes or smoothing — just the shape. */
function renderSparkline(ctx: ContextWindow): string {
  if (ctx.history.length < 2) return "";
  const w = 220;
  const h = 32;
  const pad = 2;
  const span = ctx.history.length - 1;
  const maxUsed = Math.max(...ctx.history.map((p) => p.used), 1);
  const y = (used: number) => h - pad - (used / maxUsed) * (h - 2 * pad);
  const points = ctx.history
    .map((p, i) => {
      const x = pad + (i / span) * (w - 2 * pad);
      return `${x.toFixed(1)},${y(p.used).toFixed(1)}`;
    })
    .join(" ");
  return `<svg class="ctx-spark" viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" aria-hidden="true"><polyline points="${points}" fill="none" stroke="currentColor" stroke-width="1.5" /></svg>`;
}

/**
 * The expanded breakdown: a segmented bar of what's occupying the window, a
 * legend with token counts, the cached/fresh split, the sawtooth, and a
 * compaction marker. Returns empty string when there's nothing to show.
 */
export function renderContextDetail(ctx: ContextWindow | null): string {
  if (!ctx || ctx.used <= 0) return "";

  const segments = ctx.categories
    .map((c) => {
      const pct = (c.tokens / ctx.used) * 100;
      const label = `${categoryLabels[c.name]} ${formatTokens(c.tokens)}${
        c.estimated ? " (est)" : ""
      }`;
      return `<span class="ctx-seg" data-cat="${c.name}" style="width:${pct.toFixed(
        2,
      )}%" title="${escapeHtml(label)}"></span>`;
    })
    .join("");

  const legend = ctx.categories
    .map(
      (c) =>
        `<li data-cat="${c.name}"><i class="ctx-dot" aria-hidden="true"></i><span>${
          categoryLabels[c.name]
        }${c.estimated ? "*" : ""}</span><b>${formatTokens(c.tokens)}</b></li>`,
    )
    .join("");

  const compaction = ctx.compactions.length
    ? `<span class="ctx-compact">↘ compacted${
        ctx.compactions.every((c) => !c.explicit) ? " (inferred)" : ""
      }</span>`
    : "";
  const estNote = ctx.categories.some((c) => c.estimated)
    ? `<span class="ctx-note">* estimated</span>`
    : "";

  return `
    <div class="ctx-detail">
      <div class="ctx-detail__head">
        <span>context window</span>
        <span class="ctx-detail__split">${formatTokens(
          ctx.cached,
        )} cached · ${formatTokens(ctx.fresh)} fresh</span>
      </div>
      <div class="ctx-seg-bar">${segments}</div>
      <ul class="ctx-legend">${legend}</ul>
      ${renderSparkline(ctx)}
      <div class="ctx-detail__foot">${compaction}${estNote}</div>
    </div>
  `;
}
