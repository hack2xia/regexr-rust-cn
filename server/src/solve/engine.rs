//! Thin pcre2-sys FFI engine: compile with hard limits + JIT, match with
//! preg_match_all-compatible global iteration, PHP-compatible replacement.
//!
//! All `unsafe` code is concentrated in this file for easy audit.

use std::ffi::c_int;

use pcre2_sys as sys;

use crate::config;
use crate::solve::errors::SolveError;

/// PCRE2 marks unset ovector entries with SIZE_MAX.
const PCRE2_UNSET: usize = usize::MAX;

/// A byte-span match with per-group spans (byte offsets into the subject).
#[derive(Debug)]
pub struct MatchSpan {
    pub start: usize,
    pub end: usize,
    /// Group 1..=N; `None` = group exists but did not participate.
    pub groups: Vec<Option<(usize, usize)>>,
}

pub struct CompiledRegex {
    code: *mut sys::pcre2_code_8,
    match_data: *mut sys::pcre2_match_data_8,
    mcontext: *mut sys::pcre2_match_context_8,
    jit_stack: *mut sys::pcre2_jit_stack_8,
    jit: bool,
    ngroups: usize,
    global: bool,
}

// PCRE2 code objects are safe for use from one thread at a time; every
// CompiledRegex owns its private match_data / mcontext / jit_stack and is
// created, used and dropped inside a single spawn_blocking task.
unsafe impl Send for CompiledRegex {}

fn pcre2_error_message(code: i32) -> String {
    unsafe {
        let mut buf = [0u8; 256];
        let n = sys::pcre2_get_error_message_8(code, buf.as_mut_ptr(), buf.len());
        let n = if n < 0 { 0 } else { n as usize };
        String::from_utf8_lossy(&buf[..n.min(buf.len())]).into_owned()
    }
}

