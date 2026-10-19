// SPDX-License-Identifier: MIT
//! Calendar arithmetic in UTC milliseconds since 1970, the unit the host
//! passes (JavaScript `Date.now()`), and the ASN.1 and PDF date forms.

/// Days since 1970-01-01 of a proleptic Gregorian date (month 1..=12).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of a day count since 1970.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Like JavaScript's `Date.UTC`: out-of-range months, days and times roll
/// over into the next unit instead of failing.
pub fn utc_ms(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
    let months = year * 12 + (month - 1);
    let (y, m) = (months.div_euclid(12), months.rem_euclid(12) + 1);
    let days = days_from_civil(y, m, 1) + (day - 1);
    ((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ`, as JavaScript's `toISOString`.
pub fn iso(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rest = ms.rem_euclid(86_400_000);
    let (y, mo, d) = civil_from_days(days);
    format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rest / 3_600_000,
        rest / 60_000 % 60,
        rest / 1000 % 60,
        rest % 1000
    )
}

/// `YYYY-MM-DD`.
pub fn ymd(ms: i64) -> String {
    let (y, m, d) = civil_from_days(ms.div_euclid(86_400_000));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `18 Oct 2026`, the date form of the plain-language report (UTC).
pub fn day_month_year(ms: i64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let (y, m, d) = civil_from_days(ms.div_euclid(86_400_000));
    format!("{d} {} {y}", MONTHS[(m - 1) as usize])
}

fn digits(text: &[u8]) -> Option<Vec<i64>> {
    text.iter()
        .map(|&b| b.is_ascii_digit().then(|| i64::from(b - b'0')))
        .collect()
}

fn number(d: &[i64]) -> i64 {
    d.iter().fold(0, |acc, &x| acc * 10 + x)
}

/// UTCTime `YYMMDDHHMMSSZ` (years 50-99 are 19xx).
pub fn parse_utc_time(text: &[u8]) -> Option<i64> {
    if text.len() != 13 || text[12] != b'Z' {
        return None;
    }
    let d = digits(&text[..12])?;
    let yy = number(&d[0..2]);
    let year = if yy >= 50 { 1900 + yy } else { 2000 + yy };
    Some(utc_ms(
        year,
        number(&d[2..4]),
        number(&d[4..6]),
        number(&d[6..8]),
        number(&d[8..10]),
        number(&d[10..12]),
    ))
}

/// GeneralizedTime `YYYYMMDDHHMMSSZ`.
pub fn parse_generalized_time(text: &[u8]) -> Option<i64> {
    if text.len() != 15 || text[14] != b'Z' {
        return None;
    }
    let d = digits(&text[..14])?;
    Some(utc_ms(
        number(&d[0..4]),
        number(&d[4..6]),
        number(&d[6..8]),
        number(&d[8..10]),
        number(&d[10..12]),
        number(&d[12..14]),
    ))
}

/// A PDF date `D:YYYYMMDDHHmmSS` with an optional `Z` or `+HH'mm'` offset;
/// anything after the recognised prefix is ignored.
pub fn parse_pdf_date(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() < 16 || &b[..2] != b"D:" {
        return None;
    }
    let d = digits(&b[2..16])?;
    let utc = utc_ms(
        number(&d[0..4]),
        number(&d[4..6]),
        number(&d[6..8]),
        number(&d[8..10]),
        number(&d[10..12]),
        number(&d[12..14]),
    );
    let rest = &b[16..];
    if rest.len() >= 7
        && (rest[0] == b'+' || rest[0] == b'-')
        && rest[3] == b'\''
        && rest[6] == b'\''
    {
        if let (Some(h), Some(m)) = (digits(&rest[1..3]), digits(&rest[4..6])) {
            let offset = number(&h) * 60 + number(&m);
            let sign = if rest[0] == b'-' { -1 } else { 1 };
            return Some(utc - sign * offset * 60_000);
        }
    }
    Some(utc)
}

/// The DER time for a signing-time attribute: UTCTime for 1950-2049 (RFC
/// 5280's rule), GeneralizedTime otherwise. Milliseconds are dropped.
pub fn der_time(ms: i64) -> Vec<u8> {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    let (h, mi, s) = (rest / 3600, rest / 60 % 60, rest % 60);
    if (1950..2050).contains(&y) {
        let text = format!("{:02}{mo:02}{d:02}{h:02}{mi:02}{s:02}Z", y % 100);
        crate::asn1::tlv(0x17, text.as_bytes())
    } else {
        let text = format!("{y:04}{mo:02}{d:02}{h:02}{mi:02}{s:02}Z");
        crate::asn1::tlv(0x18, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let ms = utc_ms(2026, 9, 23, 16, 0, 0);
        assert_eq!(iso(ms), "2026-09-23T16:00:00.000Z");
        assert_eq!(ymd(ms), "2026-09-23");
        assert_eq!(day_month_year(ms), "23 Sep 2026");
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(-1), "1969-12-31T23:59:59.999Z");
        assert_eq!(utc_ms(2026, 13, 1, 0, 0, 0), utc_ms(2027, 1, 1, 0, 0, 0));
        assert_eq!(parse_utc_time(b"260923160000Z"), Some(ms));
        assert_eq!(
            parse_utc_time(b"500101000000Z"),
            Some(utc_ms(1950, 1, 1, 0, 0, 0))
        );
        assert_eq!(parse_generalized_time(b"20260923160000Z"), Some(ms));
        assert_eq!(parse_utc_time(b"2609231600Z"), None);
        assert_eq!(parse_pdf_date("D:20260923180000+02'00'"), Some(ms));
        assert_eq!(parse_pdf_date("D:20260923160000Z"), Some(ms));
        assert_eq!(parse_pdf_date("D:20260923160000"), Some(ms));
        assert_eq!(parse_pdf_date("D:2026"), None);
        assert_eq!(der_time(ms), crate::asn1::tlv(0x17, b"260923160000Z"));
        assert_eq!(
            der_time(utc_ms(2050, 1, 1, 0, 0, 0)),
            crate::asn1::tlv(0x18, b"20500101000000Z")
        );
    }
}
