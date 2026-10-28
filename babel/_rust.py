"""Bridge to the Rust formatting kernel (`babel._core`).

The hot functions in `numbers.py` / `dates.py` delegate here. This module:

* caches `Locale.parse` results per input (the parse itself is verbatim,
  so lookup misses behave exactly like upstream);
* builds each locale's ingredient bundle once (pattern strings, symbols,
  names extracted from the installed `babel` data files — the single
  source of truth; no CLDR copy lives in the Rust crate) and hands it to
  the extension, which parses all patterns eagerly and caches them;
* converts number inputs exactly like upstream
  (`value if Decimal else Decimal(str(value))`) and passes the canonical
  `str()` across; conversion errors are therefore identical by
  construction;
* exposes `Fallback` (the extension's `FallbackError`): any kernel miss
  replays the verbatim upstream logic in the caller.

Set `BABEL_RUST_NO_RUST=1` to force the pure-Python paths (used by the
equivalence suite to diff both sides, and by operators who want the
upstream behavior byte-for-byte without loading the extension).

Known limitation: locale data is treated as immutable once a locale has
been formatted through the kernel. Mutating `locale._data` in place after
that point keeps serving the first snapshot through the fast path (the
fallback path always reads live data). No upstream test mutates locale
data, and `localedata._cache.clear()` correctly re-resolves because the
freshness check keys on the underlying data object.
"""

import decimal
import json
import os

from babel.core import Locale

try:
    from babel import _core as _core
except ImportError:  # extension not built: pure-Python install
    _core = None

class _PureFallback(Exception):
    """Raised when the extension is unavailable; caught like `FallbackError`."""


Fallback = _core.FallbackError if _core is not None else _PureFallback


_ENABLED = None


def enabled() -> bool:
    """Whether the Rust fast path may be attempted.

    Evaluated once per process: entry points set `BABEL_RUST_NO_RUST`
    before startup (subprocess isolation in benches and the corpus).
    """
    global _ENABLED
    if _ENABLED is None:
        _ENABLED = _core is not None and not os.environ.get("BABEL_RUST_NO_RUST")
    return _ENABLED


_LOCALE_CACHE: dict = {}
_BUNDLE_STATE: dict = {}
_IDENT_CACHE: dict = {}
# Kernel-coverage counters (tests assert the fast path actually fires).
ATTEMPTS = 0
FALLBACKS = 0


def _counted(fn):
    import functools

    @functools.wraps(fn)
    def wrapper(*args, **kwargs):
        global ATTEMPTS, FALLBACKS
        ATTEMPTS += 1
        try:
            return fn(*args, **kwargs)
        except Fallback:
            FALLBACKS += 1
            raise

    return wrapper


def cached_parse(loc, parse):
    """Verbatim `parse(loc)` with a process-wide exact-hit cache.

    `Locale` instances pass through (same as `Locale.parse`); misses run
    the given `parse` callable so errors are identical by construction.
    """
    if isinstance(loc, Locale):
        return loc
    try:
        return _LOCALE_CACHE[loc]
    except KeyError:
        pass
    except TypeError:
        return parse(loc)
    parsed = parse(loc)
    if len(_LOCALE_CACHE) < 4096:
        _LOCALE_CACHE[loc] = parsed
    return parsed


def _plain(mapping):
    return {k: v for k, v in dict(mapping).items()}


def build_bundle(locale) -> str:
    """Extract the kernel ingredient bundle for `locale` as JSON."""
    from babel.dates import (
        get_day_names,
        get_era_names,
        get_month_names,
        get_period_names,
        get_quarter_names,
    )

    def pat_key(k):
        return "default" if k is None else str(k)

    # Mirrors the `get_*_symbol` getters, which use `.get(key, default)`:
    # missing keys fall back to '.' / ',' / '+' / '-' / 'E' / '∞'.
    # Alias-only entries (e.g. `{'alias': [...]}`) carry no symbols at all;
    # upstream raises KeyError when such a system is actually used, and the
    # kernel reports them missing so the fallback replays it.
    _sym_defaults = (
        ("decimal", "decimal", "."),
        ("group", "group", ","),
        ("plus", "plusSign", "+"),
        ("minus", "minusSign", "-"),
        ("exponential", "exponential", "E"),
        ("infinity", "infinity", "\u221e"),
    )
    number_symbols = {}
    for ns, syms in locale.number_symbols.items():
        syms = _plain(syms)
        if "decimal" not in syms and "group" not in syms:
            continue
        number_symbols[str(ns)] = {
            out: syms.get(key, default) for out, key, default in _sym_defaults
        }

    months = {}
    days = {}
    for ctx in ("format", "stand-alone"):
        months[ctx] = {
            w: [get_month_names(w, ctx, locale)[i] for i in range(1, 13)]
            for w in ("wide", "abbreviated", "narrow")
        }
        days[ctx] = {
            w: [get_day_names(w, ctx, locale)[i] for i in range(7)]
            for w in ("wide", "abbreviated", "narrow", "short")
        }
    quarters = {
        ctx: {
            w: [get_quarter_names(w, ctx, locale)[i] for i in range(1, 5)]
            for w in ("wide", "abbreviated", "narrow")
        }
        for ctx in ("format", "stand-alone")
    }
    eras = {
        w: [get_era_names(w, locale)[i] for i in (0, 1)]
        for w in ("wide", "abbreviated", "narrow")
    }
    periods = {
        "format": {
            w: {str(k): str(v) for k, v in _plain(get_period_names(w, "format", locale)).items()}
            for w in ("wide", "abbreviated", "narrow")
        }
    }
    bundle = {
        "id": str(locale),
        "first_week_day": int(locale.first_week_day),
        "min_week_days": int(locale.min_week_days),
        "default_numbering_system": str(locale.default_numbering_system),
        "number_symbols": number_symbols,
        "decimal_formats": {pat_key(k): v.pattern for k, v in locale.decimal_formats.items()},
        "currency_formats": {pat_key(k): v.pattern for k, v in locale.currency_formats.items()},
        "percent_formats": {pat_key(k): v.pattern for k, v in locale.percent_formats.items()},
        "date_formats": {pat_key(k): v.pattern for k, v in locale.date_formats.items()},
        "time_formats": {pat_key(k): v.pattern for k, v in locale.time_formats.items()},
        "datetime_formats": {pat_key(k): str(v) for k, v in locale.datetime_formats.items()},
        "months": months,
        "days": days,
        "quarters": quarters,
        "eras": eras,
        "periods": periods,
    }
    return json.dumps(bundle, ensure_ascii=False)


