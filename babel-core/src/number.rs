//! Number formatting kernel: port of `babel.numbers.parse_pattern`,
//! `NumberPattern.apply` and its helpers (`_quantize_value`, `_format_int`,
//! `_format_frac`, `_format_significant`, `scientific_notation_elements`,
//! `parse_grouping`), operating on canonical decimal strings.
//!
//! The Python shim performs the verbatim input conversion
//! (`Decimal(str(value))` unless already `Decimal`, so conversion errors
//! are identical by construction) and passes `str(d)`; values engaging the
//! `decimal` module's 28-digit context, signaling NaNs, or NaNs with payload
//! take the Python fallback before reaching this module.

use crate::Error;

/// Canonical decimal value parsed from `str(Decimal)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dec {
    Nan {
        neg: bool,
    },
    Inf {
        neg: bool,
    },
    Num {
        neg: bool,
        int: String,
        frac: String,
    },
}

impl Dec {
    /// Parse the output of `str(decimal.Decimal)`: optional sign, digits
    /// with optional `.` fraction and optional `E[+-]digits` exponent, or
    /// `NaN`/`Infinity` (any case, optional sign).
    pub fn parse(s: &str) -> Result<Dec, Error> {
        let (neg, rest) = match s.strip_prefix(['-', '+']) {
            Some(r) => (s.starts_with('-'), r),
            None => (false, s),
        };
        let lower = rest.to_ascii_lowercase();
        if lower == "nan" {
            return Ok(Dec::Nan { neg });
        }
        if lower == "infinity" || lower == "inf" {
            return Ok(Dec::Inf { neg });
        }
        // Split exponent.
        let (mant, exp): (String, i64) = match rest.split_once(['E', 'e']) {
            Some((m, e)) => {
                let e: i64 = e.parse().map_err(|_| Error::Fallback)?;
                (m.to_string(), e)
            }
            None => (rest.to_string(), 0),
        };
        // Validate mantissa shape: digits with at most one '.'.
        let (ip, fp) = match mant.split_once('.') {
            Some((a, b)) => (a, b),
            None => (mant.as_str(), ""),
        };
        if ip.is_empty() && fp.is_empty() {
            return Err(Error::Fallback);
        }
        if !ip.bytes().all(|b| b.is_ascii_digit()) || !fp.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::Fallback);
        }
        // Normalize: strip leading zeros of the integer part.
        let int = ip.trim_start_matches('0');
        let int = if int.is_empty() { "0" } else { int }.to_string();
        Dec::Num {
            neg,
            int,
            frac: fp.to_string(),
        }
        .shifted(exp)
    }

    /// Shift the decimal point by `k` (scaleb), keeping (int, frac) form.
    ///
    /// Shifts beyond ±2M digits are rejected: the shim guarantees kernel
    /// inputs stay far below that (extreme magnitudes take the Python
    /// fallback); this guard only protects direct callers from huge
    /// allocations.
    fn shifted(self, k: i64) -> Result<Dec, Error> {
        const LIMIT: i64 = 2_000_000;
        if k.abs() > LIMIT {
            return Err(Error::Fallback);
        }
        match self {
            Dec::Num { neg, int, frac } => {
                if k == 0 {
                    return Ok(Dec::Num { neg, int, frac });
                }
                let mut digits = format!("{int}{frac}");
                // Position of the point measured from the left, in digits.
                let mut point = int.len() as i64 + k;
                // Strip leading zeros for the combined digits.
                let stripped = digits.trim_start_matches('0');
                let dropped = digits.len() - stripped.len();
                digits = stripped.to_string();
                point -= dropped as i64;
                if digits.is_empty() {
                    return Ok(Dec::Num {
                        neg,
                        int: "0".into(),
                        frac: String::new(),
                    });
                }
                if point <= 0 {
                    let zeros = (-point) as usize;
                    if zeros > LIMIT as usize {
                        return Err(Error::Fallback);
                    }
                    Ok(Dec::Num {
                        neg,
                        int: "0".into(),
                        frac: format!("{}{}", "0".repeat(zeros), digits),
                    })
                } else if point as usize >= digits.len() {
                    let zeros = point as usize - digits.len();
                    if zeros > LIMIT as usize {
                        return Err(Error::Fallback);
                    }
                    Ok(Dec::Num {
                        neg,
                        int: format!("{}{}", digits, "0".repeat(zeros)),
                        frac: String::new(),
                    })
                } else {
                    let (a, b) = digits.split_at(point as usize);
                    Ok(Dec::Num {
                        neg,
                        int: a.into(),
                        frac: b.into(),
                    })
                }
            }
            other => Ok(other),
        }
    }

    /// Strip trailing fractional zeros (Decimal.normalize for our domain;
    /// caller guarantees no context engagement: <= 28 significant digits).
    pub fn normalized(self) -> Dec {
        match self {
            Dec::Num { neg, int, frac } => {
                let f = frac.trim_end_matches('0');
                Dec::Num {
                    neg,
                    int,
                    frac: f.into(),
                }
            }
            other => other,
        }
    }

    /// Significant-digit count (normalized digits, zero counts as 1).
    pub fn sig_digits(&self) -> usize {
        match self {
            Dec::Nan { .. } | Dec::Inf { .. } => 0,
            Dec::Num { int, frac, .. } => {
                let t = format!("{int}{frac}");
                let t = t.trim_start_matches('0');
                if t.is_empty() {
                    1
                } else {
                    t.len()
                }
            }
        }
    }

    /// Adjusted exponent: position of the most significant digit
    /// (Decimal.adjusted; zero reports 0, matching CPython).
    pub fn adjusted(&self) -> i64 {
        match self {
            Dec::Nan { .. } | Dec::Inf { .. } => 0,
            Dec::Num { int, frac, .. } => {
                if int == "0" {
                    if frac.bytes().all(|b| b == b'0') {
                        return 0;
                    }
                    // Pure fraction: count leading fractional zeros.
                    let lz = frac.bytes().take_while(|&b| b == b'0').count();
                    -(lz as i64) - 1
                } else {
                    int.len() as i64 - 1
                }
            }
        }
    }
}