impl CompiledRegex {
    /// Compile `pattern` with the flags string (regexr flavor, e.g. "gimsxu").
    /// Mirrors PHP: `g` is stripped and only controls global matching; any
    /// other unknown character becomes an "unknown modifier" compile error.
    pub fn compile(pattern: &str, flags: &str) -> Result<Self, SolveError> {
        if pattern.len() > config::MAX_PATTERN_BYTES {
            return Err(SolveError::RegexParse {
                message: "Compilation failed: pattern too long at offset 0".into(),
            });
        }

        let mut global = false;
        // Always-on options mirror what PHP's `u` modifier sets (php-src
        // php_pcre.c): UTF + UCP (so \w, \d, \s, \b use Unicode properties)
        // + NEVER_BACKSLASH_C (\C is unsafe in UTF mode).
        let mut options: u32 = sys::PCRE2_UTF | sys::PCRE2_UCP | sys::PCRE2_NEVER_BACKSLASH_C;
        for c in flags.chars() {
            options |= match c {
                'g' => {
                    global = true;
                    continue;
                }
                'i' => sys::PCRE2_CASELESS,
                'm' => sys::PCRE2_MULTILINE,
                'n' => sys::PCRE2_NO_AUTO_CAPTURE,
                's' => sys::PCRE2_DOTALL,
                'x' => sys::PCRE2_EXTENDED,
                'u' => sys::PCRE2_UTF, // always on
                'U' => sys::PCRE2_UNGREEDY,
                'A' => sys::PCRE2_ANCHORED,
                'D' => sys::PCRE2_DOLLAR_ENDONLY,
                'J' => sys::PCRE2_DUPNAMES,
                // Whitespace modifiers are ignored by PHP.
                ' ' | '\n' | '\r' => 0,
                'S' => 0, // PHP "study" hint: no-op
                // PHP keeps `X` as a recognized-but-ignored modifier since
                // PCRE2 dropped PCRE_EXTRA (php_pcre.c: `case 'X': /* Pass.
                // */ break;`). It is NOT PCRE2_EXTENDED_MORE ("xx").
                'X' => 0,
                other => {
                    // Exact wording of PHP's "Unknown modifier 'z'" warning —
                    // the frontend displays this message verbatim.
                    return Err(SolveError::RegexParse {
                        message: format!("Unknown modifier '{other}'"),
                    });
                }
            };
        }

        let mut error_code: c_int = 0;
        let mut error_offset: usize = 0;
        let bytes = pattern.as_bytes();
        let code = unsafe {
            sys::pcre2_compile_8(
                bytes.as_ptr(),
                bytes.len(),
                options,
                &mut error_code,
                &mut error_offset,
                std::ptr::null_mut(),
            )
        };
        if code.is_null() {
            return Err(SolveError::RegexParse {
                message: format!(
                    "Compilation failed: {} at offset {}",
                    pcre2_error_message(error_code),
                    error_offset
                ),
            });
        }

        unsafe {
            // Fail closed: without match_data the FFI match call would violate
            // its preconditions, and without a match context the anti-DoS
            // limits below could not be installed. Release everything we may
            // have allocated and bail out.
            let match_data =
                sys::pcre2_match_data_create_from_pattern_8(code, std::ptr::null_mut());
            let mcontext = sys::pcre2_match_context_create_8(std::ptr::null_mut());
            if match_data.is_null() || mcontext.is_null() {
                if !mcontext.is_null() {
                    sys::pcre2_match_context_free_8(mcontext);
                }
                if !match_data.is_null() {
                    sys::pcre2_match_data_free_8(match_data);
                }
                return Err(SolveError::Internal {
                    code: 0,
                    message: "PCRE2 allocation failed".into(),
                });
            }

            // Hard anti-DoS limits (see config.rs). These live on the
            // match context in PCRE2's API. A non-zero return means the
            // limit was NOT installed: refuse to compile rather than run
            // an unbounded regex.
            if sys::pcre2_set_match_limit_8(mcontext, config::PCRE2_MATCH_LIMIT) != 0
                || sys::pcre2_set_depth_limit_8(mcontext, config::PCRE2_DEPTH_LIMIT) != 0
            {
                sys::pcre2_match_context_free_8(mcontext);
                sys::pcre2_match_data_free_8(match_data);
                return Err(SolveError::Internal {
                    code: 0,
                    message: "failed to install PCRE2 match limits".into(),
                });
            }

            let jit_stack: *mut sys::pcre2_jit_stack_8 = sys::pcre2_jit_stack_create_8(
                config::JIT_STACK_START,
                config::JIT_STACK_MAX,
                std::ptr::null_mut(),
            );
            // JIT: failure is non-fatal, fall back to the interpreter. If the
            // JIT stack could not be created, force the interpreter too — a
            // JIT-compiled pattern without an assigned stack would fall back
            // per-call anyway, and this keeps the choice explicit.
            let jit_ok = sys::pcre2_jit_compile_8(code, sys::PCRE2_JIT_COMPLETE) >= 0;
            let jit = jit_ok && !jit_stack.is_null();
            if !jit_stack.is_null() {
                sys::pcre2_jit_stack_assign_8(mcontext, None, jit_stack as *mut _);
            }

            // ovector count is the number of (start,end) pairs = ngroups + 1.
            let pairs = sys::pcre2_get_ovector_count_8(match_data) as usize;

            Ok(CompiledRegex {
                code,
                match_data,
                mcontext,
                jit_stack,
                jit,
                ngroups: pairs.saturating_sub(1),
                global,
            })
        }
    }

    pub fn global(&self) -> bool {
        self.global
    }

    pub fn ngroups(&self) -> usize {
        self.ngroups
    }

