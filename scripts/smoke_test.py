#!/usr/bin/env python3
"""Replay server/tests/fixtures against the REAL compiled binary.

The `cargo test` fixture replay runs through the in-process test router; this
script validates the actual deployment artifact end to end: static (musl)
linkage, PCRE2 JIT on the target CPU, rust-embed asset serving, and security
headers. Stdlib only (python3, no pip).

Usage:
    python3 scripts/smoke_test.py /path/to/regexr-server [--port N]
"""

import argparse
import json
import os
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import quote

ROOT = Path(__file__).resolve().parent.parent
FIXTURES = ROOT / "server" / "tests" / "fixtures"


def assert_subset(expected, actual, path, failures):
    """Port of the recursive subset assertion in server/tests/api_envelope.rs:
    every value present in `expected` must match `actual`."""
    if isinstance(expected, dict):
        if not isinstance(actual, dict):
            failures.append(f"{path}: expected object, got {type(actual).__name__}")
            return
        for k, v in expected.items():
            if k not in actual:
                failures.append(f"{path}.{k}: missing key")
            else:
                assert_subset(v, actual[k], f"{path}.{k}", failures)
    elif isinstance(expected, list):
        if not isinstance(actual, list) or len(expected) != len(actual):
            failures.append(f"{path}: expected array of {len(expected)}, got {actual!r:.120}")
            return
        for i, (e, a) in enumerate(zip(expected, actual)):
            assert_subset(e, a, f"{path}[{i}]", failures)
    elif isinstance(expected, bool) or isinstance(actual, bool):
        if expected is not actual:
            failures.append(f"{path}: expected {expected!r}, got {actual!r}")
    elif expected != actual:
        failures.append(f"{path}: expected {expected!r}, got {actual!r}")


def request(port, method, path, body=None, headers=None):
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}", data=body, headers=headers or {}, method=method
    )
    with urllib.request.urlopen(req, timeout=10) as resp:
        return resp.status, dict(resp.headers), resp.read()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("binary")
    ap.add_argument("--port", type=int, default=0)
    args = ap.parse_args()

    if args.port:
        port = args.port
    else:
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]

    env = dict(os.environ, REGEXR_ADDR=f"127.0.0.1:{port}")
    proc = subprocess.Popen(
        [args.binary], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.STDOUT
    )
    failures = []
    try:
        for _ in range(50):
            try:
                request(port, "GET", "/")
                break
            except (urllib.error.URLError, ConnectionError, OSError):
                time.sleep(0.1)
        else:
            print(f"FAIL: server did not come up on port {port}")
            return 1

        count = 0
        for f in sorted(FIXTURES.glob("*.json")):
            fixture = json.loads(f.read_text(encoding="utf-8"))
            data = json.dumps(fixture["request"], ensure_ascii=False)
            body = f"action={quote('regex/solve', safe='')}&data={quote(data, safe='')}"
            status, _, raw = request(
                port,
                "POST",
                "/server/api.php",
                body.encode(),
                {"Content-Type": "application/x-www-form-urlencoded"},
            )
            name = f.name
            count += 1
            if status != 200:
                failures.append(f"{name}: HTTP {status}")
                continue
            resp = json.loads(raw)
            if resp.get("success") is not True:
                failures.append(f"{name}: success={resp.get('success')}")
                continue
            assert_subset(fixture["response"], resp["data"], name, failures)

        status, headers, raw = request(port, "GET", "/")
        html = raw.decode("utf-8", "replace")
        if status != 200:
            failures.append(f"GET /: HTTP {status}")
        if "regexr.init(false," not in html:
            failures.append("GET /: phpinject init missing")
        if "id=\"regexWorker\"" not in html:
            failures.append("GET /: inline worker missing")
        if headers.get("x-frame-options") != "DENY":
            failures.append("GET /: x-frame-options missing")
        if "default-src 'self'" not in (headers.get("content-security-policy") or ""):
            failures.append("GET /: CSP missing")

        if failures:
            print(f"SMOKE FAIL ({len(failures)} failures / {count} fixtures):")
            for f in failures:
                print(f"  - {f}")
            return 1
        print(f"SMOKE OK: {count} fixtures + static/headers verified against {args.binary}")
        return 0
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    sys.exit(main())
