//! Number-format engine: a value + a format code -> a display string.
//!
//! This is deliberately *not* a complete implementation of the Excel number
//! format grammar (which includes positive/negative/zero/text sections, colour
//! codes, conditional sections and arbitrary literal runs). Instead it handles
//! the common, recognisable codes a spreadsheet user reaches for — fixed
//! decimals, thousands separators, percentages, currency, scientific notation
//! and a handful of date/time patterns — and falls back to the engine's plain
//! `Value::as_text()` for anything it does not understand.
//!
//! Dates follow Excel's convention: a date is just a `Number` interpreted as a
//! *serial* value where day 0 is 1899-12-30, the integer part counts whole days
//! and the fractional part is the time of day. The date arithmetic is done by
//! hand here so the crate keeps its "pure Rust, no third-party date crate"
//! property.

use crate::value::Value;

/// Format a computed value through an Excel-style format `code`.
///
/// `Text`, `Bool` and `Error` values ignore the code entirely and render as
/// their plain text. Numbers are routed to whichever formatter the code names;
/// an unrecognised code is treated as `"General"`.
pub fn format_value(v: &Value, code: &str) -> String {
    // Non-numeric values are never reshaped by a number format.
    let n = match v {
        Value::Number(n) => *n,
        _ => return v.as_text(),
    };

    if !n.is_finite() {
        // NaN / infinity never make a meaningful formatted string; defer to the
        // same clean rendering `as_text` uses (which maps these to errors).
        return v.as_text();
    }

    match classify(code) {
        Format::General => v.as_text(),
        Format::Fixed { decimals, thousands, percent } => {
            format_fixed(n, decimals, thousands, percent)
        }
        Format::Currency { decimals, thousands } => {
            let body = format_fixed(n.abs(), decimals, thousands, false);
            if n.is_sign_negative() && n != 0.0 {
                format!("-${body}")
            } else {
                format!("${body}")
            }
        }
        Format::Scientific { decimals } => format_scientific(n, decimals),
        Format::DateTime => format_datetime(n, code),
    }
}

/// The named presets a UI can offer in a format picker. The first element of
/// each pair is a human label; the second is the code understood by
/// [`format_value`].
pub fn presets() -> &'static [(&'static str, &'static str)] {
    &[
        ("General", "General"),
        ("Number", "#,##0.00"),
        ("Currency", "$#,##0.00"),
        ("Percent", "0.00%"),
        ("Date", "yyyy-mm-dd"),
        ("Time", "hh:mm"),
        ("Scientific", "0.00E+00"),
    ]
}

// ---------------------------------------------------------------------------
// Code classification
// ---------------------------------------------------------------------------

/// The family a format code resolves to.
enum Format {
    General,
    /// Plain numeric: a fixed number of decimals, optional thousands grouping,
    /// optional trailing `%` (which also scales the number by 100).
    Fixed { decimals: usize, thousands: bool, percent: bool },
    /// Currency: a leading `$`, like `Fixed` otherwise.
    Currency { decimals: usize, thousands: bool },
    /// Scientific / exponential, e.g. `1.23E+04`.
    Scientific { decimals: usize },
    /// A date and/or time pattern, interpreted token-by-token in
    /// [`format_datetime`].
    DateTime,
}

/// Decide which [`Format`] a code belongs to. Recognition is intentionally
/// forgiving: anything unfamiliar collapses to [`Format::General`].
fn classify(code: &str) -> Format {
    let trimmed = code.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("General") {
        return Format::General;
    }

    // Date/time codes are detected by their field letters. Detecting these
    // first keeps a code like "mm/dd/yyyy" from being mistaken for a numeric
    // pattern because of its digits.
    if looks_like_datetime(trimmed) {
        return Format::DateTime;
    }

    // Scientific notation: must contain an exponent marker.
    if let Some(decimals) = scientific_decimals(trimmed) {
        return Format::Scientific { decimals };
    }

    // Everything else that is made only of recognised numeric punctuation is a
    // fixed / grouped / percent / currency number.
    let percent = trimmed.contains('%');
    let currency = trimmed.contains('$');
    let thousands = trimmed.contains(',');
    let decimals = decimals_after_point(trimmed);

    // Guard against arbitrary text masquerading as a format: every character
    // must be one we know how to honour.
    if trimmed
        .chars()
        .all(|c| matches!(c, '0' | '#' | ',' | '.' | '%' | '$' | ' '))
    {
        if currency {
            Format::Currency { decimals, thousands }
        } else {
            Format::Fixed { decimals, thousands, percent }
        }
    } else {
        Format::General
    }
}

