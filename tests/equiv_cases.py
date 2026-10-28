"""Equivalence case builder: (key, op, args) tuples with JSON-safe values.

Value tags: ("dec", str) -> Decimal, ("float", "nan"|"inf"|"-inf"|num),
("date", y,m,d), ("datetime", y,mo,d,h,mi,s,us), ("tztime", ...same + utc offset min),
("time", h,mi,s,us).
"""

import datetime as _dt

CORE_LOCALES = [
    'en_US', 'de_DE', 'fr_FR', 'sv_SE', 'ar_EG', 'ja_JP', 'es_CO', 'pt_BR',
    'zh_Hans_CN', 'ar_SA', 'hi_IN', 'ru_RU', 'en_GB', 'fr_CA', 'de_AT',
    'it_IT', 'nl_NL', 'pl_PL', 'tr_TR', 'ko_KR', 'th_TH', 'he_IL',
    'fa_IR', 'bn_IN', 'root',
]

NUM_VALUES = [
    0, 1, -1, 7, 123, 1234567.891, -9876.54321, 0.5, 0.341, 25.1234,
    1099.9876, 1099.98, 99.5, 12345.678, 229291.1234, 0.07,
    ("dec", "3.141592653589793238462643383279"),
    ("dec", "-0"), ("dec", "0E-10"), ("dec", "1E+3"), ("dec", "-2.5E-4"),
    ("dec", "9999999999999999999999999999"),
    ("dec", "10000000000000000000000000000"),
    ("dec", "1E+9999"), ("dec", "1E+10000"), ("dec", "-0.000001"),
    ("float", "nan"), ("float", "inf"), ("float", "-inf"),
    ("dec", "NaN"), ("dec", "Infinity"), ("dec", "-Infinity"),
    "123.45", "1E+3", 10 ** 20, -(10 ** 20),
]

CURRENCIES = ['USD', 'EUR', 'JPY', 'CHF', 'GBP', 'XXX', 'EGP', 'COP']

NAMED = ['full', 'long', 'medium', 'short']

DATES = [
    ("date", 2007, 4, 1), ("date", 2000, 2, 29), ("date", 1999, 12, 31),
    ("date", 2026, 10, 28), ("date", 1, 1, 1), ("date", 9999, 12, 31),
    ("datetime", 2007, 4, 1, 15, 30, 0, 0),
    ("datetime", 2007, 4, 1, 0, 0, 0, 0),
    ("datetime", 2020, 6, 15, 23, 59, 59, 500000),
    ("datetime", 2007, 4, 1, 15, 30, 0, 0, 0),
    ("time", 15, 30, 0, 0), ("time", 0, 0, 0, 0), ("time", 23, 59, 59, 123000),
]

CUSTOM_PATTERNS = [
    "EEE, MMM d, ''yy", "hh 'o''clock' a", "yyyy.MM.dd G 'at' HH:mm:ss",
    "y-MM-dd", "HH:mm:ss", "EEEE, d. MMMM y", "d MMM y", "QQQ yyyy",
    "hh:mm a", "mm:ss.SSS", "DDD", "ww", "YYYY-'W'ww-e",
]


def _all_locales():
    from babel.localedata import locale_identifiers
    return sorted(locale_identifiers())


def build_cases(tier="deep"):
    """Yield (key, op, kwargs) with JSON-safe kwargs."""
    cases = []
    if tier == "deep":
        locales = CORE_LOCALES
    else:
        locales = _all_locales()

    for loc in locales:
        for v in NUM_VALUES:
            cases.append((f"num|{loc}|{v!r}", "decimal", {"v": v, "loc": loc}))
        for cur in CURRENCIES:
            cases.append((f"cur|{loc}|{cur}|std", "currency",
                          {"v": 1099.9876, "cur": cur, "loc": loc, "ft": "standard"}))
            cases.append((f"cur|{loc}|{cur}|acc", "currency",
                          {"v": -1099.9876, "cur": cur, "loc": loc, "ft": "accounting"}))
        cases.append((f"pct|{loc}", "percent", {"v": 0.341, "loc": loc}))
        cases.append((f"pct2|{loc}", "percent", {"v": 25.1234, "loc": loc}))
        if tier == "deep":
            for fmt in NAMED:
                cases.append((f"date|{loc}|{fmt}", "date",
                              {"d": DATES[0], "fmt": fmt, "loc": loc}))
                cases.append((f"time|{loc}|{fmt}", "time",
                              {"t": ("time", 15, 30, 0, 0), "fmt": fmt, "loc": loc}))
                cases.append((f"dt|{loc}|{fmt}", "datetime",
                              {"d": DATES[6], "fmt": fmt, "loc": loc}))
            for d in DATES[1:]:
                tag = "date" if d[0] in ("date", "datetime") else "time"
                op = {"date": "date", "datetime": "datetime", "time": "time"}[d[0]]
                cases.append((f"{op}|{loc}|{d}", op,
                              {("d" if op != "time" else "t"): d, "fmt": "medium", "loc": loc}))
            for p in CUSTOM_PATTERNS:
                cases.append((f"cdate|{loc}|{p}", "date",
                              {"d": DATES[0], "fmt": p, "loc": loc}))
        else:
            for fmt in NAMED:
                cases.append((f"date|{loc}|{fmt}", "date",
                              {"d": DATES[0], "fmt": fmt, "loc": loc}))
                cases.append((f"dt|{loc}|{fmt}", "datetime",
                              {"d": DATES[6], "fmt": fmt, "loc": loc}))
    if tier == "deep":
        # Option matrix on en_US + de_DE.
        for loc in ('en_US', 'de_DE', 'ar_EG'):
            for dq in (True, False):
                for gs in (True, False):
                    cases.append((f"opts|{loc}|{dq}|{gs}", "decimal",
                                  {"v": 1234567.8911, "loc": loc, "dq": dq, "gs": gs}))
            for ns in ('latn', 'default', 'arab', 'beng'):
                cases.append((f"ns|{loc}|{ns}", "decimal",
                              {"v": 1234567.891, "loc": loc, "ns": ns}))
            for expl, cur in [("USD", "#,##0.00 ¤¤"), ("EUR", "¤¤ #,##0.00")]:
                cases.append((f"expl|{loc}|{cur}", "currency_explicit",
                              {"v": 1099.98, "cur": expl, "loc": loc, "fmt": cur}))
            cases.append((f"namecur|{loc}", "currency",
                          {"v": 1099.98, "cur": "USD", "loc": loc, "ft": "name"}))
        # Error parity.
        cases.append(("err|badlocale", "decimal", {"v": 1.5, "loc": "xx_NOPE"}))
        cases.append(("err|badns", "decimal",
                      {"v": 1.5, "loc": "en_US", "ns": "bogus_ns"}))
        cases.append(("err|badcurtype", "currency",
                      {"v": 1.5, "cur": "USD", "loc": "en_US", "ft": "bogus"}))
        cases.append(("err|badfmt", "date",
                      {"d": DATES[0], "fmt": "bogus_format", "loc": "en_US"}))
        cases.append(("err|badinput", "decimal", {"v": "abc", "loc": "en_US"}))
        cases.append(("err|badbool", "decimal", {"v": True, "loc": "en_US"}))
        cases.append(("err|gfield", "date",
                      {"d": DATES[0], "fmt": "g", "loc": "en_US"}))
        cases.append(("err|badlen", "date",
                      {"d": DATES[0], "fmt": "MMMMMM", "loc": "en_US"}))
        cases.append(("err|badnumlen", "decimal_explicit",
                      {"v": 1.5, "loc": "en_US", "fmt": "@.0"}))
    return cases
