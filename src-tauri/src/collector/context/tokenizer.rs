//! The token counter behind the Layer-2 context breakdown.
//!
//! Uses `tiktoken-rs`' `o200k_base` encoding — the BPE vocabulary is embedded in
//! the binary at compile time, so counting is fully offline (no network, no API
//! key). It is exact for Codex/OpenAI text and a close proxy for Claude; that
//! proxy drift never surfaces at the top level because the breakdown is
//! normalized against the provider's authoritative occupancy total (see
//! [`super::build`]). The encoder is built once behind a `OnceLock` (the
//! construction cost is paid on first use and amortized to nothing thereafter).

use std::sync::OnceLock;

use tiktoken_rs::{o200k_base, CoreBPE};

static ENCODER: OnceLock<CoreBPE> = OnceLock::new();

/// The shared `o200k_base` encoder, built on first call. The `.expect` is on an
/// asset embedded in the binary by `tiktoken-rs`; it cannot fail at runtime, and
/// isolating it here means a hypothetical failure can never panic mid-parse.
fn encoder() -> &'static CoreBPE {
    ENCODER.get_or_init(|| o200k_base().expect("o200k_base is bundled with tiktoken-rs"))
}

/// Count the tokens in `text`. Panic-free, with a fast path for empty input.
pub fn count_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    encoder().encode_with_special_tokens(text).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_zero() {
        assert_eq!(count_tokens(""), 0);
    }

    #[test]
    fn counts_grow_with_text() {
        let short = count_tokens("hello world");
        let long = count_tokens(&"hello world ".repeat(50));
        assert!(short > 0);
        assert!(long > short);
    }
}