    /// Run one match attempt at `start`. `Ok(true)` = matched.
    ///
    /// `options` are extra match options (e.g. `PCRE2_ANCHORED |
    /// PCRE2_NOTEMPTY_ATSTART` for the empty-match retry in `match_all`).
    fn exec(&self, subject: &[u8], start: usize, options: u32) -> Result<bool, SolveError> {
        // The JIT fast path only supports a subset of match-time options:
        // PCRE2_ANCHORED is honored by the interpreter (and the pcre2_match
        // wrapper would re-route to it), but pcre2_jit_match silently IGNORES
        // it, which would turn the empty-match retry into a forward scan. So
        // any anchored call must go to the interpreter.
        let use_jit = self.jit && (options & sys::PCRE2_ANCHORED) == 0;
        let rc = unsafe {
            if use_jit {
                sys::pcre2_jit_match_8(
                    self.code,
                    subject.as_ptr(),
                    subject.len(),
                    start,
                    options,
                    self.match_data,
                    self.mcontext,
                )
            } else {
                sys::pcre2_match_8(
                    self.code,
                    subject.as_ptr(),
                    subject.len(),
                    start,
                    options,
                    self.match_data,
                    self.mcontext,
                )
            }
        };
        if rc >= 0 {
            return Ok(true);
        }
        match rc {
            sys::PCRE2_ERROR_NOMATCH => Ok(false),
            sys::PCRE2_ERROR_MATCHLIMIT => Err(SolveError::MatchLimit),
            sys::PCRE2_ERROR_DEPTHLIMIT => Err(SolveError::DepthLimit),
            sys::PCRE2_ERROR_JIT_STACKLIMIT => Err(SolveError::JitStackLimit),
            rc if (sys::PCRE2_ERROR_UTF8_ERR21..=sys::PCRE2_ERROR_UTF8_ERR1).contains(&rc) => {
                Err(SolveError::BadUtf8)
            }
            _ => Err(SolveError::Internal {
                code: rc,
                message: pcre2_error_message(rc),
            }),
        }
    }

    /// Read the ovector of the last successful match (byte offsets).
    fn current_span(&self, subject: &[u8]) -> MatchSpan {
        unsafe {
            let pairs = sys::pcre2_get_ovector_count_8(self.match_data) as usize;
            let ovec = sys::pcre2_get_ovector_pointer_8(self.match_data);
            let group_at = |idx: usize| -> Option<(usize, usize)> {
                let s = *ovec.add(idx * 2);
                let e = *ovec.add(idx * 2 + 1);
                if s == PCRE2_UNSET || e == PCRE2_UNSET {
                    None
                } else {
                    Some((s, e))
                }
            };
            let (start, end) = group_at(0).unwrap_or((0, 0));
            debug_assert!(start <= subject.len() && end <= subject.len());
            MatchSpan {
                start,
                end,
                groups: (1..pairs).map(group_at).collect(),
            }
        }
    }

    /// Find all matches with explicit resource budgets.
    ///
    /// Global iteration follows the PCRE2-recommended algorithm (also what
    /// PHP's php_pcre.c does, and what the fixtures are generated with):
    /// after an empty match, retry at the *same* offset with
    /// `PCRE2_ANCHORED | PCRE2_NOTEMPTY_ATSTART`; only if that fails advance
    /// by exactly one UTF-8 character. This finds non-empty alternative
    /// branches at the same position (e.g. `(?:|a)`) that a plain
    /// advance-one-char loop would miss.
    ///
    /// * `max_matches = Some(n)`: stop scanning after `n` matches and report
    ///   `truncated = true` (never silently).
    /// * `max_matches = None`: scan everything, bounded only by
    ///   `MAX_CAPTURE_CELLS` (a `ResultTooLarge` error, not truncation).
    pub fn match_all_limited(
        &self,
        subject: &str,
        max_matches: Option<usize>,
    ) -> Result<(Vec<MatchSpan>, bool), SolveError> {
        let bytes = subject.as_bytes();
        let mut out: Vec<MatchSpan> = Vec::new();
        let mut start: usize = 0;
        let mut options: u32 = 0;

        loop {
            if start > bytes.len() {
                break;
            }
            if !self.exec(bytes, start, options)? {
                if options == 0 {
                    break; // no more matches at all
                }
                // The non-empty retry at the same offset failed: advance one
                // *character*, not one byte (PCRE2 accepts a mid-UTF-8
                // sequence start offset, but a non-boundary offset cannot be
                // converted to UTF-16 for the frontend; PHP's UTF mode
                // advances one char too), then resume normal matching.
                options = 0;
                let mut next = start + 1;
                while next < bytes.len() && !subject.is_char_boundary(next) {
                    next += 1;
                }
                start = next;
                continue;
            }
            let span = self.current_span(bytes);
            let is_empty = span.end == span.start;
            let span_end = span.end;

            if let Some(max) = max_matches {
                if out.len() >= max {
                    // Limit reached: stop scanning. Continuing would burn up
                    // to MAX_MATCHES..subject.len FFI calls that the result
                    // discards.
                    return Ok((out, true));
                }
            }
            // Cell budget: matches x (groups + full match). checked math so
            // a pathological pattern/text combination errors out instead of
            // attempting a gigabyte-scale allocation.
            let cells = (out.len() + 1)
                .checked_mul(self.ngroups.saturating_add(1))
                .ok_or(SolveError::ResultTooLarge)?;
            if cells > config::MAX_CAPTURE_CELLS {
                return Err(SolveError::ResultTooLarge);
            }
            out.push(span);

            if is_empty {
                // Empty match was recorded; retry non-empty at the SAME
                // offset next iteration (PCRE2-recommended algorithm, also
                // what PHP's php_pcre.c does). At end-of-subject the retry
                // can never match.
                if start == bytes.len() {
                    break;
                }
                options = sys::PCRE2_ANCHORED | sys::PCRE2_NOTEMPTY_ATSTART;
            } else {
                options = 0;
                start = span_end;
            }
        }
        Ok((out, false))
    }

