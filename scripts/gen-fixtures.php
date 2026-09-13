<?php
/**
 * Regenerate the `response` field of server/tests/fixtures/*.json using the
 * original PHP PCRE semantics (port of regexr-cn server/actions/regex/solve.php).
 *
 * Run on a machine with PHP (ext-mbstring required):
 *   php scripts/gen-fixtures.php            # regenerate all fixtures in place
 *   php scripts/gen-fixtures.php 01_...json # regenerate a single fixture
 *
 * NOTE: this is a faithful, standalone port of solve.php's core logic (the
 * original cannot run without its DB bootstrap). It reproduces:
 *   - PREG_OFFSET_CAPTURE byte offsets converted to the frontend's UTF-16
 *     code-unit offsets (the original emitted UTF-8 char counts; UTF-16 is
 *     what the browser worker/frontend actually uses, so we upgrade here)
 *   - g flag -> preg_match_all vs preg_match
 *   - tool replace (preg_replace, always global) / list (per-match, concat)
 *   - tests mode: {id,i,l} / {id} / {id,error}
 *   - solve-level errors inside the success envelope
 * Fields that vary per-run (time, timestamp) are omitted; the Rust test
 * compares a subset of keys only.
 */

if ($argc < 2) { fwrite(STDERR, "usage: php gen-fixtures.php <fixture.json> [...]\n"); exit(1); }

foreach (array_slice($argv, 1) as $file) {
    $fixture = json_decode(file_get_contents($file));
    if (!$fixture) { fwrite(STDERR, "bad fixture: $file\n"); exit(1); }
    $fixture->response = solve($fixture->request);
    file_put_contents($file, json_encode($fixture, JSON_PRETTY_PRINT | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE) . "\n");
    echo "updated $file\n";
}

function solve($req) {
    // Per-request error capture: real PHP serves one request per process, so
    // error_get_last() in the original backend could never see a previous
    // request's warning. Isolate the same way (fixture 10's leaked
    // "Unknown modifier" message came from exactly this).
    $GLOBALS['__preg_warning'] = null;
    set_error_handler(function ($errno, $errstr) {
        $GLOBALS['__preg_warning'] = $errstr;
        return true; // preg_* failures are handled explicitly below
    });
    try {
        return solve_body($req);
    } finally {
        restore_error_handler();
    }
}

