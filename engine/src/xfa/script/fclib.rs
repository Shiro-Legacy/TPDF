// SPDX-License-Identifier: MIT
//! FormCalc's built-in functions that need no form: the date and time
//! numbers, English words for amounts, measurements, the financial
//! functions and the URL, HTML and XML encodings. `formcalc` calls them;
//! the picture-clause work (formatting and reading text) is `picture`'s.
//!
//! Rules:
//! - A date number counts days from FormCalc's epoch: 1 is 1900-01-01,
//!   in the proleptic Gregorian calendar (1900 is not a leap year, 2000
//!   is). Dates before 1900-01-01 or after 9999-12-31 have no number.
//! - A time number counts milliseconds from FormCalc's time epoch:
//!   1 is midnight GMT (so 0 can mean "no time"). Times carry no date:
//!   a time east of GMT wraps around midnight. There is no time-zone
//!   data: local time is GMT.
//! - The financial functions follow the usual annuity formulas over
//!   `f64`; any input outside a function's domain (a non-positive
//!   amount, rate or count where one is needed) is `None`.

/// Milliseconds in a day.
pub const DAY_MS: i64 = 86_400_000;

/// Months iterated by `IPmt`/`PPmt`, at most (a longer schedule is `None`).
pub const MAX_PERIODS: f64 = 1_000_000.0;

// ------------------------------------------------------------------ dates

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days in the years before `y` (from year 1).
fn days_before_year(y: i64) -> i64 {
    let p = y - 1;
    p * 365 + p / 4 - p / 100 + p / 400
}

/// Day 1 is 1 January of year 1.
fn ordinal(y: i64, m: u32, d: u32) -> i64 {
    let before_month: i64 = (1..m).map(|k| i64::from(days_in_month(y, k))).sum();
    days_before_year(y) + before_month + i64::from(d)
}

/// The FormCalc date number of `y-m-d`; `None` when the date does not
/// exist or lies outside 1900-01-01 ..= 9999-12-31.
pub fn date_number(y: i64, m: u32, d: u32) -> Option<i64> {
    if !(1900..=9999).contains(&y) || !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some(ordinal(y, m, d) - ordinal(1900, 1, 1) + 1)
}

/// The date of FormCalc date number `n` as `(year, month, day)`.
pub fn number_date(n: i64) -> Option<(i64, u32, u32)> {
    let last = date_number(9999, 12, 31)?;
    if !(1..=last).contains(&n) {
        return None;
    }
    let day = n + ordinal(1900, 1, 1) - 1;
    // A year at or below the answer, then forward.
    let mut y = (day * 400 / 146_097).max(1);
    while days_before_year(y + 1) < day {
        y += 1;
    }
    while days_before_year(y) >= day {
        y -= 1;
    }
    let mut left = day - days_before_year(y);
    for m in 1..=12 {
        let len = i64::from(days_in_month(y, m));
        if left <= len {
            return Some((y, m, left as u32));
        }
        left -= len;
    }
    None
}

/// `YYYY-MM-DD` (or `YYYYMMDD`, `YYYY-MM`, `YYYY`, a time after a `T`
/// ignored) as a date number.
pub fn iso_date_number(text: &str) -> Option<i64> {
    let text = text.trim();
    let date = text.split(['T', 't']).next().unwrap_or("");
    let digits = |s: &str| -> Option<i64> {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse().ok())
            .flatten()
    };
    let (y, m, d) = if date.contains('-') {
        let mut parts = date.split('-');
        let y = digits(parts.next()?)?;
        let m = parts.next().map_or(Some(1), digits)?;
        let d = parts.next().map_or(Some(1), digits)?;
        if parts.next().is_some() {
            return None;
        }
        (y, m, d)
    } else {
        match date.len() {
            8 => (
                digits(&date[..4])?,
                digits(&date[4..6])?,
                digits(&date[6..])?,
            ),
            6 => (digits(&date[..4])?, digits(&date[4..])?, 1),
            4 => (digits(date)?, 1, 1),
            _ => return None,
        }
    };
    date_number(y, u32::try_from(m).ok()?, u32::try_from(d).ok()?)
}

/// A date number as `YYYY-MM-DD`.
pub fn date_text(n: i64) -> Option<String> {
    let (y, m, d) = number_date(n)?;
    Some(format!("{y:04}-{m:02}-{d:02}"))
}

