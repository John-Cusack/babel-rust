//! Date/time formatting kernel: port of `babel.dates.tokenize_pattern`,
//! `parse_pattern` (validation half) and `DateTimeFormat.__getitem__`
//! field dispatch, operating on pre-decomposed date/time components.
//!
//! Out of scope (Python fallback): timezone fields (`z Z O v V x X`),
//! day-period rules (`b B`), custom user patterns (only locale named
//! formats live in the bundle), the `g` field (upstream `KeyError`), and
//! fractional seconds with nonzero microseconds (float banker's rounding).

use crate::bundle::Bundle;
use crate::Error;

/// A parsed datetime pattern: literal runs and (field, count) pairs.
/// Mirrors `tokenize_pattern` output; the `%%`-doubling of the Python
/// implementation is omitted because it only served the `%` operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DtToken {
    Chars(String),
    Field(char, u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DtPattern {
    pub raw: String,
    pub tokens: Vec<DtToken>,
}

/// Allowed repetition counts per field char (`None` = any), mirroring
/// `dates.PATTERN_CHARS` (only the subset relevant to tokenizing; zone
/// fields tokenize fine and are rejected at apply time).
fn allowed(char: char) -> Option<&'static [u8]> {
    match char {
        'G' => Some(&[1, 2, 3, 4, 5]),
        'y' | 'Y' | 'u' => None,
        'Q' | 'q' => Some(&[1, 2, 3, 4, 5]),
        'M' | 'L' => Some(&[1, 2, 3, 4, 5]),
        'w' => Some(&[1, 2]),
        'W' => Some(&[1]),
        'd' => Some(&[1, 2]),
        'D' => Some(&[1, 2, 3]),
        'F' => Some(&[1]),
        'g' => None,
        'E' => Some(&[1, 2, 3, 4, 5, 6]),
        'e' => Some(&[1, 2, 3, 4, 5, 6]),
        'c' => Some(&[1, 3, 4, 5, 6]),
        'a' | 'b' | 'B' => Some(&[1, 2, 3, 4, 5]),
        'h' | 'H' | 'K' | 'k' => Some(&[1, 2]),
        'm' => Some(&[1, 2]),
        's' => Some(&[1, 2]),
        'S' => None,
        'A' => None,
        'z' => Some(&[1, 2, 3, 4]),
        'Z' => Some(&[1, 2, 3, 4, 5]),
        'O' => Some(&[1, 4]),
        'v' => Some(&[1, 4]),
        'V' => Some(&[1, 2, 3, 4]),
        'x' => Some(&[1, 2, 3, 4, 5]),
        'X' => Some(&[1, 2, 3, 4, 5]),
        _ => None,
    }
}

fn is_pattern_char(c: char) -> bool {
    matches!(
        c,
        'G' | 'y'
            | 'Y'
            | 'u'
            | 'Q'
            | 'q'
            | 'M'
            | 'L'
            | 'w'
            | 'W'
            | 'd'
            | 'D'
            | 'F'
            | 'g'
            | 'E'
            | 'e'
            | 'c'
            | 'a'
            | 'b'
            | 'B'
            | 'h'
            | 'H'
            | 'K'
            | 'k'
            | 'm'
            | 's'
            | 'S'
            | 'A'
            | 'z'
            | 'Z'
            | 'O'
            | 'v'
            | 'V'
            | 'x'
            | 'X'
    )
}

