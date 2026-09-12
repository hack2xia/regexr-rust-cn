//! Byte offset (PCRE2) <-> UTF-16 code unit offset (frontend/JS) conversion.
//!
//! The browser worker (JS flavor) reports match offsets in UTF-16 code units,
//! and the frontend uses `str.substr(match.i, match.l)` plus
//! `match.l === test.text.length` (UTF-16 semantics) to render and judge
//! results. We therefore always emit UTF-16 offsets — this is *more* correct
//! for the UI than the PHP backend, which emitted UTF-8 character counts.

/// Convert a byte index into `s` (must fall on a char boundary) to a UTF-16
/// code unit index.
pub fn byte_to_utf16(s: &str, byte_idx: usize) -> usize {
    s[..byte_idx].chars().map(char::len_utf16).sum()
}

/// Length of `s` in UTF-16 code units (same as `s.length` in JS).
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_identity() {
        let s = "abc def";
        assert_eq!(byte_to_utf16(s, 4), 4);
        assert_eq!(utf16_len(s), 7);
    }

    #[test]
    fn bmp_chars_are_one_unit() {
        // "中" is one UTF-16 unit, 3 bytes in UTF-8.
        let s = "中文abc";
        assert_eq!(byte_to_utf16(s, 3), 1);
        assert_eq!(byte_to_utf16(s, 9), 5);
        assert_eq!(utf16_len(s), 5);
    }

    #[test]
    fn astral_chars_are_two_units() {
        // U+1F600 (GRINNING FACE) is a surrogate pair: 2 units, 4 bytes.
        let s = "\u{1F600}x";
        assert_eq!(byte_to_utf16(s, 4), 2);
        assert_eq!(utf16_len(s), 3);
    }
}
