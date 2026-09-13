//! Byte offset (PCRE2) <-> UTF-16 code unit offset (frontend/JS) conversion.
//!
//! The browser worker (JS flavor) reports match offsets in UTF-16 code units,
//! and the frontend uses `str.substr(match.i, match.l)` plus
//! `match.l === test.text.length` (UTF-16 semantics) to render and judge
//! results. We therefore always emit UTF-16 offsets — this is *more* correct
//! for the UI than the PHP backend, which emitted UTF-8 character counts.

/// One-pass prefix table: byte offsets of every char boundary mapped to their
/// UTF-16 code unit offsets. Building it is O(n) over the subject; each
/// lookup is O(log n) via binary search — a solve with 20k matches over a
/// large subject stays O(n + m) instead of rescanning from byte 0 per match.
pub struct Utf16Indexer {
    /// Parallel arrays: `bytes[i]` is a char-boundary byte offset,
    /// `units[i]` its UTF-16 offset. First entry is always (0, 0), last is
    /// (subject.len(), utf16_len).
    bytes: Vec<u32>,
    units: Vec<u32>,
}

impl Utf16Indexer {
    pub fn new(s: &str) -> Self {
        // A 1 MB subject can hold ~1M char boundaries; u32 covers 4G bytes /
        // 4G units, far beyond the request-size limits in config.rs.
        let mut bytes = Vec::with_capacity(s.len() + 1);
        let mut units = Vec::with_capacity(s.len() + 1);
        bytes.push(0);
        units.push(0);
        let mut u: usize = 0;
        // each char closes one boundary: entry (b + len, units up to and
        // including this char)
        for (b, c) in s.char_indices() {
            u += c.len_utf16();
            bytes.push((b + c.len_utf8()) as u32);
            units.push(u as u32);
        }
        Utf16Indexer { bytes, units }
    }

    /// UTF-16 offset for a byte index (must fall on a char boundary).
    pub fn utf16_at(&self, byte_idx: usize) -> usize {
        let i = self.bytes.partition_point(|&b| b < byte_idx as u32);
        debug_assert_eq!(
            self.bytes[i], byte_idx as u32,
            "byte_idx not on a char boundary"
        );
        self.units[i] as usize
    }

    /// Total length of the subject in UTF-16 code units.
    pub fn len(&self) -> usize {
        *self.units.last().unwrap_or(&0) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Convert a byte index into `s` (must fall on a char boundary) to a UTF-16
/// code unit index. Single-shot convenience version: O(byte_idx). For many
/// lookups on the same subject, build a [`Utf16Indexer`] instead.
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

    #[test]
    fn indexer_matches_one_shot_version() {
        let s = "ab\u{1F600}中def\u{1F600}";
        let ix = Utf16Indexer::new(s);
        for (b, _) in s.char_indices() {
            assert_eq!(ix.utf16_at(b), byte_to_utf16(s, b), "at byte {b}");
        }
        assert_eq!(ix.utf16_at(s.len()), utf16_len(s));
        assert_eq!(ix.len(), utf16_len(s));
    }
}