impl DtPattern {
    /// Port of `tokenize_pattern` + the validation half of `parse_pattern`.
    pub fn parse(pattern: &str) -> Result<DtPattern, Error> {
        // `pattern.replace("''", '\0')`, then a quote-aware scan.
        let pre: String = pattern.replace("''", "\0");
        let mut tokens: Vec<(bool, String, char, u8)> = Vec::new();
        let mut charbuf = String::new();
        let mut fieldchar = '\0';
        let mut fieldnum: u8 = 0;
        let mut quotebuf: Option<String> = None;

        let flush_chars = |charbuf: &mut String, tokens: &mut Vec<(bool, String, char, u8)>| {
            if !charbuf.is_empty() {
                tokens.push((false, charbuf.replace('\0', "'"), '\0', 0));
                charbuf.clear();
            }
        };
        let flush_field = |fieldchar: &mut char,
                           fieldnum: &mut u8,
                           tokens: &mut Vec<(bool, String, char, u8)>| {
            if *fieldchar != '\0' {
                tokens.push((true, String::new(), *fieldchar, *fieldnum));
                *fieldchar = '\0';
                *fieldnum = 0;
            }
        };

        for ch in pre.chars() {
            if quotebuf.is_none() {
                if ch == '\'' {
                    flush_field(&mut fieldchar, &mut fieldnum, &mut tokens);
                    flush_chars(&mut charbuf, &mut tokens);
                    quotebuf = Some(String::new());
                } else if is_pattern_char(ch) {
                    flush_chars(&mut charbuf, &mut tokens);
                    if ch == fieldchar {
                        fieldnum = fieldnum.saturating_add(1);
                    } else {
                        flush_field(&mut fieldchar, &mut fieldnum, &mut tokens);
                        fieldchar = ch;
                        fieldnum = 1;
                    }
                } else {
                    flush_field(&mut fieldchar, &mut fieldnum, &mut tokens);
                    charbuf.push(ch);
                }
            } else if ch == '\'' {
                charbuf.push_str(quotebuf.as_deref().unwrap_or(""));
                quotebuf = None;
            } else {
                quotebuf.as_mut().unwrap().push(ch);
            }
        }
        flush_field(&mut fieldchar, &mut fieldnum, &mut tokens);
        flush_chars(&mut charbuf, &mut tokens);

        let mut out = Vec::with_capacity(tokens.len());
        for (is_field, text, fc, fn_) in tokens {
            if is_field {
                if let Some(ok) = allowed(fc) {
                    if !ok.contains(&fn_) {
                        return Err(Error::Fallback);
                    }
                }
                out.push(DtToken::Field(fc, fn_));
            } else {
                out.push(DtToken::Chars(text));
            }
        }
        Ok(DtPattern {
            raw: pattern.to_string(),
            tokens: out,
        })
    }
}

/// Decomposed date/time components (all values caller-validated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    /// (year, month, day, weekday_mon0) for date-bearing values.
    pub date: Option<(i32, u8, u8, u8)>,
    /// (hour, minute, second, microsecond) for time-bearing values.
    pub time: Option<(u8, u8, u8, u32)>,
}

/// Days since 1970-01-01 (Hinnant's `days_from_civil`).
fn days_from_civil(y: i32, m: u8, d: u8) -> i32 {
    let mut y = y;
    let m = m as i32;
    y -= (m <= 2) as i32;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let mp = (m + if m > 2 { -3 } else { 9 }) as u32;
    let doy = (153 * mp + 2) / 5 + d as u32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i32 - 719468
}

/// Monday-based weekday (0 = Monday), matching `date.weekday()`.
fn weekday_mon0(y: i32, m: u8, d: u8) -> u8 {
    (days_from_civil(y, m, d) + 3).rem_euclid(7) as u8
}

fn day_of_year(y: i32, m: u8, d: u8) -> i32 {
    days_from_civil(y, m, d) - days_from_civil(y, 1, 1) + 1
}

/// Port of `DateTimeFormat.get_week_number`.
fn week_number(day_of_period: i32, day_of_week: i32, first_day: i32, min_days: i32) -> i32 {
    let first = (day_of_week - first_day - day_of_period + 1).rem_euclid(7);
    let mut week = (day_of_period + first - 1) / 7;
    if 7 - first >= min_days {
        week += 1;
    }
    week
}

