"""Execute equivalence cases under one babel implementation; dump JSONL.

Usage: equiv_run.py --tier deep|broad --out results.jsonl
The babel under test is whatever `import babel` resolves to (pristine
install, mirror fallback via BABEL_RUST_NO_RUST=1, or mirror + extension).
"""

import argparse
import datetime as _dt
import decimal
import json
import sys
import traceback

sys.path.insert(0, __import__('os').path.dirname(__import__('os').path.abspath(__file__)))
import equiv_cases as C


def build_value(v):
    if isinstance(v, tuple):
        tag = v[0]
        if tag == "dec":
            return decimal.Decimal(v[1])
        if tag == "float":
            return {"nan": float("nan"), "inf": float("inf"), "-inf": float("-inf")}[v[1]]
        if tag == "date":
            return _dt.date(*v[1:])
        if tag == "datetime":
            if len(v) == 9:
                return _dt.datetime(*v[1:8], tzinfo=_dt.timezone(_dt.timedelta(minutes=v[8])))
            return _dt.datetime(*v[1:])
        if tag == "time":
            return _dt.time(*v[1:])
    return v


def run_case(op, kw):
    from babel import dates as D
    from babel import numbers as N
    loc = kw.get("loc")
    if op == "decimal":
        return N.format_decimal(
            build_value(kw["v"]), locale=loc,
            decimal_quantization=kw.get("dq", True),
            group_separator=kw.get("gs", True),
            numbering_system=kw.get("ns", "latn"),
        )
    if op == "decimal_explicit":
        return N.format_decimal(build_value(kw["v"]), format=kw["fmt"], locale=loc)
    if op == "currency":
        return N.format_currency(
            build_value(kw.get("v", 1099.9876)), kw["cur"], locale=loc,
            format_type=kw.get("ft", "standard"),
        )
    if op == "currency_explicit":
        return N.format_currency(
            build_value(kw["v"]), kw["cur"], format=kw["fmt"], locale=loc)
    if op == "percent":
        return N.format_percent(build_value(kw["v"]), locale=loc)
    if op == "date":
        return D.format_date(build_value(kw["d"]), format=kw["fmt"], locale=loc)
    if op == "time":
        return D.format_time(build_value(kw["t"]), format=kw["fmt"], locale=loc)
    if op == "datetime":
        return D.format_datetime(build_value(kw["d"]), format=kw["fmt"], locale=loc)
    raise AssertionError(f"unknown op {op}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tier", default="deep")
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    import babel
    impl = getattr(babel, "__file__", "?")
    cases = C.build_cases(args.tier)
    n_ok = 0
    with open(args.out, "w", encoding="utf-8") as f:
        for key, op, kw in cases:
            try:
                val = run_case(op, kw)
                f.write(json.dumps({"key": key, "status": "ok", "value": val},
                                   ensure_ascii=False) + "\n")
                n_ok += 1
            except Exception as e:  # noqa: BLE001 - record, don't stop
                f.write(json.dumps({
                    "key": key, "status": "error",
                    "etype": f"{type(e).__module__}.{type(e).__name__}",
                    "emsg": str(e),
                }, ensure_ascii=False) + "\n")
    print(f"{args.tier}: {n_ok}/{len(cases)} ok [{impl}]")


main()
