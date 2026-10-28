#!/usr/bin/env python3
"""Two-track bench: pure-Python `babel` vs `babel-rust`, measured only.

Tracks mirror the frozen WORKLOAD.md shape (6 locales x 9 ops), split per
operation so every figure is labeled by workload:

  decimal / currency / percent / date / time / datetime / lookup

Each side runs in a FRESH process per track (honest peak RSS); wall time is
the median of 3 reps of run(), reported per iteration. The pure side sets
BABEL_RUST_NO_RUST=1 (verbatim fallback path); the fast side uses the
compiled extension. No projections.

Usage: venv/bin/python benches/two_track.py [--iters N] [--out FILE]
"""

import argparse
import json
import os
import statistics
import subprocess
import sys
import time

PORT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
LOCALES = ['en_US', 'de_DE', 'fr_FR', 'sv_SE', 'ar_EG', 'ja_JP']

TRACKS = ['decimal', 'currency', 'percent', 'date', 'time', 'datetime', 'lookup']
# Reps per run() call, sized so the pure side takes ~0.2-0.5 s per track.
REPS = {'decimal': 4000, 'currency': 2500, 'percent': 4000, 'date': 2500,
        'time': 2500, 'datetime': 1500, 'lookup': 6000}

WORKER = r'''
import datetime, os, sys, time
sys.path.insert(0, {port!r})
track = os.environ["BB_TRACK"]
reps = int(os.environ["BB_REPS"])
if os.environ.get("BABEL_RUST_NO_RUST"):
    pass
from babel import Locale
from babel.dates import format_date, format_time, format_datetime
from babel.numbers import format_currency, format_decimal, format_percent
D = datetime.date(2007, 4, 1)
DT = datetime.datetime(2007, 4, 1, 15, 30)
T = datetime.time(15, 30)
LOCALES = {locales!r}
for loc in LOCALES:  # warm: locale data load happens once here
    Locale.parse(loc)
    format_decimal(1.5, locale=loc)
def run():
    total = 0
    for i in range(reps):
        a = 1234567.891 + (i % 997) * 0.137
        for loc in LOCALES:
            if track == "decimal":
                total += len(format_decimal(a, locale=loc))
                total += len(format_decimal(-9876.54321 - (i % 61), locale=loc))
            elif track == "currency":
                total += len(format_currency(a, "USD", locale=loc))
                total += len(format_currency(99.5 + (i % 17), "EUR", locale=loc))
            elif track == "percent":
                total += len(format_percent(a / 100000.0, locale=loc))
                total += len(format_percent(25.1234, locale=loc))
            elif track == "date":
                total += len(format_date(D, format="medium", locale=loc))
                total += len(format_date(D, format="full", locale=loc))
            elif track == "time":
                total += len(format_time(T, format="medium", locale=loc))
                total += len(format_time(T, format="short", locale=loc))
            elif track == "datetime":
                total += len(format_datetime(DT, format="medium", locale=loc))
                total += len(format_datetime(DT, format="short", locale=loc))
            elif track == "lookup":
                total += len(str(Locale.parse(loc)))
    return total
import resource
t0 = time.perf_counter()
c = run()
wall = time.perf_counter() - t0
peak_kb = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
print(f"{wall:.6f} {peak_kb} {c}")
'''.replace("{port!r}", repr(PORT)).replace("{locales!r}", repr(LOCALES))


def bench_track(python, track, pure):
    env = dict(os.environ)
    env["BB_TRACK"] = track
    env["BB_REPS"] = str(REPS[track])
    if pure:
        env["BABEL_RUST_NO_RUST"] = "1"
    walls, peaks, check = [], [], None
    for _ in range(3):
        t0 = time.perf_counter()
        p = subprocess.run([python, "-c", WORKER], capture_output=True,
                           text=True, env=env, timeout=600)
        dt = time.perf_counter() - t0
        assert p.returncode == 0, p.stderr[-2000:]
        wall, peak_kb, c = p.stdout.strip().split()
        if check is None:
            check = c
        assert c == check, f"output diverged on {track} pure={pure}"
        walls.append(float(wall))
        peaks.append(int(peak_kb))
        _ = dt
    return statistics.median(walls), max(peaks), check


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--iters", type=int, default=1)
    ap.add_argument("--out", default=None)
    args = ap.parse_args()
    python = sys.executable
    rows = []
    for track in TRACKS:
        pure_wall, pure_rss, c0 = bench_track(python, track, True)
        rust_wall, rust_rss, c1 = bench_track(python, track, False)
        assert c0 == c1, f"checksum diverged on {track}"
        rows.append({
            "track": track,
            "pure_wall_s": pure_wall, "rust_wall_s": rust_wall,
            "speedup": pure_wall / rust_wall if rust_wall else float("inf"),
            "pure_rss_kb": pure_rss, "rust_rss_kb": rust_rss,
            "checksum": c0,
        })
        print(f"{track:9s} pure {pure_wall:.4f}s rss {pure_rss}k | "
              f"rust {rust_wall:.4f}s rss {rust_rss}k | "
              f"{pure_wall / rust_wall:.2f}x", flush=True)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            json.dump(rows, f, indent=2)


main()