/// Port of `DateTimeFormat.get_week_of_year` for (y, m, d).
fn week_of_year(y: i32, m: u8, d: u8, first_day: i32, min_days: i32) -> i32 {
    let wday = weekday_mon0(y, m, d) as i32;
    let doy = day_of_year(y, m, d);
    let week = week_number(doy, wday, first_day, min_days);
    if week == 0 {
        let (py, pm, pd) = (y - 1, 12, 31);
        let pw = weekday_mon0(py, pm, pd) as i32;
        return week_number(day_of_year(py, pm, pd), pw, first_day, min_days);
    }
    if week > 52 {
        let nw = weekday_mon0(y + 1, 1, 1) as i32;
        if week_number(1, nw, first_day, min_days) == 1
            && 32 - (nw - first_day).rem_euclid(7) <= d as i32
        {
            return 1;
        }
    }
    week
}

fn pad0(value: i64, len: u8) -> String {
    let s = value.to_string();
    if s.len() < len as usize {
        format!("{}{s}", "0".repeat(len as usize - s.len()))
    } else {
        s
    }
}

/// Apply a parsed pattern to decomposed parts: port of
/// `DateTimeFormat.__getitem__` dispatched over the token stream.
pub fn apply(bundle: &Bundle, pat: &DtPattern, parts: Parts) -> Result<String, Error> {
    let mut out = String::new();
    for tok in &pat.tokens {
        match tok {
            DtToken::Chars(s) => out.push_str(s),
            DtToken::Field(c, n) => out.push_str(&apply_field(bundle, *c, *n, parts)?),
        }
    }
    Ok(out)
}

fn need_date(parts: Parts) -> Result<(i32, u8, u8, u8), Error> {
    parts.date.ok_or(Error::Fallback)
}

fn need_time(parts: Parts) -> Result<(u8, u8, u8, u32), Error> {
    parts.time.ok_or(Error::Fallback)
}