/// Today's date number from the system clock (GMT).
pub fn today() -> i64 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    // 1970-01-01 is date number 25,568.
    let epoch = date_number(1970, 1, 1).unwrap_or(25_568);
    epoch + secs.div_euclid(86_400)
}

/// The time number of the moment now (GMT).
pub fn now_time() -> i64 {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    ms.rem_euclid(DAY_MS) + 1
}

// ------------------------------------------------------------------ times

/// A canonical time (`HH[:MM[:SS[.FFF]]]` or `HHMM[SS]`, then `Z` or
/// `±HH[:MM]`; a date before a `T` ignored) as a time number, moved to
/// GMT by its zone.
pub fn iso_time_number(text: &str) -> Option<i64> {
    let text = text.trim();
    let text = match text.find(['T', 't']) {
        Some(at) => &text[at + 1..],
        None => text,
    };
    let (clock, zone) = match text.find(['Z', 'z', '+', '-']) {
        Some(at) => (&text[..at], zone_minutes(&text[at..])?),
        None => (text, 0),
    };
    let (clock, ms) = match clock.split_once(['.', ',']) {
        Some((c, frac)) => {
            let frac = &frac[..frac.len().min(3)];
            if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let scale = 10i64.pow(3 - frac.len() as u32);
            (c, frac.parse::<i64>().ok()? * scale)
        }
        None => (clock, 0),
    };
    let parts: Vec<&str> = if clock.contains(':') {
        clock.split(':').collect()
    } else if clock.len() % 2 == 0 && clock.is_ascii() {
        (0..clock.len() / 2)
            .map(|i| &clock[i * 2..i * 2 + 2])
            .collect()
    } else {
        return None;
    };
    if parts.is_empty()
        || parts.len() > 3
        || parts
            .iter()
            .any(|p| p.len() != 2 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let at = |i: usize| parts.get(i).map_or(0, |p| p.parse::<i64>().unwrap_or(99));
    let (h, m, s) = (at(0), at(1), at(2));
    if h > 24 || m > 59 || s > 59 || (h == 24 && (m, s, ms) != (0, 0, 0)) {
        return None;
    }
    let local = ((h * 60 + m) * 60 + s) * 1000 + ms;
    Some((local - i64::from(zone) * 60_000).rem_euclid(DAY_MS) + 1)
}

/// `Z`, `±HH`, `±HHMM` or `±HH:MM` as minutes east of GMT.
fn zone_minutes(text: &str) -> Option<i32> {
    if text.eq_ignore_ascii_case("z") {
        return Some(0);
    }
    let sign = match text.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let rest: String = text[1..].chars().filter(|&c| c != ':').collect();
    if !(rest.len() == 2 || rest.len() == 4) || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i32 = rest[..2].parse().ok()?;
    let m: i32 = match rest.get(2..) {
        Some(m) if !m.is_empty() => m.parse().ok()?,
        _ => 0,
    };
    (h <= 14 && m < 60).then_some(sign * (h * 60 + m))
}

/// A time number as canonical `HH:MM:SS.FFF`; `None` below 1.
pub fn time_text(n: i64) -> Option<String> {
    if n < 1 {
        return None;
    }
    let ms = (n - 1).rem_euclid(DAY_MS);
    Some(format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    ))
}

// ------------------------------------------------------------------ words

const ONES: [&str; 20] = [
    "Zero",
    "One",
    "Two",
    "Three",
    "Four",
    "Five",
    "Six",
    "Seven",
    "Eight",
    "Nine",
    "Ten",
    "Eleven",
    "Twelve",
    "Thirteen",
    "Fourteen",
    "Fifteen",
    "Sixteen",
    "Seventeen",
    "Eighteen",
    "Nineteen",
];
const TENS: [&str; 10] = [
    "", "", "Twenty", "Thirty", "Forty", "Fifty", "Sixty", "Seventy", "Eighty", "Ninety",
];
const GROUPS: [&str; 5] = ["", "Thousand", "Million", "Billion", "Trillion"];

/// 1 ..= 99: `Forty-two`.
fn tens_words(n: u64) -> String {
    if n < 20 {
        return ONES[n as usize].to_string();
    }
    let tens = TENS[(n / 10) as usize];
    match n % 10 {
        0 => tens.to_string(),
        unit => format!("{tens}-{}", ONES[unit as usize].to_lowercase()),
    }
}

