"""Equivalence gate: pristine upstream == fallback == Rust kernel.

Runs the case corpus (`equiv_cases`, deep + broad tiers) under three
implementations in fresh subprocesses and asserts byte-identical outcomes
(values and exception type+message). Also asserts kernel coverage: the
fast path must serve the large majority of non-error calls (guards the
`Fallback` seam against silent over-firing).
"""

import json
import os
import subprocess
import sys

import pytest

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
MIRROR = os.path.dirname(TESTS_DIR)
PRISTINE = os.environ.get(
    "BABEL_PRISTINE_PYTHON",
    "/home/john/firstmate/data/rs-port-babel/venv/bin/python",
)

FALLBACK_ENV = dict(os.environ, BABEL_RUST_NO_RUST="1")
COVERAGE_FLOOR = 0.95


def run_corpus(python, extra_env, tier, out):
    env = dict(os.environ)
    env.update(extra_env)
    env["PYTHONPATH"] = MIRROR + os.pathsep + env.get("PYTHONPATH", "")
    r = subprocess.run(
        [python, os.path.join(TESTS_DIR, "equiv_run.py"),
         "--tier", tier, "--out", out],
        capture_output=True, text=True, cwd="/tmp", env=env, timeout=1200,
    )
    assert r.returncode == 0, f"corpus runner failed: {r.stderr[-2000:]}"
    rows = {}
    with open(out, encoding="utf-8") as f:
        for line in f:
            row = json.loads(line)
            rows[row["key"]] = row
    return rows


def norm(row):
    return (row.get("status"), row.get("value"),
            row.get("etype"), row.get("emsg"))


@pytest.mark.parametrize("tier", ["deep", "broad"])
def test_three_way_equivalence(tmp_path, tier):
    here_python = sys.executable
    rust = run_corpus(here_python, {}, tier, str(tmp_path / "rust.jsonl"))
    pure = run_corpus(here_python, {"BABEL_RUST_NO_RUST": "1"},
                      tier, str(tmp_path / "pure.jsonl"))
    pristine = run_corpus(PRISTINE, {"PYTHONPATH": ""}, tier,
                          str(tmp_path / "pristine.jsonl"))
    assert set(rust) == set(pure) == set(pristine)
    assert len(rust) > 2000
    bad = [(k, rust[k], pure[k], pristine[k]) for k in rust
           if not (norm(rust[k]) == norm(pure[k]) == norm(pristine[k]))]
    assert not bad, f"{len(bad)} divergences, first: {bad[:3]}"


def test_kernel_coverage():
    """The fast path must serve the bulk of kernel-domain calls."""
    from babel import _rust
    if not _rust.enabled():
        pytest.skip("extension disabled (BABEL_RUST_NO_RUST=1); kernel coverage n/a")
    import datetime
    from babel.dates import format_date, format_datetime, format_time
    from babel.localedata import locale_identifiers
    from babel.numbers import format_currency, format_decimal, format_percent
    assert _rust.enabled(), "extension not loaded; coverage meaningless"
    _rust.ATTEMPTS = 0
    _rust.FALLBACKS = 0
    d = datetime.date(2007, 4, 1)
    dt = datetime.datetime(2007, 4, 1, 15, 30)
    t = datetime.time(15, 30)
    for loc in sorted(locale_identifiers()):
        format_decimal(1234567.891, locale=loc)
        format_currency(1099.98, "USD", locale=loc)
        format_percent(0.341, locale=loc)
        format_date(d, locale=loc)
        format_time(t, locale=loc)
        format_datetime(dt, locale=loc)
    hit = 1 - _rust.FALLBACKS / _rust.ATTEMPTS
    assert hit >= COVERAGE_FLOOR, f"kernel hit rate {hit:.3f} < {COVERAGE_FLOOR}"
