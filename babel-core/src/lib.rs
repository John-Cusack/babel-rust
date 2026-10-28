//! Formatting kernels ported from `babel` (python-babel) 2.18.0.
//!
//! Scope: number/currency/percent formatting (`numbers.py`: `parse_pattern`,
//! `NumberPattern::apply` and its helpers) plus date/time formatting
//! (`dates.py`: `tokenize_pattern`, `DateTimePattern` apply). User-supplied
//! patterns, plural rules, compact/scientific formats, timezones and
//! day-period rules stay in Python behind the documented seam; anything
//! outside the kernel's exact domain returns [`Error::Fallback`] so the
//! Python shim replays the verbatim upstream logic.

pub mod bundle;
pub mod datetime;
pub mod number;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use bundle::Bundle;

/// Error type for kernel entry points.
///
/// `Fallback` means "outside the kernel's exact domain": the Python shim
/// must replay the verbatim upstream implementation. It is raised for
/// user-supplied patterns, compact/scientific formats, plural-driven
/// currency names, timezones, day-period rules, extreme magnitudes that
/// would engage the `decimal` module 28-digit context, and any missing
/// bundle ingredient. It is never raised for malformed *caller* input that
/// upstream also rejects — those errors are raised as the matching Python
/// exception type by the binding layer after running the identical check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Fallback,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Fallback => write!(
                f,
                "value outside the Rust kernel domain; use Python fallback"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Parsed-locale cache: locale identifier -> fully parsed bundle.
///
/// Filled once per locale by `ensure_locale` (patterns parsed eagerly);
/// formatting entry points only do a hash lookup plus pure string work.
/// The Python shim re-calls `ensure_locale` whenever the underlying locale
/// data object identity changes, so mutated locale data can never read
/// stale here.
static CACHE: LazyLock<RwLock<HashMap<String, Arc<Bundle>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

fn cache() -> &'static RwLock<HashMap<String, Arc<Bundle>>> {
    &CACHE
}

/// Parse and cache one locale bundle. Idempotent; re-parses when the
/// bundle JSON differs from the cached copy (locale data mutated).
pub fn ensure_locale(id: &str, bundle_json: &str) -> Result<bool, Error> {
    let bundle: bundle::RawBundle =
        serde_json::from_str(bundle_json).map_err(|_| Error::Fallback)?;
    let parsed = Bundle::parse(id, bundle)?;
    let mut cache = cache().write().map_err(|_| Error::Fallback)?;
    let changed = match cache.get(id) {
        Some(existing) => existing.as_ref() != &parsed,
        None => true,
    };
    if changed {
        cache.insert(id.to_string(), Arc::new(parsed));
    }
    Ok(changed)
}

/// Look up a cached bundle (clone is a refcount bump, not a deep copy).
pub fn bundle_for(id: &str) -> Result<Arc<Bundle>, Error> {
    cache()
        .read()
        .map_err(|_| Error::Fallback)?
        .get(id)
        .cloned()
        .ok_or(Error::Fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_miss_is_fallback() {
        assert_eq!(bundle_for("xx_NOPE"), Err(Error::Fallback));
    }
}