    /// Find all matches under the default match-count cap.
    pub fn match_all(&self, subject: &str) -> Result<Vec<MatchSpan>, SolveError> {
        Ok(self
            .match_all_limited(subject, Some(config::MAX_MATCHES))?
            .0)
    }

    /// First match only (non-global semantics).
    pub fn match_one(&self, subject: &str) -> Result<Option<MatchSpan>, SolveError> {
        if self.exec(subject.as_bytes(), 0, 0)? {
            Ok(Some(self.current_span(subject.as_bytes())))
        } else {
            Ok(None)
        }
    }

    fn group_lookup<'a>(
        &'a self,
        subject: &'a str,
        span: &'a MatchSpan,
    ) -> impl Fn(usize) -> Option<&'a str> + 'a {
        move |n: usize| {
            if n == 0 {
                // $0 / \0: the whole match.
                return Some(&subject[span.start..span.end]);
            }
            if n > span.groups.len() {
                return None;
            }
            span.groups[n - 1].map(|(s, e)| &subject[s..e])
        }
    }

    /// PHP `preg_replace` semantics: replace every match (always global),
    /// expanding the replacement string per match.
    pub fn replace(&self, subject: &str, repl: &str) -> Result<String, SolveError> {
        let (spans, _) = self.match_all_limited(subject, None)?;
        self.replace_with_spans(subject, &spans, repl)
    }

    /// Replace pre-computed spans (avoids re-running `match_all` when the
    /// caller already has them). Output is bounded by `MAX_TOOL_RESULT_BYTES`;
    /// exceeding it is a hard error, never a silently truncated string.
    pub fn replace_with_spans(
        &self,
        subject: &str,
        spans: &[MatchSpan],
        repl: &str,
    ) -> Result<String, SolveError> {
        let mut out = String::new();
        out.try_reserve(subject.len().saturating_add(repl.len()))
            .map_err(|_| SolveError::ResultTooLarge)?;
        let mut last = 0usize;
        for sp in spans {
            out.push_str(&subject[last..sp.start]);
            let expansion = {
                let lookup = self.group_lookup(subject, sp);
                crate::solve::subst::expand(repl, self.ngroups, lookup)
            };
            if out.len() + expansion.len() > config::MAX_TOOL_RESULT_BYTES {
                return Err(SolveError::ResultTooLarge);
            }
            out.push_str(&expansion);
            last = sp.end;
        }
        out.push_str(&subject[last..]);
        Ok(out)
    }

    /// PHP `list` tool semantics: each matched text is replaced (limit 1)
    /// independently and all results are concatenated without separator.
    pub fn list(&self, subject: &str, repl: &str) -> Result<String, SolveError> {
        let (spans, _) = self.match_all_limited(subject, None)?;
        self.list_with_spans(subject, &spans, repl)
    }

    /// List pre-computed spans; same output budget as `replace_with_spans`.
    pub fn list_with_spans(
        &self,
        subject: &str,
        spans: &[MatchSpan],
        repl: &str,
    ) -> Result<String, SolveError> {
        let mut out = String::new();
        for sp in spans {
            let expansion = {
                let lookup = self.group_lookup(subject, sp);
                crate::solve::subst::expand(repl, self.ngroups, lookup)
            };
            if out.len() + expansion.len() > config::MAX_TOOL_RESULT_BYTES {
                return Err(SolveError::ResultTooLarge);
            }
            out.push_str(&expansion);
        }
        Ok(out)
    }
}