/// 1 ..= 999: `One Hundred and Forty-two`.
fn hundreds_words(n: u64) -> String {
    let (hundreds, rest) = (n / 100, n % 100);
    match (hundreds, rest) {
        (0, r) => tens_words(r),
        (h, 0) => format!("{} Hundred", ONES[h as usize]),
        (h, r) => format!("{} Hundred and {}", ONES[h as usize], tens_words(r)),
    }
}

/// A whole number in English words (`Zero` for 0).
fn number_words(n: u64) -> String {
    if n == 0 {
        return ONES[0].to_string();
    }
    let mut parts = Vec::new();
    let mut rest = n;
    let mut group = 0;
    while rest > 0 && group < GROUPS.len() {
        let chunk = rest % 1000;
        if chunk > 0 {
            let words = hundreds_words(chunk);
            parts.push(if GROUPS[group].is_empty() {
                words
            } else {
                format!("{words} {}", GROUPS[group])
            });
        }
        rest /= 1000;
        group += 1;
    }
    parts.reverse();
    parts.join(" ")
}

/// `WordNum`: `n` in English words. `identifier` 0 gives the whole
/// number (`One Hundred and Twenty-three`), 1 adds `Dollars`, 2 adds
/// `Dollars And ... Cents` with the cents rounded. A negative number, or
/// one of a thousand trillion or more, is `*`.
pub fn word_num(n: f64, identifier: i64) -> String {
    if !(0.0..1e15).contains(&n) {
        return "*".into();
    }
    let (whole, cents) = if identifier == 2 {
        let total = (n * 100.0).round() as u64;
        (total / 100, total % 100)
    } else {
        (n.trunc() as u64, 0)
    };
    let words = number_words(whole);
    match identifier {
        1 => format!("{words} Dollars"),
        2 => format!("{words} Dollars And {} Cents", number_words(cents)),
        _ => words,
    }
}

// ------------------------------------------------------------ measurements

/// A unit name as FormCalc writes it, and its size in points.
fn unit(name: &str) -> Option<(&'static str, f64)> {
    Some(match name.to_ascii_lowercase().as_str() {
        "in" | "inch" | "inches" => ("in", 72.0),
        "cm" | "centimeter" | "centimeters" | "centimetre" | "centimetres" => ("cm", 72.0 / 2.54),
        "mm" | "millimeter" | "millimeters" | "millimetre" | "millimetres" => ("mm", 72.0 / 25.4),
        "pt" | "point" | "points" => ("pt", 1.0),
        "mp" | "millipoint" | "millipoints" => ("mp", 0.001),
        _ => return None,
    })
}

/// A measurement (`"2.5cm"`, `" 3 in"`, `"72"`) as its number and unit
/// (inches when none is written). `None` when it does not read.
fn measurement(text: &str) -> Option<(f64, &'static str, f64)> {
    let t = text.trim();
    let end = t
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || ((c == '-' || c == '+') && i == 0)))
        .map_or(t.len(), |(i, _)| i);
    let value: f64 = t[..end].parse().ok()?;
    let name = t[end..].trim();
    let (name, size) = if name.is_empty() {
        ("in", 72.0)
    } else {
        unit(name)?
    };
    Some((value, name, size))
}

/// `UnitType`: the unit a measurement is written in.
pub fn unit_type(text: &str) -> Option<&'static str> {
    measurement(text).map(|(_, name, _)| name)
}

/// `UnitValue`: a measurement in `to` (inches when `None`).
pub fn unit_value(text: &str, to: Option<&str>) -> Option<f64> {
    let (value, _, size) = measurement(text)?;
    let (_, to_size) = match to {
        Some(name) => unit(name.trim())?,
        None => ("in", 72.0),
    };
    Some(value * size / to_size)
}

// ---------------------------------------------------------------- finance

fn finite(x: f64) -> Option<f64> {
    x.is_finite().then_some(x)
}

/// `Pmt`: the payment per period that repays `principal` at `rate` per
/// period over `periods`.
pub fn pmt(principal: f64, rate: f64, periods: f64) -> Option<f64> {
    if principal <= 0.0 || rate <= 0.0 || periods <= 0.0 {
        return None;
    }
    finite(principal * rate / (1.0 - (1.0 + rate).powf(-periods)))
}

/// `PV`: what `periods` payments of `payment` at `rate` are worth now.
pub fn pv(payment: f64, rate: f64, periods: f64) -> Option<f64> {
    if payment <= 0.0 || rate <= 0.0 || periods <= 0.0 {
        return None;
    }
    finite(payment * (1.0 - (1.0 + rate).powf(-periods)) / rate)
}

