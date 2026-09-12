//! PCRE substitution-string expansion for the `replace` / `list` tools.
//!
//! Follows the PHP `preg_replace()` replacement semantics that the original
//! backend implemented:
//! - `\1`..`\99`, `$1`..`$99`, `${n}` capture-group references
//! - `\\` -> literal `\`, `\$` -> literal `$`
//! - a reference to a group that exists but did not participate -> empty string
//! - a reference to a group number larger than the pattern's group count:
//!   try the shorter number (first digit) and emit the leftover digit(s)
//!   literally (PHP's documented `$13` -> `$1` + `"3"` rule); if even the
//!   shorter form is invalid, emit the token literally.
//! - `$` / `\` followed by anything else: emitted literally (backslash kept).
//!
//! `$&`, `` $` ``, `$'`, `$$` are intentionally NOT implemented: the frontend
//! PCRE profile (`dev/src/profiles/pcre.js`) disables them, so they never
//! reach the server in valid input.

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
                        emit_group(&mut out, n, total_groups, &lookup, &format!("${{{n}}}"));
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
                emit_group(
                    &mut out,
                    n,
                    total_groups,
                    &lookup,
                    &chars[i..i + 1 + consumed].iter().collect::<String>(),
                );
                i += 1 + consumed;
            }
            (_, Some(d)) if c == '\\' && d.is_ascii_digit() => {
                let (n, consumed) = parse_number(&chars, i + 1, 2);
                emit_group(
                    &mut out,
                    n,
                    total_groups,
                    &lookup,
                    &chars[i..i + 1 + consumed].iter().collect::<String>(),
                );
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

fn digits_of(mut n: usize) -> usize {
    let mut d = 1;
    while n >= 10 {
        n /= 10;
        d += 1;
    }
    d
}

fn emit_group<'a>(
    out: &mut String,
    n: usize,
    total_groups: usize,
    lookup: &impl Fn(usize) -> Option<&'a str>,
    literal: &str,
) {
    if n > total_groups {
        // Try PHP's fallback: strip trailing digits until the remaining group
        // number is valid; otherwise emit the token literally.
        let nd = digits_of(n);
        if nd > 1 {
            let lead = n / 10usize.pow((nd - 1) as u32);
            if lead != 0 && lead <= total_groups {
                if let Some(t) = lookup(lead) {
                    out.push_str(t);
                }
                // leftover digits were part of `literal`; re-emit them
                let leftover = &literal[literal.len() - (nd - digits_of(lead))..];
                out.push_str(leftover);
                return;
            }
        }
        out.push_str(literal);
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
    fn nonexistent_group_fallback() {
        let lookup = |n: usize| if n == 1 { Some("A") } else { None };
        // $13 with 1 group -> group 1 + literal "3"
        assert_eq!(expand("$13", 1, lookup), "A3");
        // $9 with 1 group -> literal "$9"
        assert_eq!(expand("$9", 1, lookup), "$9");
        // \12 with 1 group -> "A2"
        assert_eq!(expand("\\12", 1, lookup), "A2");
    }

    #[test]
    fn dollar_without_digits_is_literal() {
        assert_eq!(expand("$ x", 0, |_| None), "$ x");
        assert_eq!(expand("cost: $", 0, |_| None), "cost: $");
    }
}
