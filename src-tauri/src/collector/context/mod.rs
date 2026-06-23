//! Context-window reconstruction shared by the Claude and Codex adapters.
//!
//! The adapters walk their transcripts once and feed two things into here:
//!   * [`OccupancyRaw`] — Layer 1 ground truth captured from the *last* model
//!     call (occupancy, cached split, and the limit).
//!   * a [`SegmentAccumulator`] — the per-category text used for the Layer-2
//!     breakdown.
//! [`build`] then tokenizes each category, normalizes the counts so they sum
//! exactly to the authoritative `used` total, and assembles a [`ContextWindow`].

pub mod tokenizer;

use crate::collector::session::{
    CategorySlice, Compaction, ContextCategory, ContextSnapshot, ContextWindow, LimitSource,
};
use tokenizer::count_tokens;

/// Layer-1 occupancy facts, overwritten as the parser sees each model call so the
/// final value reflects the most recent prompt.
#[derive(Clone, Debug, Default)]
pub struct OccupancyRaw {
    pub used: u64,
    pub cached: u64,
    pub limit: Option<u64>,
    pub limit_source: LimitSource,
}

/// Per-category text buffers filled during a single transcript parse. `Other` is
/// residual-only and has no buffer.
#[derive(Default)]
pub struct SegmentAccumulator {
    system: String,
    tool_defs: String,
    tool_defs_set: bool,
    memory: String,
    file_reads: String,
    conversation: String,
}

impl SegmentAccumulator {
    /// Append a chunk of transcript text to a category bucket.
    pub fn push(&mut self, cat: ContextCategory, text: &str) {
        if text.is_empty() {
            return;
        }
        let buf = match cat {
            ContextCategory::SystemInstructions => &mut self.system,
            ContextCategory::ToolDefinitions => &mut self.tool_defs,
            ContextCategory::Memory => &mut self.memory,
            ContextCategory::FileReads => &mut self.file_reads,
            ContextCategory::Conversation => &mut self.conversation,
            // `Other` is whatever normalization can't attribute; never written here.
            ContextCategory::Other => return,
        };
        buf.push_str(text);
        buf.push('\n');
    }

    /// Record the tool-definition schema text exactly once (it's sent once per
    /// session, so later turns must not re-count it).
    pub fn set_tool_defs_once(&mut self, text: &str) {
        if !self.tool_defs_set && !text.is_empty() {
            self.tool_defs = text.to_string();
            self.tool_defs_set = true;
        }
    }

    /// Total tokens across every category buffer.
    ///
    /// Adapters whose provider does *not* report an occupancy figure (Hermes —
    /// `messages.token_count` is always NULL) tokenize the in-context transcript
    /// themselves and use this sum as the authoritative `used`. Passing this same
    /// value as [`OccupancyRaw::used`] makes [`build`]'s normalization an identity
    /// (`scale == 1`), so the Layer-2 breakdown sums to it exactly.
    pub fn total_tokens(&self) -> u64 {
        (count_tokens(&self.system)
            + count_tokens(&self.tool_defs)
            + count_tokens(&self.memory)
            + count_tokens(&self.file_reads)
            + count_tokens(&self.conversation)) as u64
    }
}

/// Static context-window limit for a Claude model. Claude transcripts never carry
/// the limit, so it's resolved from the model name (all current models are 200k;
/// the 1M context is API-beta only and does not appear in Claude Code sessions).
pub fn claude_limit(model: &str) -> Option<u64> {
    if model.to_ascii_lowercase().contains("claude") {
        Some(200_000)
    } else {
        None
    }
}

/// Static fallback limit for a Codex model, used only when a `token_count` event
/// did not carry `model_context_window`.
pub fn codex_limit(model: &str) -> Option<u64> {
    let m = model.to_ascii_lowercase();
    if m.contains("codex-spark") {
        Some(128_000)
    } else if m.contains("gpt-5") {
        Some(272_000)
    } else {
        None
    }
}