/// `FV`: what `periods` payments of `payment` at `rate` grow to.
pub fn fv(payment: f64, rate: f64, periods: f64) -> Option<f64> {
    if payment <= 0.0 || rate < 0.0 || periods <= 0.0 {
        return None;
    }
    if rate == 0.0 {
        return finite(payment * periods);
    }
    finite(payment * ((1.0 + rate).powf(periods) - 1.0) / rate)
}

/// `Rate`: the rate per period that grows `present` to `future` over
/// `periods`.
pub fn rate(future: f64, present: f64, periods: f64) -> Option<f64> {
    if future <= 0.0 || present <= 0.0 || periods <= 0.0 {
        return None;
    }
    finite((future / present).powf(1.0 / periods) - 1.0)
}

/// `CTerm`: the periods `present` needs at `rate` to grow to `future`.
pub fn cterm(rate: f64, future: f64, present: f64) -> Option<f64> {
    if rate <= 0.0 || future <= 0.0 || present <= 0.0 {
        return None;
    }
    finite((future / present).ln() / (1.0 + rate).ln())
}

/// `Term`: the periods payments of `payment` at `rate` need to reach
/// `future`.
pub fn term(payment: f64, rate: f64, future: f64) -> Option<f64> {
    if payment <= 0.0 || rate <= 0.0 || future <= 0.0 {
        return None;
    }
    finite((1.0 + future * rate / payment).ln() / (1.0 + rate).ln())
}

/// `NPV`: the present value of `values`, one a period from the next
/// period on, at `rate`.
pub fn npv(rate: f64, values: &[f64]) -> Option<f64> {
    if rate <= -1.0 || values.is_empty() {
        return None;
    }
    let mut factor = 1.0;
    let mut total = 0.0;
    for v in values {
        factor *= 1.0 + rate;
        total += v / factor;
    }
    finite(total)
}

