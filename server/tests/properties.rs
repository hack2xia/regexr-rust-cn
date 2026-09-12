//! Property-based tests (proptest) for the two engine invariants that
//! hand-written fixtures cannot cover systematically:
//! 1. every match offset stays on a UTF-8 char boundary (guards the
//!    mid-character-offset panic class, see fixture 13),
//! 2. byte -> UTF-16 offset mapping is a bijection onto 0..=len (the
//!    frontend slices text with these offsets).

use proptest::prelude::*;

use regexr_server::solve::engine::CompiledRegex;
use regexr_server::solve::offsets::{byte_to_utf16, utf16_len};

/// Patterns that can match empty — the family that produced the
/// mid-character-offset bug — plus a couple of non-empty ones.
const PATTERNS: &[&str] = &["x*", "[0-9]*", "(a|)", "b?", "(?:)", "\\s*", ".*", "中*"];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn match_offsets_stay_on_char_boundaries_and_in_range(
        text in ".*",
        pattern in prop::sample::select(PATTERNS.to_vec()),
    ) {
        let re = CompiledRegex::compile(&pattern, "g").unwrap();
        // A limit error is a legitimate result for adversarial inputs;
        // the invariant below only concerns successful matches.
        let spans = match re.match_all(&text) {
            Ok(s) => s,
            Err(_) => return Ok(()),
        };
        for sp in &spans {
            prop_assert!(sp.start <= sp.end && sp.end <= text.len());
            prop_assert!(
                text.is_char_boundary(sp.start),
                "match start {} is mid-character in {text:?}",
                sp.start
            );
            prop_assert!(
                text.is_char_boundary(sp.end),
                "match end {} is mid-character in {text:?}",
                sp.end
            );
        }
    }

    #[test]
    fn utf16_offset_map_is_monotone_injective(s in ".*") {
        // Char boundaries map to strictly increasing UTF-16 offsets, from 0
        // to exactly utf16_len, with steps of 1 (BMP char) or 2 (astral
        // char). In particular distinct boundaries never collide, so the
        // frontend can invert any server-issued offset back to a boundary.
        let mut boundaries: Vec<usize> = s.char_indices().map(|(b, _)| b).collect();
        boundaries.push(s.len());
        let mapped: Vec<usize> = boundaries.iter().map(|&b| byte_to_utf16(&s, b)).collect();
        prop_assert_eq!(*mapped.first().unwrap_or(&0), 0);
        prop_assert_eq!(*mapped.last().unwrap_or(&0), utf16_len(&s));
        for w in mapped.windows(2) {
            let step = w[1] - w[0];
            prop_assert!(step == 1 || step == 2, "step {step} at utf16 offset {}", w[0]);
        }
    }
}