/// Round half-even the digit string `int.frac` to `keep` fractional digits.
/// Returns (int, frac) with frac.len() == keep.
fn round_half_even(int: &str, frac: &str, keep: usize) -> (String, String) {
    if frac.len() <= keep {
        let mut f = frac.to_string();
        f.push_str(&"0".repeat(keep - frac.len()));
        return (int.to_string(), f);
    }
    let (k, rest) = frac.split_at(keep);
    let drop_first = rest.as_bytes()[0];
    let rest_nonzero = rest[1..].bytes().any(|b| b != b'0');
    let round_up = drop_first > b'5'
        || (drop_first == b'5'
            && (rest_nonzero || {
                let last_kept = if keep == 0 {
                    int.as_bytes().last().copied().unwrap_or(b'0')
                } else {
                    k.as_bytes().last().copied().unwrap_or(b'0')
                };
                (last_kept - b'0') % 2 == 1
            }));
    if !round_up {
        return (int.to_string(), k.to_string());
    }
    // Add one at position `keep`.
    let mut digits: Vec<u8> = format!("{int}{k}").bytes().collect();
    let mut i = digits.len();
    loop {
        if i == 0 {
            digits.insert(0, b'1');
            break;
        }
        i -= 1;
        if digits[i] == b'9' {
            digits[i] = b'0';
        } else {
            digits[i] += 1;
            break;
        }
    }
    let s = String::from_utf8(digits).expect("digits");
    if keep == 0 {
        (s, String::new())
    } else {
        let at = s.len() - keep;
        (s[..at].to_string(), s[at..].to_string())
    }
}

/// Parsed LDML number pattern: port of `parse_pattern` + `NumberPattern`.
impl NumPattern {
    /// Parse a number pattern string (see [`parse_pattern`]).
    pub fn parse(pattern: &str) -> Result<NumPattern, Error> {
        parse_pattern(pattern)
    }
}

