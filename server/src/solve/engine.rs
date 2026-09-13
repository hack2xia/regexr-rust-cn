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
        let mut options: u32 = sys::PCRE2_UTF;
        for c in flags.chars() {
            options |= match c {
                'g' => {
                    global = true;
                    continue;
                }
                'i' => sys::PCRE2_CASELESS,
                'm' => sys::PCRE2_MULTILINE,
                's' => sys::PCRE2_DOTALL,
                'x' => sys::PCRE2_EXTENDED,
                'u' => sys::PCRE2_UTF, // always on
                'U' => sys::PCRE2_UNGREEDY,
                'A' => sys::PCRE2_ANCHORED,
                'D' => sys::PCRE2_DOLLAR_ENDONLY,
                'J' => sys::PCRE2_DUPNAMES,
                'S' => 0, // PHP "study" hint: no-op
                'X' => sys::PCRE2_EXTENDED_MORE,
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
            // JIT: failure is non-fatal, fall back to the interpreter.
            let jit = sys::pcre2_jit_compile_8(code, sys::PCRE2_JIT_COMPLETE) >= 0;

            let match_data =
                sys::pcre2_match_data_create_from_pattern_8(code, std::ptr::null_mut());
            let mcontext = sys::pcre2_match_context_create_8(std::ptr::null_mut());
            let mut jit_stack: *mut sys::pcre2_jit_stack_8 = std::ptr::null_mut();
            if !mcontext.is_null() {
                // Hard anti-DoS limits (see config.rs). These live on the
                // match context in PCRE2's API.
                sys::pcre2_set_match_limit_8(mcontext, config::PCRE2_MATCH_LIMIT);
                sys::pcre2_set_depth_limit_8(mcontext, config::PCRE2_DEPTH_LIMIT);
                jit_stack = sys::pcre2_jit_stack_create_8(
                    config::JIT_STACK_START,
                    config::JIT_STACK_MAX,
                    std::ptr::null_mut(),
                );
                if !jit_stack.is_null() {
                    sys::pcre2_jit_stack_assign_8(mcontext, None, jit_stack as *mut _);
                }
            }

            // ovector count is the number of (start,end) pairs = ngroups + 1.
            let pairs = if match_data.is_null() {
                0
            } else {
                sys::pcre2_get_ovector_count_8(match_data) as usize
            };

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
    fn exec(&self, subject: &[u8], start: usize) -> Result<bool, SolveError> {
        let rc = unsafe {
            if self.jit {
                sys::pcre2_jit_match_8(
                    self.code,
                    subject.as_ptr(),
                    subject.len(),
                    start,
                    0,
                    self.match_data,
                    self.mcontext,
                )
            } else {
                sys::pcre2_match_8(
                    self.code,
                    subject.as_ptr(),
                    subject.len(),
                    start,
                    0,
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

    /// Find all matches. Global iteration mirrors `preg_match_all`: after an
    /// empty match the scan position advances by exactly one byte
    /// (PHP php_pcre.c semantics; also what the JS worker does via lastIndex++).
    pub fn match_all(&self, subject: &str) -> Result<Vec<MatchSpan>, SolveError> {
        let bytes = subject.as_bytes();
        let mut out: Vec<MatchSpan> = Vec::new();
        let mut start: usize = 0;

        loop {
            if start > bytes.len() {
                break;
            }
            if !self.exec(bytes, start)? {
                break;
            }
            let span = self.current_span(bytes);
            start = if span.end == span.start {
                // Advance one *character*, not one byte: PCRE2 accepts a
                // mid-UTF-8-sequence start offset (byte semantics), but a
                // non-boundary match offset cannot be converted to UTF-16
                // for the frontend (and PHP's UTF mode advances one char too).
                let mut next = span.end + 1;
                while next < bytes.len() && !subject.is_char_boundary(next) {
                    next += 1;
                }
                next
            } else {
                span.end
            };
            if out.len() < config::MAX_MATCHES {
                out.push(span);
            } else {
                // Limit reached: stop scanning. Continuing would burn up to
                // MAX_MATCHES..subject.len FFI calls that the result discards.
                break;
            }
        }
        Ok(out)
    }

    /// First match only (non-global semantics).
    pub fn match_one(&self, subject: &str) -> Result<Option<MatchSpan>, SolveError> {
        if self.exec(subject.as_bytes(), 0)? {
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
        let spans = self.match_all(subject)?;
        let mut out = String::with_capacity(subject.len());
        let mut last = 0usize;
        for sp in &spans {
            out.push_str(&subject[last..sp.start]);
            let lookup = self.group_lookup(subject, sp);
            out.push_str(&crate::solve::subst::expand(repl, self.ngroups, lookup));
            last = sp.end;
        }
        out.push_str(&subject[last..]);
        Ok(out)
    }

    /// PHP `list` tool semantics: each matched text is replaced (limit 1)
    /// independently and all results are concatenated without separator.
    pub fn list(&self, subject: &str, repl: &str) -> Result<String, SolveError> {
        let spans = self.match_all(subject)?;
        let mut out = String::new();
        for sp in &spans {
            let lookup = self.group_lookup(subject, sp);
            out.push_str(&crate::solve::subst::expand(repl, self.ngroups, lookup));
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
    fn version_available() {
        let v = pcre2_version();
        eprintln!("PCRE2 version = {v:?}");
        assert!(v.starts_with("10."), "got: {v:?}");
    }
}
