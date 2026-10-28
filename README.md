# babel-rust

Rust port of the [`python-babel/babel`](https://github.com/python-babel/babel)
formatting hot path (2.18.0), with Python bindings. Drop-in mirror: same
import name (`babel`), same behavior, measured faster on formatting-heavy
workloads.

Upstream attribution: this project re-implements behavior described by
python-babel/babel (BSD-3-Clause, (c) 2013-2026 by the Babel Team; see
[LICENSE](LICENSE)). Locale data files (`babel/locale-data/*.dat`,
`babel/global.dat`) are CLDR-derived data used under the Unicode License
Agreement (see [LICENSE.unicode](LICENSE.unicode) and [NOTICE](NOTICE)).
No second copy of CLDR is vendored into the Rust crate: at runtime the
extension receives small per-locale ingredient bundles (pattern strings,
symbols, names) extracted from the installed data files above, which remain
the single source of truth.

## Scope

Ported to Rust (`babel-core`, no Python dependency) as one hot unit:

* number/currency/percent formatting (`numbers.parse_pattern`,
  `NumberPattern.apply` and helpers) for locale-default patterns;
* date/time/datetime formatting (`dates.tokenize_pattern`,
  `DateTimePattern` field dispatch) for the named formats
  (`full`/`long`/`medium`/`short`);
* locale-data lookup caching: each locale's bundle (patterns parsed
  eagerly) is cached once per process; per-call formatting crosses the
  FFI boundary with scalars only.

Deliberately **not** ported (documented seam — these stay Python, and any
input touching them replays the verbatim upstream logic):

* user-supplied `format` patterns and `format_type='name'` currency names
  (plural rules stay Python);
* compact/scientific formats, `force_frac`, timezone fields
  (`z Z O v V x X`), day-period rules (`b B`), the `g` field;
* values that would engage the `decimal` module's 28-digit context,
  signaling NaNs, or NaNs with payload;
* everything outside `format_decimal` / `format_currency` /
  `format_percent` / `format_date` / `format_time` / `format_datetime`
  (catalogs, pluralization, lists, units, parsing, intervals stay
  byte-identical upstream code).

The pure-Python fallback is always available: `BABEL_RUST_NO_RUST=1`
forces it (the equivalence suite diffs both sides), and a build without
the extension imports and runs as upstream.

## Layout

* `babel/` — the `babel` package (non-hot modules byte-identical to
  upstream 2.18.0; hot modules add a marked delegation block, fallback
  bodies verbatim) plus `babel/_rust.py` (the bridge) and `babel/_core*`
  (the compiled extension).
* `babel-core/` — reusable Rust core crate (patterns, decimal arithmetic,
  date fields, locale bundle cache). Usable from Rust directly.
* `src/` — PyO3 bindings (`babel._core`).
* `tests/` — equivalence corpus (`test_equiv_*.py`, Rust vs fallback
  across many locales) plus the unmodified upstream suite
  (`tests/upstream/`, run green in both modes).
* `benches/two_track.py` — measured-only two-track bench (wall + peak
  RSS, fresh process per input/side, every figure labeled by workload).

## Status

See the bench table and parity counts in the rustsmith evidence PR
(`John-Cusack/rustsmith`, `fm/rs-port-babel` branch).