/// Parsed LDML number pattern: port of `parse_pattern` + `NumberPattern`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumPattern {
    pub prefix: (String, String),
    pub suffix: (String, String),
    pub grouping: (u32, u32),
    pub int_prec: (u32, u32),
    pub frac_prec: (u32, u32),
    pub exp_prec: Option<(u32, u32)>,
    pub exp_plus: bool,
    pub scale: u32,
    pub significant: bool,
}

fn parse_precision(p: &str) -> (u32, u32) {
    let (mut min, mut max) = (0u32, 0u32);
    for c in p.chars() {
        match c {
            '@' | '0' => {
                min += 1;
                max += 1;
            }
            '#' => max += 1,
            ',' => {}
            _ => break,
        }
    }
    (min, max)
}

/// Port of `parse_grouping`.
pub fn parse_grouping(p: &str) -> (u32, u32) {
    let width = p.len();
    let g1 = match p.rfind(',') {
        None => return (1000, 1000),
        Some(i) => (width - i - 1) as u32,
    };
    let head = &p[..p.len() - g1 as usize - 1];
    match head.rfind(',') {
        None => (g1, g1),
        Some(g2) => {
            let g2 = (width - g1 as usize - g2 - 2) as u32;
            (g1, g2)
        }
    }
}

/// Split one subpattern into (prefix, number, suffix), replicating
/// `_number_pattern_re = (prefix)(number)(suffix)` matched at position 0:
/// greedy quoted/plain prefix, then a `[0-9@#.,E+]*` run, then the rest.
fn match_number(sub: &str) -> (String, String, String) {
    let bytes = sub.as_bytes();
    // Greedy prefix: atoms of `'[^']*'` or single [^'0-9@#.,].
    let mut prefix_end = 0usize;
    let mut j = 0usize;
    while j < bytes.len() {
        let c = bytes[j];
        if c == b'\'' {
            match sub[j + 1..].find('\'') {
                Some(k) => {
                    j = j + 1 + k + 1;
                    prefix_end = j;
                }
                None => break,
            }
        } else if c.is_ascii_digit() || c == b'@' || c == b'#' || c == b'.' || c == b',' {
            break;
        } else {
            j += 1;
            prefix_end = j;
        }
    }
    let prefix = sub[..prefix_end].to_string();
    // Number run.
    let mut n = prefix_end;
    while n < bytes.len() {
        let c = bytes[n];
        if c.is_ascii_digit()
            || c == b'@'
            || c == b'#'
            || c == b'.'
            || c == b','
            || c == b'E'
            || c == b'+'
        {
            n += 1;
        } else {
            break;
        }
    }
    (prefix, sub[prefix_end..n].to_string(), sub[n..].to_string())
}

/// Port of `parse_pattern(pattern: str)`.
///
/// Scientific (`E`) and significant-digit (`@`) patterns parse
/// successfully; their *application* stays in Python (fallback) because
/// the kernel's named-format domain never contains them.
pub fn parse_pattern(pattern: &str) -> Result<NumPattern, Error> {
    let (pos_prefix, number, pos_suffix, neg_prefix, neg_suffix);
    if let Some(semi) = pattern.find(';') {
        let (pos, neg) = pattern.split_at(semi);
        let neg = &neg[1..];
        let (pp, num, ps) = match_number(pos);
        let (np, _, ns) = match_number(neg);
        pos_prefix = pp;
        number = num;
        pos_suffix = ps;
        neg_prefix = np;
        neg_suffix = ns;
    } else {
        let (pp, num, ps) = match_number(pattern);
        neg_prefix = format!("-{pp}");
        pos_prefix = pp;
        number = num;
        pos_suffix = ps.clone();
        neg_suffix = ps;
    }
    let (number, exp) = match number.split_once('E') {
        Some((a, b)) => (a.to_string(), Some(b.to_string())),
        None => (number, None),
    };
    if number.contains('@') && number.contains('.') && number.contains('0') {
        return Err(Error::Fallback);
    }
    let (integer, fraction) = match number.rsplit_once('.') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (number.clone(), String::new()),
    };
    let int_prec = parse_precision(&integer);
    let frac_prec = parse_precision(&fraction);
    let (exp_prec, exp_plus) = match exp {
        Some(e) => {
            let plus = e.starts_with('+');
            let e = e.trim_start_matches('+');
            (Some(parse_precision(e)), plus)
        }
        None => (None, false),
    };
    let grouping = parse_grouping(&integer);
    let affixes = format!("{pos_prefix}{pos_suffix}{neg_prefix}{neg_suffix}");
    let scale = if affixes.contains('%') {
        2
    } else if affixes.contains('‰') {
        3
    } else {
        0
    };
    Ok(NumPattern {
        prefix: (pos_prefix, neg_prefix),
        suffix: (pos_suffix, neg_suffix),
        grouping,
        int_prec,
        frac_prec,
        exp_prec,
        exp_plus,
        scale,
        significant: pattern.contains('@'),
    })
}