impl Drop for CompiledRegex {
    fn drop(&mut self) {
        unsafe {
            if !self.jit_stack.is_null() {
                sys::pcre2_jit_stack_free_8(self.jit_stack);
            }
            if !self.mcontext.is_null() {
                sys::pcre2_match_context_free_8(self.mcontext);
            }
            if !self.match_data.is_null() {
                sys::pcre2_match_data_free_8(self.match_data);
            }
            if !self.code.is_null() {
                sys::pcre2_code_free_8(self.code);
            }
        }
    }
}

/// PCRE2 library version string (for the UI's reference sidebar).
pub fn pcre2_version() -> String {
    unsafe {
        let mut buf = [0u8; 64];
        // NOTE: pcre2_config() returns the string length (incl. NUL) for
        // string-valued options, not 0.
        let rc = sys::pcre2_config_8(sys::PCRE2_CONFIG_VERSION, buf.as_mut_ptr() as *mut _);
        if rc >= 0 {
            let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            let v = String::from_utf8_lossy(&buf[..end]).into_owned();
            if !v.is_empty() {
                return v;
            }
        }
        "PCRE2".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_match_offsets() {
        let re = CompiledRegex::compile(r"\w+", "g").unwrap();
        let spans = re.match_all("ab cd").unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!((spans[0].start, spans[0].end), (0, 2));
        assert_eq!((spans[1].start, spans[1].end), (3, 5));
    }

    #[test]
    fn utf8_offsets_and_groups() {
        let re = CompiledRegex::compile(r"(中)+", "g").unwrap();
        let spans = re.match_all("a中中b").unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (1, 7)); // bytes
        assert_eq!(spans[0].groups[0], Some((4, 7)));
    }

    #[test]
    fn compile_error_has_offset() {
        let err = match CompiledRegex::compile("a(b", "") {
            Err(e) => e,
            Ok(_) => panic!("expected compile error"),
        };
        match err {
            SolveError::RegexParse { message } => {
                assert!(message.contains("at offset"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn unknown_modifier() {
        match CompiledRegex::compile("a", "gz") {
            Err(SolveError::RegexParse { .. }) => {}
            _ => panic!("expected compile error"),
        }
    }

    #[test]
    fn empty_match_retries_nonempty_alternative() {
        // PCRE2/PHP global iteration: after the empty match at 0, a non-empty
        // retry at the same offset must find the "a" branch of (?:|a). At
        // offset 1 the subject has 'b', so the retry fails there and the scan
        // advances one char.
        let re = CompiledRegex::compile(r"(?:|a)", "g").unwrap();
        let spans = re.match_all("ab").unwrap();
        let pairs: Vec<(usize, usize)> = spans.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(pairs, vec![(0, 0), (0, 1), (1, 1), (2, 2)]);
    }

    #[test]
    fn empty_match_global_does_not_hang() {
        let re = CompiledRegex::compile("a*", "g").unwrap();
        let spans = re.match_all("baa").unwrap();
        // "": 0..0, "aa": 1..3, "": 3..3
        let pairs: Vec<(usize, usize)> = spans.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(pairs, vec![(0, 0), (1, 3), (3, 3)]);
    }

    #[test]
    fn empty_match_multibyte_advances_one_char() {
        // Regression: advancing one byte after an empty match used to land
        // mid-character, producing offsets that panicked the byte->UTF-16
        // conversion. One empty match per character is expected instead.
        let re = CompiledRegex::compile("x*", "g").unwrap();
        let spans = re.match_all("中文abc").unwrap();
        let pairs: Vec<(usize, usize)> = spans.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(pairs, vec![(0, 0), (3, 3), (6, 6), (7, 7), (8, 8), (9, 9)]);
    }

    #[test]
    fn empty_match_astral_stays_on_char_boundaries() {
        let re = CompiledRegex::compile("x*", "g").unwrap();
        let spans = re.match_all("\u{1F600}x").unwrap();
        // "" at the emoji, "x" at 4..5, then "" at the end.
        let pairs: Vec<(usize, usize)> = spans.iter().map(|s| (s.start, s.end)).collect();
        assert_eq!(pairs, vec![(0, 0), (4, 5), (5, 5)]);
    }

    #[test]
    fn match_limit_is_enforced() {
        let re = CompiledRegex::compile(r"(a+)+$", "").unwrap();
        let text = "a".repeat(60) + "b";
        match re.match_one(&text) {
            Err(SolveError::MatchLimit) => {}
            other => panic!("expected MatchLimit, got {other:?}"),
        }
    }

    #[test]
    fn unmatched_group_maps_to_none() {
        let re = CompiledRegex::compile(r"(a)|(b)", "g").unwrap();
        let spans = re.match_all("ab").unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].groups[0], Some((0, 1)));
        assert_eq!(spans[0].groups[1], None);
        assert_eq!(spans[1].groups[0], None);
        assert_eq!(spans[1].groups[1], Some((1, 2)));
    }

    #[test]
    fn replace_expands_refs() {
        let re = CompiledRegex::compile(r"(\w+)@(\w+)", "").unwrap();
        let out = re.replace("bob@host x", "$2:$1").unwrap();
        assert_eq!(out, "host:bob x");
    }

    #[test]
    fn list_concatenates() {
        let re = CompiledRegex::compile(r"\d+", "g").unwrap();
        let out = re.list("a1b22c333", "<$0>").unwrap();
        assert_eq!(out, "<1><22><333>");
    }

    #[test]
    fn many_groups_hit_capture_cell_budget() {
        // (x?) never consumes 'a', so every position yields one empty match
        // carrying 601 cells: 2000 positions x 601 > MAX_CAPTURE_CELLS must
        // error instead of building the full span list.
        let pattern = "(x?)".repeat(600);
        let re = CompiledRegex::compile(&pattern, "g").unwrap();
        let text = "a".repeat(2000);
        match re.match_all(&text) {
            Err(SolveError::ResultTooLarge) => {}
            other => panic!("expected ResultTooLarge, got {:?}", other.map(|v| v.len())),
        }
    }

    #[test]
    fn replace_output_budget_is_enforced() {
        // 10,000 matches x 1KiB replacement = ~10MB > MAX_TOOL_RESULT_BYTES.
        let re = CompiledRegex::compile(r"a", "g").unwrap();
        let text = "a".repeat(10_000);
        let repl = "x".repeat(1024);
        match re.replace(&text, &repl) {
            Err(SolveError::ResultTooLarge) => {}
            Ok(_) => panic!("expected ResultTooLarge"),
            other => panic!("expected ResultTooLarge, got {other:?}"),
        }
    }

    #[test]
    fn unicode_word_semantics_ucp() {
        // PCRE2_UCP is always on (PHP /u parity): \w matches CJK code points.
        let re = CompiledRegex::compile(r"\w+", "g").unwrap();
        let spans = re.match_all("中文").unwrap();
        assert_eq!(spans.len(), 1);
        assert_eq!((spans[0].start, spans[0].end), (0, 6));
    }

    #[test]
    fn version_available() {
        let v = pcre2_version();
        eprintln!("PCRE2 version = {v:?}");
        assert!(v.starts_with("10."), "got: {v:?}");
    }
}