/// `Apr`: the annual rate (twelve monthly periods) at which `periods`
/// monthly payments of `payment` repay `principal`.
pub fn apr(principal: f64, payment: f64, periods: f64) -> Option<f64> {
    if principal <= 0.0 || payment <= 0.0 || periods <= 0.0 || payment * periods <= principal {
        return None;
    }
    // The payment grows with the rate: bisect for it.
    let payment_at = |r: f64| principal * r / (1.0 - (1.0 + r).powf(-periods));
    let (mut lo, mut hi) = (1e-12, 1.0);
    if payment_at(hi) < payment {
        return None;
    }
    for _ in 0..200 {
        let mid = (lo + hi) / 2.0;
        if payment_at(mid) < payment {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    finite((lo + hi) / 2.0 * 12.0)
}

/// The interest and principal parts of `count` monthly payments from
/// payment `first` (1-based) of a loan of `principal` at the annual
/// `rate`, paid off with `payment` a month. `None` for a schedule past
/// `MAX_PERIODS`; zeros when the payment does not cover a month's
/// interest.
fn amortized(
    principal: f64,
    rate: f64,
    payment: f64,
    first: f64,
    count: f64,
) -> Option<(f64, f64)> {
    if principal <= 0.0 || rate <= 0.0 || payment <= 0.0 || first < 1.0 || count < 1.0 {
        return None;
    }
    let (first, count) = (first.trunc(), count.trunc());
    if first + count > MAX_PERIODS {
        return None;
    }
    let monthly = rate / 12.0;
    if payment <= principal * monthly {
        return Some((0.0, 0.0));
    }
    let (mut balance, mut interest, mut repaid) = (principal, 0.0, 0.0);
    let mut month = 1.0;
    while month < first + count && balance > 0.0 {
        let i = balance * monthly;
        let p = (payment - i).min(balance);
        if month >= first {
            interest += i;
            repaid += p;
        }
        balance -= p;
        month += 1.0;
    }
    Some((interest, repaid))
}

/// `IPmt`: the interest in `count` payments from payment `first`.
pub fn ipmt(principal: f64, rate: f64, payment: f64, first: f64, count: f64) -> Option<f64> {
    amortized(principal, rate, payment, first, count).map(|(i, _)| i)
}

/// `PPmt`: the principal repaid by `count` payments from payment `first`.
pub fn ppmt(principal: f64, rate: f64, payment: f64, first: f64, count: f64) -> Option<f64> {
    amortized(principal, rate, payment, first, count).map(|(_, p)| p)
}

// ---------------------------------------------------------------- encoding

/// `Encode`: `text` for a URL (`%XX` for every byte but letters, digits
/// and `-_.~`), HTML or XML (the markup characters as entities; HTML also
/// every character past ASCII as `&#xH;`). `None` for another kind.
pub fn encode(text: &str, kind: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    match kind.to_ascii_lowercase().as_str() {
        "url" => {
            for b in text.bytes() {
                if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                    out.push(char::from(b));
                } else {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
        kind @ ("html" | "xml") => {
            for c in text.chars() {
                match c {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    '"' => out.push_str("&quot;"),
                    '\'' => out.push_str("&apos;"),
                    c if kind == "html" && !c.is_ascii() => {
                        out.push_str(&format!("&#x{:X};", u32::from(c)));
                    }
                    c => out.push(c),
                }
            }
        }
        _ => return None,
    }
    Some(out)
}

/// `Decode`: undoes `encode` (named entities `amp`, `lt`, `gt`, `quot`,
/// `apos`, `nbsp` and numeric ones; `%XX` and `+` for URLs). Anything
/// malformed is kept as written.
pub fn decode(text: &str, kind: &str) -> Option<String> {
    match kind.to_ascii_lowercase().as_str() {
        "url" => {
            let bytes = text.as_bytes();
            let mut out = Vec::with_capacity(bytes.len());
            let mut i = 0;
            while i < bytes.len() {
                let hex = (bytes[i] == b'%')
                    .then(|| text.get(i + 1..i + 3))
                    .flatten()
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match (bytes[i], hex) {
                    (_, Some(b)) => {
                        out.push(b);
                        i += 3;
                    }
                    (b'+', None) => {
                        out.push(b' ');
                        i += 1;
                    }
                    (b, None) => {
                        out.push(b);
                        i += 1;
                    }
                }
            }
            Some(String::from_utf8_lossy(&out).into_owned())
        }
        "html" | "xml" => {
            let mut out = String::with_capacity(text.len());
            let mut rest = text;
            while let Some(at) = rest.find('&') {
                out.push_str(&rest[..at]);
                rest = &rest[at..];
                let entity = rest
                    .find(';')
                    .filter(|&end| end <= 12)
                    .and_then(|end| Some((entity_char(&rest[1..end])?, end)));
                match entity {
                    Some((c, end)) => {
                        out.push(c);
                        rest = &rest[end + 1..];
                    }
                    None => {
                        out.push('&');
                        rest = &rest[1..];
                    }
                }
            }
            out.push_str(rest);
            Some(out)
        }
        _ => None,
    }
}

fn entity_char(name: &str) -> Option<char> {
    if let Some(num) = name.strip_prefix('#') {
        let code = match num.strip_prefix(['x', 'X']) {
            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
            None => num.parse().ok()?,
        };
        return char::from_u32(code);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => '\u{a0}',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_numbers_follow_the_gregorian_calendar() {
        assert_eq!(date_number(1900, 1, 1), Some(1));
        assert_eq!(date_number(1900, 3, 1), Some(60), "1900 has no 29 February");
        assert_eq!(date_number(1900, 2, 29), None);
        assert_eq!(date_number(2000, 2, 29), Some(36_584));
        assert_eq!(date_number(2100, 2, 29), None);
        assert_eq!(date_number(1970, 1, 1), Some(25_568));
        assert_eq!(date_number(2026, 11, 5), Some(46_330));
        assert_eq!(date_number(1899, 12, 31), None);
        for n in [1, 59, 60, 36_584, 36_585, 46_330, 2_958_464] {
            let (y, m, d) = number_date(n).unwrap();
            assert_eq!(date_number(y, m, d), Some(n), "{n}");
        }
        assert_eq!(number_date(0), None);
        assert_eq!(number_date(2_958_465), None, "past 9999-12-31");
        assert_eq!(date_text(36_584).as_deref(), Some("2000-02-29"));
        assert_eq!(iso_date_number("2026-11-05T10:00:00Z"), Some(46_330));
        assert_eq!(iso_date_number("20261105"), Some(46_330));
        assert_eq!(iso_date_number("2026-13-01"), None);
        assert!(today() >= 46_000);
    }

    #[test]
    fn time_numbers_count_from_midnight_gmt() {
        assert_eq!(iso_time_number("00:00:00"), Some(1));
        assert_eq!(iso_time_number("13:13:13"), Some(47_593_001));
        assert_eq!(iso_time_number("13:13:13.5"), Some(47_593_501));
        assert_eq!(iso_time_number("T131313"), Some(47_593_001));
        assert_eq!(iso_time_number("15:13:13+02:00"), Some(47_593_001));
        assert_eq!(iso_time_number("01:00-02"), Some(3 * 3_600_000 + 1));
        assert_eq!(iso_time_number("00:30+01:00"), Some(DAY_MS - 1_800_000 + 1));
        assert_eq!(iso_time_number("25:00"), None);
        assert_eq!(time_text(47_593_001).as_deref(), Some("13:13:13.000"));
        assert_eq!(time_text(0), None);
        assert!((1..=DAY_MS).contains(&now_time()));
    }

    #[test]
    fn words_and_measurements() {
        assert_eq!(
            word_num(1234.0, 0),
            "One Thousand Two Hundred and Thirty-four"
        );
        assert_eq!(word_num(123.45, 0), "One Hundred and Twenty-three");
        assert_eq!(word_num(123.45, 1), "One Hundred and Twenty-three Dollars");
        assert_eq!(
            word_num(1154.67, 2),
            "One Thousand One Hundred and Fifty-four Dollars And Sixty-seven Cents"
        );
        assert_eq!(word_num(0.0, 0), "Zero");
        assert_eq!(word_num(2_000_015.0, 0), "Two Million Fifteen");
        assert_eq!(word_num(-1.0, 0), "*");
        assert_eq!(unit_type("2.54cm"), Some("cm"));
        assert_eq!(unit_type(" 36 inches"), Some("in"));
        assert_eq!(unit_type("72"), Some("in"));
        assert_eq!(unit_type("3furlongs"), None);
        let close = |a: Option<f64>, b: f64| (a.unwrap() - b).abs() < 1e-9;
        assert!(close(unit_value("2in", Some("cm")), 5.08));
        assert!(close(unit_value("72pt", None), 1.0));
        assert!(close(unit_value("25.4mm", Some("pt")), 72.0));
    }

    #[test]
    fn financial_functions() {
        let close = |a: Option<f64>, b: f64| (a.unwrap() - b).abs() < 0.005;
        // Hand-computed from the annuity formulas.
        assert!(close(pmt(150_000.0, 0.0475 / 12.0, 300.0), 855.18));
        assert!(close(pv(400.0, 0.10 / 12.0, 360.0), 45_580.33));
        assert!(close(fv(400.0, 0.10 / 12.0, 360.0), 904_195.17));
        assert!(close(fv(100.0, 0.0, 12.0), 1200.0));
        assert!(close(rate(110.0, 100.0, 1.0), 0.1));
        assert!(close(cterm(0.02, 1000.0, 100.0), 116.28));
        assert!(close(term(475.0, 0.05, 1500.0), 3.0048));
        assert!(close(npv(0.065, &[5000.0]), 4694.84));
        assert!((apr(35_000.0, 269.50, 360.0).unwrap() - 0.0852).abs() < 0.0001);
        assert!(close(ipmt(30_000.0, 0.085, 295.50, 7.0, 3.0), 624.88));
        assert!(close(ppmt(30_000.0, 0.085, 295.50, 7.0, 3.0), 261.62));
        assert_eq!(pmt(0.0, 0.1, 3.0), None);
        assert_eq!(apr(1000.0, 1.0, 10.0), None);
        assert_eq!(ipmt(1.0, 0.1, 1.0, 1.0, 1e12), None);
        assert_eq!(ipmt(30_000.0, 0.085, 100.0, 1.0, 3.0), Some(0.0));
    }

    #[test]
    fn encodings_round_trip() {
        assert_eq!(encode("a b&c/é", "url").unwrap(), "a%20b%26c%2F%C3%A9");
        assert_eq!(decode("a%20b%26c%2F%C3%A9+x", "url").unwrap(), "a b&c/é x");
        assert_eq!(
            encode("<a href=\"x\">é</a>", "html").unwrap(),
            "&lt;a href=&quot;x&quot;&gt;&#xE9;&lt;/a&gt;"
        );
        assert_eq!(encode("é&", "xml").unwrap(), "é&amp;");
        assert_eq!(
            decode("&lt;b&gt; &#233;&#x41; &bogus; & x", "html").unwrap(),
            "<b> éA &bogus; & x"
        );
        assert_eq!(encode("x", "base64"), None);
    }
}