/// Symbols needed by the number kernel (one numbering system).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NumSymbols {
    pub decimal: String,
    pub group: String,
    pub plus: String,
    pub minus: String,
    pub exponential: String,
    pub infinity: String,
}

/// Quote handling: port of `re.sub(r"'([^']*)'", lambda m: m.group(1) or "'",
/// retval)`.
fn unquote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.split('\'');
    // Text before the first quote passes through.
    out.push_str(it.next().unwrap_or(""));
    // Remaining pieces come in (inner, outer) pairs.
    while let Some(inner) = it.next() {
        match it.next() {
            Some(o) => {
                out.push_str(if inner.is_empty() { "'" } else { inner });
                out.push_str(o);
            }
            None => {
                // Trailing unmatched quote: kept verbatim (the regex would
                // not match it either).
                out.push('\'');
                out.push_str(inner);
            }
        }
    }
    out
}

/// Port of `NumberPattern._format_int`.
fn format_int(value: &str, min: u32, grouping: (u32, u32), group_sym: &str) -> String {
    let mut value = value.to_string();
    if value.len() < min as usize {
        value = format!("{}{value}", "0".repeat(min as usize - value.len()));
    }
    let (mut gsize, g2) = (grouping.0 as usize, grouping.1 as usize);
    let mut ret = String::new();
    while value.len() > gsize {
        ret = format!("{}{}{ret}", group_sym, &value[value.len() - gsize..]);
        value.truncate(value.len() - gsize);
        gsize = g2;
    }
    format!("{value}{ret}")
}

/// Port of `NumberPattern._quantize_value` (non-scientific tail).
#[allow(clippy::too_many_arguments)]
fn quantize_value(
    int: &str,
    frac: &str,
    frac_prec: (u32, u32),
    group_separator: bool,
    pat: &NumPattern,
    sym: &NumSymbols,
) -> String {
    let (ri, rf) = round_half_even(int, frac, frac_prec.1 as usize);
    let integer_part = if group_separator {
        format_int(&ri, pat.int_prec.0, pat.grouping, &sym.group)
    } else {
        let mut v = ri;
        if v.len() < pat.int_prec.0 as usize {
            v = format!("{}{v}", "0".repeat(pat.int_prec.0 as usize - v.len()));
        }
        v
    };
    // Port of `_format_frac`.
    let (fmin, _) = frac_prec;
    let mut f = rf;
    if f.len() < fmin as usize {
        f = format!("{f}{}", "0".repeat(fmin as usize - f.len()));
    }
    let frac_out = if frac_prec.1 == 0 || (fmin == 0 && f.bytes().all(|b| b == b'0')) {
        String::new()
    } else {
        let mut t = f;
        while t.len() > fmin as usize && t.ends_with('0') {
            t.pop();
        }
        format!("{}{t}", sym.decimal)
    };
    format!("{integer_part}{frac_out}")
}