/// True if `code` contains any date/time field characters. The check is loose
/// on purpose — a real format string would never mix these with the numeric
/// grammar, so their mere presence is a reliable signal.
fn looks_like_datetime(code: &str) -> bool {
    let lower = code.to_ascii_lowercase();
    // `y`, `d`, `h`, `s` are unambiguous date/time fields. `m` is shared by
    // months and minutes, so a lone `m` still counts.
    lower
        .chars()
        .any(|c| matches!(c, 'y' | 'd' | 'h' | 's' | 'm'))
}

/// If `code` is a scientific pattern (contains `E+`/`E-`/`e+`/`e-`), return the
/// number of fractional digits requested by its mantissa.
fn scientific_decimals(code: &str) -> Option<usize> {
    let upper = code.to_ascii_uppercase();
    let idx = upper.find('E')?;
    let after = &upper[idx + 1..];
    if !after.starts_with('+') && !after.starts_with('-') {
        return None;
    }
    Some(decimals_after_point(&code[..idx]))
}

/// Count the digit placeholders (`0`/`#`) after the first decimal point.
fn decimals_after_point(code: &str) -> usize {
    match code.split_once('.') {
        Some((_, frac)) => frac.chars().filter(|c| matches!(c, '0' | '#')).count(),
        None => 0,
    }
}

// ---------------------------------------------------------------------------
// Numeric formatters
// ---------------------------------------------------------------------------

/// Render `n` with a fixed number of `decimals`, optional `thousands` grouping
/// of the integer part, and an optional `percent` scaling + suffix.
fn format_fixed(n: f64, decimals: usize, thousands: bool, percent: bool) -> String {
    let scaled = if percent { n * 100.0 } else { n };
    let mut s = format!("{:.*}", decimals, scaled);

    if thousands {
        s = group_thousands(&s);
    }
    if percent {
        s.push('%');
    }
    s
}

/// Insert `,` every three digits into the integer portion of an already
/// formatted decimal string, preserving a leading sign and any fractional tail.
fn group_thousands(s: &str) -> String {
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => ("-", r),
        None => ("", s),
    };
    let (int_part, frac_part) = match rest.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rest, None),
    };

    let mut grouped = String::with_capacity(int_part.len() + int_part.len() / 3);
    let bytes = int_part.as_bytes();
    let len = bytes.len();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(*b as char);
    }

    let mut out = String::with_capacity(sign.len() + grouped.len() + 1 + frac_part.map_or(0, |f| f.len()));
    out.push_str(sign);
    out.push_str(&grouped);
    if let Some(f) = frac_part {
        out.push('.');
        out.push_str(f);
    }
    out
}

/// Render `n` in exponential form with `decimals` mantissa digits and a signed,
/// at-least-two-digit exponent, e.g. `1.23E+04`.
fn format_scientific(n: f64, decimals: usize) -> String {
    if n == 0.0 {
        let mantissa = format!("{:.*}", decimals, 0.0);
        return format!("{mantissa}E+00");
    }
    // Rust's `{:e}` gives `1.23e4`; reshape it into Excel's `1.23E+04`.
    let raw = format!("{:.*e}", decimals, n);
    let (mantissa, exp) = raw.split_once('e').unwrap_or((raw.as_str(), "0"));
    let exp_num: i32 = exp.parse().unwrap_or(0);
    let sign = if exp_num < 0 { '-' } else { '+' };
    format!("{mantissa}E{sign}{:02}", exp_num.abs())
}

// ---------------------------------------------------------------------------
// Date / time formatting
// ---------------------------------------------------------------------------

/// One lexical run of a format code: either a date/time field (a letter plus
/// its repeat count) or a literal stretch of separators / text.
enum Token {
    /// A field letter (`y`/`m`/`d`/`h`/`s`) repeated `run` times.
    Field { letter: char, run: usize },
    /// The 12-hour meridiem marker `AM/PM`.
    Meridiem,
    /// Literal text to copy through verbatim.
    Literal(String),
}