/// A representative token cost for Claude Code's built-in tool definitions, which
/// the transcript does not serialize. Flagged `estimated` in the breakdown so the
/// UI can mark it as approximate rather than measured.
pub const CLAUDE_TOOLDEF_ESTIMATE: u64 = 13_000;

/// Assemble a [`ContextWindow`] from Layer-1 occupancy and Layer-2 segments.
///
/// When `enable_layer2` is set and there is occupancy to attribute, each category
/// is tokenized and scaled by `used / raw_sum` (floored), with the leftover folded
/// into `Other`. This guarantees `Σ categories == used` exactly while keeping the
/// proportions faithful to the tokenized text. `tooldef_estimate` (when non-zero)
/// substitutes for the tool-definition bucket, which Claude can't tokenize.
pub fn build(
    occ: OccupancyRaw,
    seg: &SegmentAccumulator,
    mut history: Vec<ContextSnapshot>,
    compactions: Vec<Compaction>,
    tooldef_estimate: u64,
    enable_layer2: bool,
) -> ContextWindow {
    let fresh = occ.used.saturating_sub(occ.cached);
    let fill_pct = occ.limit.and_then(|limit| {
        if limit == 0 {
            None
        } else {
            Some(occ.used as f32 / limit as f32 * 100.0)
        }
    });

    let mut categories = Vec::new();
    let mut residual = 0;
    if enable_layer2 && occ.used > 0 {
        let tool_tokens = if tooldef_estimate > 0 {
            tooldef_estimate
        } else {
            count_tokens(&seg.tool_defs) as u64
        };
        let raw: [(ContextCategory, u64, bool); 5] = [
            (
                ContextCategory::SystemInstructions,
                count_tokens(&seg.system) as u64,
                false,
            ),
            (
                ContextCategory::ToolDefinitions,
                tool_tokens,
                tooldef_estimate > 0,
            ),
            (
                ContextCategory::Memory,
                count_tokens(&seg.memory) as u64,
                false,
            ),
            (
                ContextCategory::FileReads,
                count_tokens(&seg.file_reads) as u64,
                false,
            ),
            (
                ContextCategory::Conversation,
                count_tokens(&seg.conversation) as u64,
                false,
            ),
        ];
        let raw_sum: u64 = raw.iter().map(|(_, t, _)| *t).sum();

        let mut scaled_sum = 0u64;
        if raw_sum > 0 {
            // Floor every scaled count so the running sum never exceeds `used`;
            // the remainder becomes `Other`, keeping the total exact.
            let scale = occ.used as f64 / raw_sum as f64;
            for (cat, tokens, estimated) in raw {
                if tokens == 0 {
                    continue;
                }
                let scaled = (tokens as f64 * scale).floor() as u64;
                if scaled == 0 {
                    continue;
                }
                scaled_sum += scaled;
                categories.push(CategorySlice {
                    name: cat,
                    tokens: scaled,
                    estimated,
                });
            }
        }
        residual = occ.used.saturating_sub(scaled_sum);
        if residual > 0 {
            categories.push(CategorySlice {
                name: ContextCategory::Other,
                tokens: residual,
                estimated: false,
            });
        }
    }

    if history.len() > 20 {
        let drop = history.len() - 20;
        history.drain(0..drop);
    }

    ContextWindow {
        used: occ.used,
        limit: occ.limit,
        fill_pct,
        limit_source: occ.limit_source,
        cached: occ.cached,
        fresh,
        categories,
        residual,
        history,
        compactions,
    }
}