def ensure_locale(locale, hint=None) -> str:
    """Make the Rust core cache this locale's bundle; return its identifier.

    Re-extracts when the underlying locale data object changed (e.g. after
    `localedata._cache.clear()` + reload). Raises `Fallback` when the
    extension is unavailable or the locale has no kernel-usable data, in
    which case the caller replays the upstream logic.
    """
    ident = None
    if hint is not None:
        try:
            ident = _IDENT_CACHE.get(hint)
        except TypeError:
            hint = None
    if ident is None:
        ident = str(locale)
        if hint is not None and len(_IDENT_CACHE) < 4096:
            try:
                _IDENT_CACHE[hint] = ident
            except TypeError:
                pass
    data_id = id(locale._data._data)
    if _BUNDLE_STATE.get(ident) != data_id:
        if _core is None:
            raise RuntimeError("babel._core extension is not available")
        try:
            bundle_json = build_bundle(locale)
        except Exception:
            raise Fallback("locale bundle not extractable")
        _core.ensure_locale_py(bundle_json)
        _BUNDLE_STATE[ident] = data_id
    return ident


def dec_str(value) -> str:
    """Verbatim upstream input conversion, returned as canonical `str()`."""
    if not isinstance(value, decimal.Decimal):
        value = decimal.Decimal(str(value))
    return str(value)


_GET_CURRENCY_SYMBOL = None
_GET_CURRENCY_PRECISION = None


def _currency_ingredients(locale, currency):
    """Verbatim symbol + precision lookups (module globals cache imports)."""
    global _GET_CURRENCY_SYMBOL, _GET_CURRENCY_PRECISION
    if _GET_CURRENCY_SYMBOL is None:
        from babel.numbers import get_currency_precision as gcp
        from babel.numbers import get_currency_symbol as gcs
        _GET_CURRENCY_SYMBOL = gcs
        _GET_CURRENCY_PRECISION = gcp
    return (_GET_CURRENCY_SYMBOL(currency, locale),
            _GET_CURRENCY_PRECISION(currency))


@_counted
def format_decimal(locale, number, decimal_quantization, group_separator,
                 numbering_system, hint=None):
    ident = ensure_locale(locale, hint)
    return _core.format_decimal(
        ident, dec_str(number), decimal_quantization, group_separator, numbering_system
    )


@_counted
def format_currency(
    locale,
    number,
    currency,
    currency_digits,
    decimal_quantization,
    group_separator,
    numbering_system,
    format_type,
    hint=None,
):
    symbol, prec = _currency_ingredients(locale, currency)
    ident = ensure_locale(locale, hint)
    return _core.format_currency(
        ident,
        dec_str(number),
        symbol,
        currency,
        prec,
        currency_digits,
        decimal_quantization,
        group_separator,
        numbering_system,
        format_type,
    )


@_counted
def format_percent(locale, number, decimal_quantization, group_separator,
                 numbering_system, hint=None):
    ident = ensure_locale(locale, hint)
    return _core.format_percent(
        ident, dec_str(number), decimal_quantization, group_separator, numbering_system
    )


@_counted
def format_date(locale, fmt_name, value, hint=None):
    ident = ensure_locale(locale, hint)
    return _core.format_date(
        ident, fmt_name, value.year, value.month, value.day, value.weekday()
    )


@_counted
def format_time(locale, fmt_name, value, hint=None):
    ident = ensure_locale(locale, hint)
    return _core.format_time(
        ident, fmt_name, value.hour, value.minute, value.second, value.microsecond
    )


@_counted
def format_datetime(locale, fmt_name, value, hint=None):
    ident = ensure_locale(locale, hint)
    return _core.format_datetime(
        ident,
        fmt_name,
        value.year,
        value.month,
        value.day,
        value.weekday(),
        value.hour,
        value.minute,
        value.second,
        value.microsecond,
    )