function solve_body($req) {
    $pattern = isset($req->pattern) ? $req->pattern : '';
    $flags = isset($req->flags) ? $req->flags : '';
    $mode = isset($req->mode) ? $req->mode : 'text';
    $global = strpos($flags, 'g') !== false;
    // The Rust server always compiles with PCRE2_UTF (documented in
    // README/DEPLOY). The gold standard must match that declared semantics,
    // so /u is forced here regardless of the request flags.
    $modifiers = str_replace('g', '', $flags) . 'u';
    $re = "/{$pattern}/{$modifiers}";

    $out = ['id' => isset($req->id) ? $req->id : null, 'mode' => $mode];

    if ($mode === 'tests') {
        $matches = [];
        foreach (($req->tests ?: []) as $t) {
            $text = isset($t->text) ? $t->text : '';
            $entry = ['id' => isset($t->id) ? $t->id : null];
            $m = [];
            $ok = $global
                ? preg_match_all($re, $text, $m, PREG_OFFSET_CAPTURE | PREG_SET_ORDER | PREG_UNMATCHED_AS_NULL)
                : preg_match($re, $text, $m, PREG_OFFSET_CAPTURE | PREG_UNMATCHED_AS_NULL);
            if ($ok === false) {
                $entry['error'] = pcre_error();
            } elseif ($ok > 0) {
                $set = $global ? $m[0] : $m;
                $entry['i'] = utf16_offset($text, $set[0][1]);
                $entry['l'] = utf16_len($set[0][0]);
            }
            $matches[] = (object) $entry;
        }
        $out['matches'] = $matches;
        return (object) $out;
    }

    // mode = text
    $text = isset($req->text) ? $req->text : '';
    $m = [];
    $ok = $global
        ? preg_match_all($re, $text, $m, PREG_OFFSET_CAPTURE | PREG_SET_ORDER | PREG_UNMATCHED_AS_NULL)
        : preg_match($re, $text, $m, PREG_OFFSET_CAPTURE | PREG_UNMATCHED_AS_NULL);
    if ($ok === false) {
        $out['matches'] = [];
        $out['error'] = pcre_error();
        return (object) $out;
    }

    $matches = [];
    $sets = $global ? $m : [$m];
    foreach ($sets as $set) {
        if ($set[0][0] === '' && $set[0][1] === -1) { continue; } // unmatched overall (shouldn't happen)
        $groups = [];
        foreach ($set as $k => $v) {
            if (!is_int($k)) { continue; }          // named groups skipped (they also have numeric keys)
            if ($k === 0) { continue; }             // group 0 not repeated in groups[]
            $groups[] = $v[1] === -1
                ? (object) ['i' => 0, 'l' => 0]     // PHP gave {0,0} for unmatched
                : (object) ['i' => utf16_offset($text, $v[1]), 'l' => utf16_len($v[0])];
        }
        $matches[] = (object) [
            'i' => utf16_offset($text, $set[0][1]),
            'l' => utf16_len($set[0][0]),
            'groups' => $groups,
        ];
    }
    $out['matches'] = $matches;

    if (isset($req->tool)) {
        $toolId = isset($req->tool->id) ? $req->tool->id : '';
        $input = isset($req->tool->input) ? $req->tool->input : '';
        $result = '';
        if ($toolId === 'replace') {
            $result = preg_replace($re, $input, $text);
        } elseif ($toolId === 'list') {
            $sets2 = $global ? $m : [$m];
            foreach ($sets2 as $set) {
                $result .= preg_replace($re, $input, $set[0][0], 1);
            }
        }
        $out['tool'] = (object) ['id' => $toolId, 'result' => $result];
    }
    return (object) $out;
}

function pcre_error() {
    $name = 'PREG_INTERNAL_ERROR';
    $id = 'error';
    $code = preg_last_error();
    if ($code === PREG_BACKTRACK_LIMIT_ERROR) { $name = 'PREG_BACKTRACK_LIMIT_ERROR'; $id = 'infinite'; }
    elseif ($code === PREG_RECURSION_LIMIT_ERROR) { $name = 'PREG_RECURSION_LIMIT_ERROR'; $id = 'infinite'; }
    elseif ($code === PREG_JIT_STACKLIMIT_ERROR) { $name = 'PREG_JIT_STACKLIMIT_ERROR'; $id = 'infinite'; }
    elseif ($code === PREG_BAD_UTF8_ERROR) { $name = 'PREG_BAD_UTF8_ERROR'; $id = 'badutf8'; }
    elseif ($code === PREG_BAD_UTF8_OFFSET_ERROR) { $name = 'PREG_BAD_UTF8_OFFSET_ERROR'; $id = 'badutf8'; }
    $msg = preg_replace('/^[a-z_():\s]+/', '', (string) $GLOBALS['__preg_warning']);
    $e = ['name' => $name, 'id' => $id];
    if ($msg !== '' && $msg !== null) { $e['message'] = $msg; }
    if ($id === 'infinite') { $e['warning'] = true; }
    return (object) $e;
}

/** UTF-8 byte offset -> UTF-16 code unit offset (frontend semantics). */
function utf16_offset($text, $byteOffset) {
    return utf16_len(mb_strcut($text, 0, $byteOffset, 'UTF-8'));
}

/** Length in UTF-16 code units (JS `str.length`). */
function utf16_len($s) {
    return strlen(mb_convert_encoding($s, 'UTF-16BE', 'UTF-8')) / 2;
}