#[allow(clippy::too_many_arguments)]
fn apply_field(bundle: &Bundle, char: char, num: u8, parts: Parts) -> Result<String, Error> {
    let first_day = bundle.first_week_day as i32;
    let min_days = bundle.min_week_days as i32;
    match char {
        'G' => {
            let (y, _, _, _) = need_date(parts)?;
            let width = match num.max(3) {
                3 => "abbreviated",
                4 => "wide",
                _ => "narrow",
            };
            let names = bundle.eras.get(width).ok_or(Error::Fallback)?;
            names
                .get(usize::from(y >= 0))
                .cloned()
                .ok_or(Error::Fallback)
        }
        'y' | 'Y' | 'u' => {
            let (mut y, mo, d, _) = need_date(parts)?;
            if char.is_uppercase() {
                let woy = week_of_year(y, mo, d, first_day, min_days);
                if mo == 1 && d < 7 && woy >= 52 {
                    y -= 1;
                } else if mo == 12 && d > 25 && woy <= 2 {
                    y += 1;
                }
            }
            let s = pad0(y as i64, num);
            Ok(if num == 2 {
                s[s.len().saturating_sub(2)..].to_string()
            } else {
                s
            })
        }
        'Q' | 'q' => {
            let (_, mo, _, _) = need_date(parts)?;
            let q = ((mo - 1) / 3 + 1) as usize;
            if num <= 2 {
                return Ok(pad0(q as i64, num));
            }
            let width = match num {
                3 => "abbreviated",
                4 => "wide",
                _ => "narrow",
            };
            let ctx = if char == 'q' { "stand-alone" } else { "format" };
            bundle
                .quarters
                .get(ctx)
                .and_then(|m| m.get(width))
                .and_then(|v| v.get(q - 1))
                .cloned()
                .ok_or(Error::Fallback)
        }
        'M' | 'L' => {
            let (_, mo, _, _) = need_date(parts)?;
            if num <= 2 {
                return Ok(pad0(mo as i64, num));
            }
            let width = match num {
                3 => "abbreviated",
                4 => "wide",
                _ => "narrow",
            };
            let ctx = if char == 'L' { "stand-alone" } else { "format" };
            bundle
                .months
                .get(ctx)
                .and_then(|m| m.get(width))
                .and_then(|v| v.get(mo as usize - 1))
                .cloned()
                .ok_or(Error::Fallback)
        }
        'w' => {
            let (y, mo, d, _) = need_date(parts)?;
            Ok(pad0(
                week_of_year(y, mo, d, first_day, min_days) as i64,
                num,
            ))
        }
        'W' => {
            let (y, mo, d, _) = need_date(parts)?;
            let wd = weekday_mon0(y, mo, d) as i32;
            Ok(week_number(d as i32, wd, first_day, min_days).to_string())
        }
        'd' => {
            let (_, _, d, _) = need_date(parts)?;
            Ok(pad0(d as i64, num))
        }
        'D' => {
            let (y, mo, d, _) = need_date(parts)?;
            Ok(pad0(day_of_year(y, mo, d) as i64, num))
        }
        'F' => {
            let (_, _, d, _) = need_date(parts)?;
            Ok((((d - 1) / 7 + 1) as i64).to_string())
        }
        'E' | 'e' | 'c' => {
            let (y, mo, d, wday) = need_date(parts)?;
            let wd = weekday_mon0(y, mo, d);
            debug_assert_eq!(wd, wday);
            let mut n = num;
            if n < 3 {
                if char.is_lowercase() {
                    let v = (7 - first_day + wd as i32).rem_euclid(7) + 1;
                    return Ok(pad0(v as i64, n));
                }
                n = 3;
            }
            let width = match n {
                3 => "abbreviated",
                4 => "wide",
                5 => "narrow",
                _ => "short",
            };
            let ctx = if char == 'c' { "stand-alone" } else { "format" };
            bundle
                .days
                .get(ctx)
                .and_then(|m| m.get(width))
                .and_then(|v| v.get(wd as usize))
                .cloned()
                .ok_or(Error::Fallback)
        }
        'a' => {
            let (h, _, _, _) = need_time(parts)?;
            let period = if h >= 12 { "pm" } else { "am" };
            // Upstream tries [specific, wide, narrow, abbreviated].
            let first = match num.max(3) {
                3 => "abbreviated",
                4 => "wide",
                _ => "narrow",
            };
            for w in [first, "wide", "narrow", "abbreviated"] {
                if let Some(name) = bundle
                    .periods
                    .get("format")
                    .and_then(|m| m.get(w))
                    .and_then(|m| m.get(period))
                {
                    return Ok(name.clone());
                }
            }
            Err(Error::Fallback)
        }
        'b' | 'B' => Err(Error::Fallback),
        'h' => {
            let (h, _, _, _) = need_time(parts)?;
            Ok(pad0(if h % 12 == 0 { 12 } else { h % 12 } as i64, num))
        }
        'H' => {
            let (h, _, _, _) = need_time(parts)?;
            Ok(pad0(h as i64, num))
        }
        'K' => {
            let (h, _, _, _) = need_time(parts)?;
            Ok(pad0((h % 12) as i64, num))
        }
        'k' => {
            let (h, _, _, _) = need_time(parts)?;
            Ok(pad0(if h == 0 { 24 } else { h } as i64, num))
        }
        'm' => {
            let (_, mi, _, _) = need_time(parts)?;
            Ok(pad0(mi as i64, num))
        }
        's' => {
            let (_, _, s, _) = need_time(parts)?;
            Ok(pad0(s as i64, num))
        }
        'S' => {
            let (_, _, _, us) = need_time(parts)?;
            if us != 0 {
                // Float banker's rounding (`round(us/1e6, num) * 10**num`)
                // stays in Python.
                return Err(Error::Fallback);
            }
            Ok("0".repeat(num as usize))
        }
        'A' => {
            let (h, mi, s, us) = need_time(parts)?;
            let msecs = us as i64 / 1000 + s as i64 * 1000 + mi as i64 * 60000 + h as i64 * 3600000;
            Ok(pad0(msecs, num))
        }
        'z' | 'Z' | 'O' | 'v' | 'V' | 'x' | 'X' => Err(Error::Fallback),
        _ => Err(Error::Fallback),
    }
}