/// Port of `NumberPattern.apply` for the kernel domain.
///
/// * `pat` — parsed pattern (locale default for the requested kind).
/// * `value` — canonical `str(Decimal)` of the verbatim-converted input.
/// * `currency` — `(symbol, CODE)` when a currency applies, else `None`.
///   Patterns containing `¤¤¤` (plural-driven long names) are rejected by
///   the caller (fallback), so only `¤`/`¤¤` are substituted here.
/// * `frac_override` — resolved `(min, max)` when the caller overrides the
///   pattern precision (currency digits, or quantization-disabled
///   widening); `None` selects `pat.frac_prec`.
#[allow(clippy::too_many_arguments)]
pub fn apply(
    pat: &NumPattern,
    value: &str,
    currency: Option<(&str, &str)>,
    frac_override: Option<(u32, u32)>,
    group_separator: bool,
    sym: &NumSymbols,
) -> Result<String, Error> {
    let dec = Dec::parse(value)?;
    // Sign like upstream: `is_negative = int(value.is_signed())` read
    // before `abs()`; `-0` therefore takes the negative prefix.
    let is_neg = match &dec {
        Dec::Nan { neg } | Dec::Inf { neg } => *neg,
        Dec::Num { neg, .. } => *neg,
    };
    // Mirror the upstream guards: infinite values bypass quantization and
    // render the infinity symbol, but still flow through affixes, currency
    // substitution and quote handling. NaN flows through the digit path
    // (`f"{nan:f}" == "nan"`, grouped as a plain string).
    // scaleb(scale) then abs().normalize().
    let dec = dec.shifted(pat.scale as i64)?.normalized();
    let number = match dec {
        Dec::Inf { .. } => sym.infinity.clone(),
        Dec::Nan { .. } => quantize_value("NaN", "", (0, 0), group_separator, pat, sym),
        Dec::Num {
            ref int, ref frac, ..
        } => {
            if pat.exp_prec.is_some() || pat.significant {
                // Scientific and significant-digit application stay in
                // Python: the kernel's named-format domain never contains
                // them, and the fallback replays the verbatim logic.
                return Err(Error::Fallback);
            }
            let frac_prec = frac_override.unwrap_or(pat.frac_prec);
            quantize_value(int, frac, frac_prec, group_separator, pat, sym)
        }
    };
    let mut retval = format!("{}{}{}", prefix(pat, is_neg), number, suffix(pat, is_neg));
    if retval.contains('¤') {
        let (symbol, code) = currency.ok_or(Error::Fallback)?;
        // Order matters (upstream): ¤¤¤ first — but the caller guarantees
        // no ¤¤¤ remains; encountering one here is a bug guard.
        if retval.contains("¤¤¤") {
            return Err(Error::Fallback);
        }
        retval = retval.replace("¤¤", code);
        retval = retval.replace('¤', symbol);
    }
    Ok(unquote(&retval))
}

fn prefix(pat: &NumPattern, neg: bool) -> &str {
    if neg {
        &pat.prefix.1
    } else {
        &pat.prefix.0
    }
}

fn suffix(pat: &NumPattern, neg: bool) -> &str {
    if neg {
        &pat.suffix.1
    } else {
        &pat.suffix.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouping_parse() {
        assert_eq!(parse_grouping("##"), (1000, 1000));
        assert_eq!(parse_grouping("#,###"), (3, 3));
        assert_eq!(parse_grouping("#,####,###"), (3, 4));
    }

    #[test]
    fn match_number_basic() {
        assert_eq!(
            match_number("¤#,##0.00"),
            ("¤".into(), "#,##0.00".into(), "".into())
        );
        assert_eq!(
            match_number("#,##0.00"),
            ("".into(), "#,##0.00".into(), "".into())
        );
    }

    #[test]
    fn round_half_even_cases() {
        assert_eq!(round_half_even("1", "5", 0), ("2".into(), "".into()));
        assert_eq!(round_half_even("2", "5", 0), ("2".into(), "".into()));
        assert_eq!(round_half_even("1", "25", 1), ("1".into(), "2".into()));
        assert_eq!(round_half_even("1", "35", 1), ("1".into(), "4".into()));
        assert_eq!(round_half_even("9", "99", 1), ("10".into(), "0".into()));
    }

    #[test]
    fn dec_shift() {
        assert_eq!(
            Dec::parse("123.45").unwrap().shifted(2).unwrap(),
            Dec::Num {
                neg: false,
                int: "12345".into(),
                frac: "".into()
            }
        );
        assert_eq!(
            Dec::parse("1").unwrap().shifted(-3).unwrap(),
            Dec::Num {
                neg: false,
                int: "0".into(),
                frac: "001".into()
            }
        );
    }
}
