//! Central security & resource-limit configuration.
//!
//! Every hardening knob lives here so it can be reviewed line-by-line
//! during a security audit.

/// Maximum accepted request body size (form-urlencoded `data` JSON).
pub const MAX_BODY_BYTES: usize = 1 << 20; // 1 MiB

/// Maximum number of concurrent solve requests before shedding load.
pub const MAX_CONCURRENT_SOLVE: usize = 8;

/// Wall-clock backstop for a request. NOTE: this cannot interrupt a running
/// `pcre2_match` FFI call; the real hard limits against pathological regexes
/// are `PCRE2_MATCH_LIMIT` / `PCRE2_DEPTH_LIMIT` below.
pub const REQUEST_TIMEOUT_SECS: u64 = 10;

/// PCRE2 match limit: bounds total match attempts (anti catastrophic backtracking).
pub const PCRE2_MATCH_LIMIT: u32 = 1_000_000;

/// PCRE2 depth limit: bounds recursion depth of the interpreter/JIT.
pub const PCRE2_DEPTH_LIMIT: u32 = 10_000;

/// JIT stack limits (initial, maximum) in bytes.
pub const JIT_STACK_START: usize = 64 * 1024;
pub const JIT_STACK_MAX: usize = 1024 * 1024;

/// Maximum number of matches returned for one solve (protects the UI/JSON size).
pub const MAX_MATCHES: usize = 20_000;

/// Maximum number of capture-group cells (matches x groups) materialized for
/// one solve. A 16KiB pattern can declare thousands of groups; without this
/// budget, thousands of groups x 20,000 matches would allocate gigabytes of
/// intermediate `MatchSpan`/JSON data and OOM the process.
pub const MAX_CAPTURE_CELLS: usize = 1_000_000;

/// Maximum expanded `replace`/`list` tool output size in bytes. A ~1MiB
/// replacement string combined with many empty matches could otherwise
/// produce tens of GB of output.
pub const MAX_TOOL_RESULT_BYTES: usize = 4 * 1024 * 1024;

/// Backstop for the serialized JSON response. The per-tool caps above bound
/// the intermediate structures; this bounds the final envelope (including
/// tests-mode and per-match JSON overhead).
pub const MAX_RESPONSE_BYTES: usize = 32 * 1024 * 1024;

/// Maximum number of tests per request in `tests` mode.
pub const MAX_TESTS: usize = 1_000;

/// Maximum pattern length in bytes.
pub const MAX_PATTERN_BYTES: usize = 16 * 1024;

/// Listen address, overridable via `REGEXR_ADDR`.
pub const DEFAULT_ADDR: &str = "0.0.0.0:8080";
