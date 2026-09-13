//! PCRE substitution-string expansion for the `replace` / `list` tools.
//!
//! Follows the PHP `preg_replace()` replacement semantics, verified against
//! real PHP 8.x (Docker对拍, see scripts/gen-fixtures.php):
//! - `\1`..`\99`, `$1`..`$99`, `${n}` capture-group references (up to two
//!   digits parsed for the bare forms)
//! - `\\` -> literal `\`, `\$` -> literal `$`
//! - a reference to a group that exists but did not participate -> empty string
//! - a reference to a group number larger than the pattern's group count ->
//!   empty string. PHP does NOT decompose (`$13` with one group is empty, not
//!   `$1` + `"3"` — that is the JS rule; the frontend PCRE profile also sets
//!   `substdecomposeref: false`).
//! - `$` / `\` followed by anything else: emitted literally (backslash kept).
//!
//! `$&`, `` $` ``, `$'`, `$$` are intentionally NOT implemented: the frontend
//! PCRE profile (`dev/src/profiles/pcre.js`) disables them, so they never
//! reach the server in valid input. `\g{...}` is also not a preg_replace
//! replacement form and stays literal.

/// Expand the replacement string.
///
/// * `total_groups`: number of capturing groups in the pattern.
/// * `lookup(n)`: returns the captured text for group `n` (1-based),
///   `None` if the group exists but did not participate in the match.
pub fn expand<'a>(
    repl: &str,
    total_groups: usize,
    lookup: impl Fn(usize) -> Option<&'a str>,
) -> String {
    let mut out = String::with_capacity(repl.len());
    let chars: Vec<char> = repl.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c != '\\' && c != '$' {
            out.push(c);
            i += 1;
            continue;
        }

        let next = chars.get(i + 1).copied();

        match (c, next) {
            (_, Some('\\')) if c == '\\' => {
                out.push('\\');
                i += 2;
            }
            (_, Some('$')) if c == '\\' => {
                out.push('$');
                i += 2;
            }
            ('$', Some('{')) => {
                // ${n} form; if malformed, emit literally.
                match parse_braced(&chars, i + 2) {
                    Some((n, end)) => {
                        emit_group(&mut out, n, total_groups, &lookup);
                        i = end;
                    }
                    None => {
                        out.push('$');
                        i += 1;
                    }
                }
            }
            (_, Some(d)) if c == '$' && d.is_ascii_digit() => {
                let (n, consumed) = parse_number(&chars, i + 1, 2);
                emit_group(&mut out, n, total_groups, &lookup);
                i += 1 + consumed;
            }
            (_, Some(d)) if c == '\\' && d.is_ascii_digit() => {
                let (n, consumed) = parse_number(&chars, i + 1, 2);
                emit_group(&mut out, n, total_groups, &lookup);
                i += 1 + consumed;
            }
            _ => {
                // Keep the escape character literally (`\q`, trailing `\`, `$`...).
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

fn parse_braced(chars: &[char], start: usize) -> Option<(usize, usize)> {
    let mut j = start;
    while j < chars.len() && chars[j].is_ascii_digit() && j - start < 9 {
        j += 1;
    }
    if j == start || j >= chars.len() || chars[j] != '}' {
        return None;
    }
    let n: String = chars[start..j].iter().collect();
    let n: usize = n.parse().ok()?;
    Some((n, j + 1))
}

/// Parse up to `max_digits` ASCII digits starting at `start`; returns the
/// number and how many digits were consumed.
fn parse_number(chars: &[char], start: usize, max_digits: usize) -> (usize, usize) {
    let mut n: usize = 0;
    let mut consumed = 0;
    while consumed < max_digits
        && start + consumed < chars.len()
        && chars[start + consumed].is_ascii_digit()
    {
        n = n * 10 + (chars[start + consumed] as usize - '0' as usize);
        consumed += 1;
    }
    (n, consumed)
}

fn emit_group<'a>(
    out: &mut String,
    n: usize,
    total_groups: usize,
    lookup: &impl Fn(usize) -> Option<&'a str>,
) {
    if n > total_groups {
        // PHP: a reference past the pattern's group count expands to nothing
        // (verified: `$13` with 1 group -> "", no decomposition, no literal).
        return;
    }
    if let Some(t) = lookup(n) {
        out.push_str(t);
    }
    // exists but unmatched -> empty string
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_passthrough() {
        assert_eq!(expand("plain", 0, |_| -> Option<&str> { None }), "plain");
    }

    #[test]
    fn backslash_forms() {
        assert_eq!(expand("a\\\\b", 0, |_| None), "a\\b");
        assert_eq!(expand("a\\$1", 0, |_| None), "a$1");
        assert_eq!(expand("a\\q", 0, |_| None), "a\\q");
    }

    #[test]
    fn dollar_and_backslash_refs() {
        let lookup = |n: usize| if n == 1 { Some("A") } else { None };
        assert_eq!(expand("\\1", 1, lookup), "A");
        assert_eq!(expand("$1", 1, lookup), "A");
        assert_eq!(expand("${1}", 1, lookup), "A");
        assert_eq!(expand("$1$1", 1, lookup), "AA");
    }

    #[test]
    fn unmatched_existing_group_is_empty() {
        let lookup = |n: usize| if n == 1 { Some("A") } else { None };
        assert_eq!(expand("[\\2]", 2, lookup), "[]");
    }

    #[test]
    fn nonexistent_group_is_empty() {
        // PHP (verified against real PHP 8): a reference past the group count
        // expands to nothing — no decomposition, no literal echo.
        let lookup = |n: usize| if n == 1 { Some("A") } else { None };
        assert_eq!(expand("$13", 1, lookup), "");
        assert_eq!(expand("$9", 1, lookup), "");
        assert_eq!(expand("\\12", 1, lookup), "");
        assert_eq!(expand("${13}", 1, lookup), "");
        // two digits parse as one reference when the group exists
        let lookup13 = |n: usize| if n == 13 { Some("X") } else { None };
        assert_eq!(expand("$13", 15, lookup13), "X");
        // two digits when only a 1-digit group exists: nothing (not $1+"3")
        assert_eq!(expand("$12", 1, lookup), "");
    }

    #[test]
    fn dollar_without_digits_is_literal() {
        assert_eq!(expand("$ x", 0, |_| None), "$ x");
        assert_eq!(expand("cost: $", 0, |_| None), "cost: $");
    }
}
