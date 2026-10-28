//! Python bindings for the babel formatting hot path (`babel._core`).
//!
//! Thin entry points: the Python shim (`babel/_rust.py`) performs the
//! verbatim input conversion (so conversion errors are identical by
//! construction), resolves locale bundles once per locale, and passes only
//! scalars/strings per call. Anything outside the kernel domain raises
//! `FallbackError`, which the shim catches to replay the upstream logic.

use babel_core::{bundle_for, datetime as dt, ensure_locale, number as num, Error};
use pyo3::prelude::*;
use pyo3::{create_exception, exceptions::PyValueError};

create_exception!(babel_core, FallbackError, pyo3::exceptions::PyException);

fn fallback(e: Error) -> PyErr {
    match e {
        Error::Fallback => FallbackError::new_err("outside the Rust kernel domain"),
    }
}

#[pyfunction]
fn ensure_locale_py(bundle_json: &str) -> PyResult<bool> {
    let raw: serde_json::Value =
        serde_json::from_str(bundle_json).map_err(|e| PyValueError::new_err(e.to_string()))?;
    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| PyValueError::new_err("bundle lacks string 'id'"))?;
    ensure_locale(id, bundle_json).map_err(fallback)
}

/// Shared number entry: `kind` selects the pattern (`decimal`,
/// `currency:<format_type>`, `percent`).
#[allow(clippy::too_many_arguments)]
fn format_number(
    locale_id: &str,
    kind: &str,
    value: &str,
    symbol: Option<&str>,
    code: Option<&str>,
    prec: Option<u32>,
    currency_digits: bool,
    decimal_quantization: bool,
    group_separator: bool,
    ns: &str,
) -> Result<String, Error> {
    let bundle = bundle_for(locale_id)?;
    let ns_key = bundle.resolve_ns(ns)?.to_string();
    let sym = bundle.symbols(&ns_key)?;
    let pat = if kind == "decimal" {
        &bundle.decimal_default
    } else if kind == "percent" {
        &bundle.percent_default
    } else if let Some(ft) = kind.strip_prefix("currency:") {
        bundle.currency.get(ft).ok_or(Error::Fallback)?
    } else {
        return Err(Error::Fallback);
    };
    // Kernel-domain triggers (mirror the `decimal` module limits the
    // fallback would otherwise engage): >28 significant digits, absurd
    // magnitudes, signaling NaNs, NaN payloads.
    let dec = num::Dec::parse(value)?;
    match &dec {
        num::Dec::Nan { .. } => {
            if value.trim_start_matches(['-', '+']).len() > 3 {
                return Err(Error::Fallback);
            }
        }
        num::Dec::Num { int, frac, .. } => {
            let t = format!("{int}{frac}");
            let sig = t.trim_start_matches('0');
            let sig_len = if sig.is_empty() { 1 } else { sig.len() };
            if sig_len > 28 {
                return Err(Error::Fallback);
            }
            let adj = dec.adjusted();
            if adj.abs() > 9999 {
                return Err(Error::Fallback);
            }
        }
        num::Dec::Inf { .. } => {}
    }
    // Signaling NaN arrives as "snan"/"sNaN": not parseable above... it
    // falls into the mantissa check; guard explicitly anyway.
    if matches!(dec, num::Dec::Nan { .. })
        && value.to_ascii_lowercase().trim_start_matches(['-', '+']) != "nan"
    {
        return Err(Error::Fallback);
    }
    let (nat, sig_len, f0_len, is_zero) = match &dec.normalized() {
        num::Dec::Num { int, frac, .. } => {
            let zero = int == "0" && frac.bytes().all(|b| b == b'0');
            let t = format!("{int}{frac}");
            let t = t.trim_start_matches('0');
            (
                frac.len() as u32,
                if t.is_empty() { 1 } else { t.len() },
                frac.len(),
                zero,
            )
        }
        _ => (0, 0, 0, false),
    };
    // Quantize-width guard: under the `decimal` module's default context
    // (prec=28) a quantized result wider than 28 significant digits raises
    // InvalidOperation. Bound the result width conservatively
    // (sig + appended zeros + 1 for a rounding carry); overshoot takes the
    // verbatim fallback, which raises exactly like upstream.
    // Resolve the target max fraction digits first (same selection as below).
    let f_target = {
        let base = match prec {
            Some(pp) if currency_digits => pp,
            _ => pat.frac_prec.1,
        };
        if decimal_quantization {
            base
        } else {
            base.max(nat)
        }
    };
    if !is_zero && sig_len + f_target.saturating_sub(f0_len as u32) as usize + 1 > 28 {
        return Err(Error::Fallback);
    }
    let mut frac_sel = if let Some(p) = prec {
        if currency_digits {
            Some((p, p))
        } else {
            None
        }
    } else {
        None
    };
    if !decimal_quantization {
        let base = frac_sel.unwrap_or(pat.frac_prec);
        frac_sel = Some((base.0, base.1.max(nat)));
    }
    let currency = match (symbol, code) {
        (Some(s), Some(c)) => Some((s, c)),
        _ => None,
    };
    num::apply(pat, value, currency, frac_sel, group_separator, &sym)
}