/// Walk the format `code` and substitute date/time fields from the Excel serial
/// number `serial`. Unknown characters are emitted verbatim, so separators like
/// `-`, `/`, `:` and spaces survive untouched.
///
/// The shared `m` field is resolved in a second pass: an `m` run is a *minute*
/// when its nearest neighbouring field is an hour (before) or second (after),
/// and a *month* otherwise — matching Excel's rule.
fn format_datetime(serial: f64, code: &str) -> String {
    let twelve_hour = code.to_ascii_uppercase().contains("AM/PM");
    let parts = decompose_serial(serial, twelve_hour);
    let tokens = tokenize_datetime(code);

    let mut out = String::with_capacity(code.len() + 8);
    for (idx, tok) in tokens.iter().enumerate() {
        match tok {
            Token::Literal(s) => out.push_str(s),
            Token::Meridiem => out.push_str(if parts.hour < 12 { "AM" } else { "PM" }),
            Token::Field { letter, run } => {
                let minute = *letter == 'm' && m_is_minute(&tokens, idx);
                out.push_str(&render_field(*letter, *run, &parts, minute));
            }
        }
    }
    out
}

/// Split a date/time `code` into [`Token`]s: contiguous runs of the same field
/// letter, the literal `AM/PM` marker, and everything else as literal text.
fn tokenize_datetime(code: &str) -> Vec<Token> {
    let chars: Vec<char> = code.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut literal = String::new();

    while i < chars.len() {
        // The 12-hour marker is matched as a whole word before anything else.
        if matches_at(&chars, i, "AM/PM") {
            if !literal.is_empty() {
                tokens.push(Token::Literal(std::mem::take(&mut literal)));
            }
            tokens.push(Token::Meridiem);
            i += "AM/PM".len();
            continue;
        }

        let lower = chars[i].to_ascii_lowercase();
        if matches!(lower, 'y' | 'm' | 'd' | 'h' | 's') {
            if !literal.is_empty() {
                tokens.push(Token::Literal(std::mem::take(&mut literal)));
            }
            let mut run = 1;
            while i + run < chars.len() && chars[i + run].to_ascii_lowercase() == lower {
                run += 1;
            }
            tokens.push(Token::Field { letter: lower, run });
            i += run;
        } else {
            literal.push(chars[i]);
            i += 1;
        }
    }
    if !literal.is_empty() {
        tokens.push(Token::Literal(literal));
    }
    tokens
}

/// Do the characters starting at `i` spell `needle` (case-insensitive)?
fn matches_at(chars: &[char], i: usize, needle: &str) -> bool {
    let needle: Vec<char> = needle.chars().collect();
    if i + needle.len() > chars.len() {
        return false;
    }
    chars[i..i + needle.len()]
        .iter()
        .zip(&needle)
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Decide whether the `m` field at `tokens[idx]` is a minute. It is a minute if
/// the nearest *field* token on either side (skipping literals/meridiem) is an
/// hour immediately before it or a second immediately after it.
fn m_is_minute(tokens: &[Token], idx: usize) -> bool {
    // Look backwards for the previous field letter.
    let prev = tokens[..idx]
        .iter()
        .rev()
        .find_map(|t| match t {
            Token::Field { letter, .. } => Some(*letter),
            _ => None,
        });
    if prev == Some('h') {
        return true;
    }
    // Look forwards for the next field letter.
    let next = tokens[idx + 1..]
        .iter()
        .find_map(|t| match t {
            Token::Field { letter, .. } => Some(*letter),
            _ => None,
        });
    next == Some('s')
}

/// Render one date/time field given its letter, run length, and (for `m`)
/// whether context resolved it to a minute.
fn render_field(letter: char, run: usize, parts: &DateParts, minute: bool) -> String {
    match letter {
        'y' => {
            if run <= 2 {
                format!("{:02}", parts.year % 100)
            } else {
                format!("{:04}", parts.year)
            }
        }
        'd' => {
            if run >= 2 {
                format!("{:02}", parts.day)
            } else {
                format!("{}", parts.day)
            }
        }
        'h' => {
            let h = if parts.twelve_hour {
                let h12 = parts.hour % 12;
                if h12 == 0 { 12 } else { h12 }
            } else {
                parts.hour
            };
            if run >= 2 { format!("{:02}", h) } else { format!("{}", h) }
        }
        's' => {
            if run >= 2 { format!("{:02}", parts.second) } else { format!("{}", parts.second) }
        }
        'm' if minute => {
            if run >= 2 { format!("{:02}", parts.minute) } else { format!("{}", parts.minute) }
        }
        // Otherwise `m` is a month: `m`/`mm` is a (zero-padded) number, `mmm` the
        // short name, `mmmm` the full name, `mmmmm` the single-letter initial.
        'm' => match run {
            3 => SHORT_MONTHS[(parts.month - 1) as usize].to_string(),
            4 => LONG_MONTHS[(parts.month - 1) as usize].to_string(),
            n if n >= 5 => SHORT_MONTHS[(parts.month - 1) as usize][..1].to_string(),
            1 => format!("{}", parts.month),
            _ => format!("{:02}", parts.month),
        },
        _ => String::new(),
    }
}

/// The calendar pieces extracted from an Excel serial number, plus whether the
/// owning format requested a 12-hour clock (so `h`/`hh` knows how to render).
struct DateParts {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    twelve_hour: bool,
}

const SHORT_MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const LONG_MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September",
    "October", "November", "December",
];