/// Parse an RFC3339/ISO-8601 UTC timestamp ("2026-06-19T10:00:05.000Z") to epoch
/// milliseconds. Returns `None` for anything it doesn't recognize, so callers fall
/// back to the file mtime. Assumes UTC (`Z`); a trailing offset is ignored.
pub fn parse_iso8601_ms(s: &str) -> Option<i64> {
    if s.len() < 19 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    let month: i64 = s.get(5..7)?.parse().ok()?;
    let day: i64 = s.get(8..10)?.parse().ok()?;
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    let minute: i64 = s.get(14..16)?.parse().ok()?;
    let second: i64 = s.get(17..19)?.parse().ok()?;

    let mut millis = 0i64;
    if s.as_bytes().get(19) == Some(&b'.') {
        let frac: String = s[20..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .take(3)
            .collect();
        if !frac.is_empty() {
            millis = format!("{frac:0<3}").parse().unwrap_or(0);
        }
    }

    // days-from-civil (Howard Hinnant): convert the calendar date to a day count
    // relative to the Unix epoch, then add the time-of-day.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let secs = days * 86400 + hour * 3600 + minute * 60 + second;
    Some(secs * 1000 + millis)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occ(used: u64, cached: u64, limit: Option<u64>) -> OccupancyRaw {
        OccupancyRaw {
            used,
            cached,
            limit,
            limit_source: LimitSource::Reported,
        }
    }

    #[test]
    fn fill_pct_and_fresh_are_derived() {
        let cw = build(
            occ(50_000, 40_000, Some(200_000)),
            &SegmentAccumulator::default(),
            vec![],
            vec![],
            0,
            false,
        );
        assert_eq!(cw.fresh, 10_000);
        assert!((cw.fill_pct.unwrap() - 25.0).abs() < 0.01);
    }

    #[test]
    fn unknown_limit_yields_no_pct() {
        let cw = build(
            occ(50_000, 0, None),
            &SegmentAccumulator::default(),
            vec![],
            vec![],
            0,
            false,
        );
        assert!(cw.fill_pct.is_none());
    }

    #[test]
    fn categories_sum_exactly_to_used() {
        let mut seg = SegmentAccumulator::default();
        seg.push(
            ContextCategory::SystemInstructions,
            "you are a helpful assistant",
        );
        seg.push(
            ContextCategory::Conversation,
            &"some conversation text ".repeat(40),
        );
        seg.push(
            ContextCategory::FileReads,
            &"file contents here ".repeat(80),
        );
        let cw = build(
            occ(120_000, 90_000, Some(200_000)),
            &seg,
            vec![],
            vec![],
            13_000,
            true,
        );
        let sum: u64 = cw.categories.iter().map(|c| c.tokens).sum();
        assert_eq!(
            sum, cw.used,
            "categories must reconcile to the authoritative total"
        );
        assert!(cw
            .categories
            .iter()
            .any(|c| matches!(c.name, ContextCategory::ToolDefinitions) && c.estimated));
    }

    #[test]
    fn history_caps_at_twenty_keeping_newest() {
        let history: Vec<ContextSnapshot> = (0..30)
            .map(|i| ContextSnapshot {
                at: i,
                used: i as u64,
            })
            .collect();
        let cw = build(
            occ(1, 0, None),
            &SegmentAccumulator::default(),
            history,
            vec![],
            0,
            false,
        );
        assert_eq!(cw.history.len(), 20);
        assert_eq!(cw.history.first().unwrap().at, 10);
        assert_eq!(cw.history.last().unwrap().at, 29);
    }

    #[test]
    fn iso8601_parses_to_epoch_ms() {
        // 2026-06-19T10:00:05.000Z — verified against a known epoch.
        let ms = parse_iso8601_ms("2026-06-19T10:00:05.000Z").unwrap();
        assert_eq!(ms, 1_781_863_205_000);
        // fractional millis honored
        let a = parse_iso8601_ms("2026-06-19T10:00:05.250Z").unwrap();
        assert_eq!(a - ms, 250);
        assert!(parse_iso8601_ms("not a date").is_none());
    }

    #[test]
    fn claude_and_codex_limits() {
        assert_eq!(claude_limit("claude-opus-4-8"), Some(200_000));
        assert_eq!(claude_limit("gpt-5.5"), None);
        assert_eq!(codex_limit("gpt-5.5"), Some(272_000));
        assert_eq!(codex_limit("gpt-5.3-codex-spark"), Some(128_000));
    }
}
