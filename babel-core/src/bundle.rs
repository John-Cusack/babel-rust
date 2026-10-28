//! Locale bundle: the small flat ingredient set the Python shim extracts
//! once per locale from the installed `babel` data files (pattern strings,
//! symbols, names) and hands to the Rust core. The core parses all patterns
//! eagerly at `ensure_locale` time and caches them; per-call formatting
//! then crosses the FFI boundary with scalars only — never CLDR dicts.
//!
//! The bundle is JSON (built with stdlib `json` on the Python side). `None`
//! dict keys in babel (`decimal_formats[None]`, `percent_formats[None]`)
//! arrive as the string `"default"`.

use std::collections::HashMap;

use serde::Deserialize;

use crate::datetime::DtPattern;
use crate::number::{NumPattern, NumSymbols};
use crate::Error;

/// Raw bundle as received from Python (all strings).
#[derive(Debug, Clone, Deserialize)]
pub struct RawBundle {
    pub id: String,
    pub first_week_day: u8,
    pub min_week_days: u8,
    pub default_numbering_system: String,
    pub number_symbols: HashMap<String, HashMap<String, String>>,
    pub decimal_formats: HashMap<String, String>,
    pub currency_formats: HashMap<String, String>,
    pub percent_formats: HashMap<String, String>,
    pub date_formats: HashMap<String, String>,
    pub time_formats: HashMap<String, String>,
    pub datetime_formats: HashMap<String, String>,
    pub months: HashMap<String, HashMap<String, Vec<String>>>,
    pub days: HashMap<String, HashMap<String, Vec<String>>>,
    pub quarters: HashMap<String, HashMap<String, Vec<String>>>,
    pub eras: HashMap<String, Vec<String>>,
    pub periods: HashMap<String, HashMap<String, HashMap<String, String>>>,
}

/// Fully parsed bundle kept in the locale cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundle {
    pub id: String,
    pub first_week_day: u8,
    pub min_week_days: u8,
    pub default_numbering_system: String,
    pub number_symbols: HashMap<String, HashMap<String, String>>,
    pub decimal_default: NumPattern,
    pub currency: HashMap<String, NumPattern>,
    pub percent_default: NumPattern,
    pub date: HashMap<String, DtPattern>,
    pub time: HashMap<String, DtPattern>,
    /// Raw glue patterns (`{0}` = time, `{1}` = date); substitution uses
    /// chained single-pass `replace` exactly like upstream
    /// (`glue.replace('{0}', time).replace('{1}', date)`), preserving the
    /// edge case where inserted text itself contains braces.
    pub datetime_glue: HashMap<String, String>,
    pub months: HashMap<String, HashMap<String, Vec<String>>>,
    pub days: HashMap<String, HashMap<String, Vec<String>>>,
    pub quarters: HashMap<String, HashMap<String, Vec<String>>>,
    pub eras: HashMap<String, Vec<String>>,
    pub periods: HashMap<String, HashMap<String, HashMap<String, String>>>,
}

impl Bundle {
    pub fn parse(id: &str, raw: RawBundle) -> Result<Bundle, Error> {
        let decimal_default = raw
            .decimal_formats
            .get("default")
            .ok_or(Error::Fallback)
            .and_then(|p| NumPattern::parse(p).map_err(|_| Error::Fallback))?;
        let mut currency = HashMap::new();
        for (k, p) in &raw.currency_formats {
            currency.insert(
                k.clone(),
                NumPattern::parse(p).map_err(|_| Error::Fallback)?,
            );
        }
        let percent_default = raw
            .percent_formats
            .get("default")
            .ok_or(Error::Fallback)
            .and_then(|p| NumPattern::parse(p).map_err(|_| Error::Fallback))?;
        let mut date = HashMap::new();
        for (k, p) in &raw.date_formats {
            date.insert(k.clone(), DtPattern::parse(p)?);
        }
        let mut time = HashMap::new();
        for (k, p) in &raw.time_formats {
            time.insert(k.clone(), DtPattern::parse(p)?);
        }
        Ok(Bundle {
            id: id.to_string(),
            first_week_day: raw.first_week_day,
            min_week_days: raw.min_week_days,
            default_numbering_system: raw.default_numbering_system,
            number_symbols: raw.number_symbols,
            decimal_default,
            currency,
            percent_default,
            date,
            time,
            datetime_glue: raw.datetime_formats,
            months: raw.months,
            days: raw.days,
            quarters: raw.quarters,
            eras: raw.eras,
            periods: raw.periods,
        })
    }

    /// Symbols for a numbering system key (`latn`, `arab`, ...).
    pub fn symbols(&self, ns: &str) -> Result<NumSymbols, Error> {
        let m = self.number_symbols.get(ns).ok_or(Error::Fallback)?;
        let get = |k: &str| m.get(k).cloned().ok_or(Error::Fallback);
        Ok(NumSymbols {
            decimal: get("decimal")?,
            group: get("group")?,
            plus: get("plus")?,
            minus: get("minus")?,
            exponential: get("exponential")?,
            infinity: get("infinity")?,
        })
    }

    /// Resolve the effective numbering-system key, mirroring
    /// `numbers._get_numbering_system` (validation happens Python-side, so
    /// a miss here is a bug guard → fallback, never a new error).
    pub fn resolve_ns<'a>(&'a self, ns: &'a str) -> Result<&'a str, Error> {
        if ns == "default" {
            Ok(&self.default_numbering_system)
        } else if self.number_symbols.contains_key(ns) {
            Ok(ns)
        } else {
            Err(Error::Fallback)
        }
    }
}