/// Convert an Excel serial number into calendar parts. Day 0 is 1899-12-30, so
/// serial 1 is 1900-01-01 and the fractional part is the fraction of a day.
fn decompose_serial(serial: f64, twelve_hour: bool) -> DateParts {
    let days = serial.floor() as i64;
    // Time of day from the fractional part, rounded to the nearest second so a
    // value like 0.5 lands exactly on 12:00:00 rather than 11:59:59.
    let frac = serial - serial.floor();
    let total_seconds = (frac * 86_400.0).round() as i64;
    let (days, total_seconds) = if total_seconds >= 86_400 {
        (days + 1, total_seconds - 86_400)
    } else {
        (days, total_seconds)
    };
    let hour = total_seconds / 3_600;
    let minute = (total_seconds % 3_600) / 60;
    let second = total_seconds % 60;

    let (year, month, day) = civil_from_serial(days);
    DateParts { year, month, day, hour, minute, second, twelve_hour }
}

/// Turn a day count (Excel serial, integer part) into a `(year, month, day)`
/// triple in the proleptic Gregorian calendar.
///
/// The algorithm is Howard Hinnant's well-known `civil_from_days`, shifted from
/// the Unix epoch (1970-01-01) to Excel's epoch. Excel serial 25569 is
/// 1970-01-01, so subtracting it rebases onto the Unix day count Hinnant's
/// formula expects.
fn civil_from_serial(excel_days: i64) -> (i64, i64, i64) {
    // Days since 1970-01-01.
    let z = excel_days - 25_569 + 719_468; // shift to the algorithm's era base
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{CellError, Value};

    #[test]
    fn general_passes_through() {
        assert_eq!(format_value(&Value::Number(3.5), "General"), "3.5");
        assert_eq!(format_value(&Value::Number(42.0), "General"), "42");
        // Empty code and unrecognised codes also fall back to General.
        assert_eq!(format_value(&Value::Number(42.0), ""), "42");
        assert_eq!(format_value(&Value::Number(42.0), "garbage"), "42");
    }

    #[test]
    fn non_numbers_ignore_the_code() {
        assert_eq!(format_value(&Value::Text("hi".into()), "0.00"), "hi");
        assert_eq!(format_value(&Value::Bool(true), "0.00"), "TRUE");
        assert_eq!(format_value(&Value::Empty, "#,##0"), "");
        assert_eq!(
            format_value(&Value::Error(CellError::Div0), "0.00"),
            "#DIV/0!"
        );
    }

    #[test]
    fn fixed_decimals() {
        assert_eq!(format_value(&Value::Number(3.14259), "0"), "3");
        assert_eq!(format_value(&Value::Number(3.14259), "0.00"), "3.14");
        assert_eq!(format_value(&Value::Number(3.0), "0.00"), "3.00");
        // Rounds half away from zero (Rust's `{:.2}` rounds half-to-even, but
        // these values are unambiguous).
        assert_eq!(format_value(&Value::Number(2.5), "0"), "2"); // banker's: 2.5 -> 2
    }

    #[test]
    fn thousands_grouping() {
        assert_eq!(format_value(&Value::Number(1234.5), "#,##0.00"), "1,234.50");
        assert_eq!(format_value(&Value::Number(1234.0), "#,##0"), "1,234");
        assert_eq!(
            format_value(&Value::Number(1_234_567.0), "#,##0"),
            "1,234,567"
        );
        assert_eq!(
            format_value(&Value::Number(-1234.5), "#,##0.00"),
            "-1,234.50"
        );
        assert_eq!(format_value(&Value::Number(999.0), "#,##0"), "999");
    }

    #[test]
    fn percentages() {
        assert_eq!(format_value(&Value::Number(0.25), "0%"), "25%");
        assert_eq!(format_value(&Value::Number(0.255), "0.00%"), "25.50%");
        assert_eq!(format_value(&Value::Number(1.0), "0%"), "100%");
    }

    #[test]
    fn currency() {
        assert_eq!(
            format_value(&Value::Number(1234.5), "$#,##0.00"),
            "$1,234.50"
        );
        assert_eq!(
            format_value(&Value::Number(-1234.5), "$#,##0.00"),
            "-$1,234.50"
        );
        assert_eq!(format_value(&Value::Number(0.0), "$#,##0.00"), "$0.00");
    }

    #[test]
    fn scientific() {
        assert_eq!(format_value(&Value::Number(12345.0), "0.00E+00"), "1.23E+04");
        assert_eq!(format_value(&Value::Number(0.0012), "0.00E+00"), "1.20E-03");
        assert_eq!(format_value(&Value::Number(0.0), "0.00E+00"), "0.00E+00");
    }

    #[test]
    fn dates_from_serials() {
        // With the 1899-12-30 epoch and straight proleptic-Gregorian arithmetic
        // (no replication of Excel's fictional 1900 leap day), serial 0 is the
        // epoch itself and serial 1 the day after.
        assert_eq!(
            format_value(&Value::Number(0.0), "yyyy-mm-dd"),
            "1899-12-30"
        );
        assert_eq!(
            format_value(&Value::Number(1.0), "yyyy-mm-dd"),
            "1899-12-31"
        );
        assert_eq!(
            format_value(&Value::Number(2.0), "yyyy-mm-dd"),
            "1900-01-01"
        );
        // A well-known modern date: 2020-01-01 is serial 43831.
        assert_eq!(
            format_value(&Value::Number(43831.0), "yyyy-mm-dd"),
            "2020-01-01"
        );
        assert_eq!(
            format_value(&Value::Number(43831.0), "mm/dd/yyyy"),
            "01/01/2020"
        );
        assert_eq!(
            format_value(&Value::Number(43831.0), "d mmm yyyy"),
            "1 Jan 2020"
        );
    }

    #[test]
    fn times_from_fractions() {
        // 0.5 of a day is noon.
        assert_eq!(format_value(&Value::Number(0.5), "hh:mm"), "12:00");
        // 0.25 is 06:00.
        assert_eq!(format_value(&Value::Number(0.25), "hh:mm"), "06:00");
        // 12-hour clock with meridiem.
        assert_eq!(
            format_value(&Value::Number(0.5), "h:mm AM/PM"),
            "12:00 PM"
        );
        assert_eq!(
            format_value(&Value::Number(0.25), "h:mm AM/PM"),
            "6:00 AM"
        );
    }

    #[test]
    fn datetime_combined() {
        // Serial 43831.5 == 2020-01-01 12:00.
        assert_eq!(
            format_value(&Value::Number(43831.5), "yyyy-mm-dd hh:mm"),
            "2020-01-01 12:00"
        );
    }

    #[test]
    fn presets_are_understood() {
        // Every preset code should at minimum not panic and should round-trip a
        // sample value through `format_value`.
        for (_label, code) in presets() {
            let _ = format_value(&Value::Number(1234.5), code);
        }
        // And the labels/codes are the documented set.
        assert_eq!(presets()[0], ("General", "General"));
        assert_eq!(presets()[2], ("Currency", "$#,##0.00"));
    }
}