#[pyfunction]
#[pyo3(signature = (locale_id, value, decimal_quantization=true, group_separator=true, ns="latn"))]
fn format_decimal(
    locale_id: &str,
    value: &str,
    decimal_quantization: bool,
    group_separator: bool,
    ns: &str,
) -> PyResult<String> {
    format_number(
        locale_id,
        "decimal",
        value,
        None,
        None,
        None,
        true,
        decimal_quantization,
        group_separator,
        ns,
    )
    .map_err(fallback)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
#[pyo3(signature = (locale_id, value, symbol, code, prec, currency_digits=true, decimal_quantization=true, group_separator=true, ns="latn", format_type="standard"))]
fn format_currency(
    locale_id: &str,
    value: &str,
    symbol: &str,
    code: &str,
    prec: u32,
    currency_digits: bool,
    decimal_quantization: bool,
    group_separator: bool,
    ns: &str,
    format_type: &str,
) -> PyResult<String> {
    format_number(
        locale_id,
        &format!("currency:{format_type}"),
        value,
        Some(symbol),
        Some(code),
        Some(prec),
        currency_digits,
        decimal_quantization,
        group_separator,
        ns,
    )
    .map_err(fallback)
}

#[pyfunction]
#[pyo3(signature = (locale_id, value, decimal_quantization=true, group_separator=true, ns="latn"))]
fn format_percent(
    locale_id: &str,
    value: &str,
    decimal_quantization: bool,
    group_separator: bool,
    ns: &str,
) -> PyResult<String> {
    format_number(
        locale_id,
        "percent",
        value,
        None,
        None,
        None,
        true,
        decimal_quantization,
        group_separator,
        ns,
    )
    .map_err(fallback)
}

#[pyfunction]
fn format_date(
    locale_id: &str,
    fmt_name: &str,
    year: i32,
    month: u8,
    day: u8,
    weekday_mon0: u8,
) -> PyResult<String> {
    (|| {
        let bundle = bundle_for(locale_id)?;
        let pat = bundle.date.get(fmt_name).ok_or(Error::Fallback)?;
        dt::apply(
            &bundle,
            pat,
            dt::Parts {
                date: Some((year, month, day, weekday_mon0)),
                time: None,
            },
        )
    })()
    .map_err(fallback)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn format_time(
    locale_id: &str,
    fmt_name: &str,
    hour: u8,
    minute: u8,
    second: u8,
    microsecond: u32,
) -> PyResult<String> {
    (|| {
        let bundle = bundle_for(locale_id)?;
        let pat = bundle.time.get(fmt_name).ok_or(Error::Fallback)?;
        dt::apply(
            &bundle,
            pat,
            dt::Parts {
                date: None,
                time: Some((hour, minute, second, microsecond)),
            },
        )
    })()
    .map_err(fallback)
}

#[pyfunction]
#[allow(clippy::too_many_arguments)]
fn format_datetime(
    locale_id: &str,
    fmt_name: &str,
    year: i32,
    month: u8,
    day: u8,
    weekday_mon0: u8,
    hour: u8,
    minute: u8,
    second: u8,
    microsecond: u32,
) -> PyResult<String> {
    (|| {
        let bundle = bundle_for(locale_id)?;
        let glue = bundle.datetime_glue.get(fmt_name).ok_or(Error::Fallback)?;
        let dpat = bundle.date.get(fmt_name).ok_or(Error::Fallback)?;
        let tpat = bundle.time.get(fmt_name).ok_or(Error::Fallback)?;
        let parts = dt::Parts {
            date: Some((year, month, day, weekday_mon0)),
            time: Some((hour, minute, second, microsecond)),
        };
        let date_s = dt::apply(&bundle, dpat, parts)?;
        let time_s = dt::apply(&bundle, tpat, parts)?;
        // Verbatim: upstream strips quotes from the glue first
        // (`get_datetime_format(...).replace("'", "")`), then chained
        // single-pass substitution of time ({0}) and date ({1}).
        let unquoted: String = glue.chars().filter(|&c| c != '\'').collect();
        Ok::<String, Error>(unquoted.replace("{0}", &time_s).replace("{1}", &date_s))
    })()
    .map_err(fallback)
}

#[pymodule]
fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(ensure_locale_py, m)?)?;
    m.add_function(wrap_pyfunction!(format_decimal, m)?)?;
    m.add_function(wrap_pyfunction!(format_currency, m)?)?;
    m.add_function(wrap_pyfunction!(format_percent, m)?)?;
    m.add_function(wrap_pyfunction!(format_date, m)?)?;
    m.add_function(wrap_pyfunction!(format_time, m)?)?;
    m.add_function(wrap_pyfunction!(format_datetime, m)?)?;
    m.add("FallbackError", m.py().get_type::<FallbackError>())?;
    Ok(())
}
