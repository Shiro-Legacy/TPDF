// SPDX-License-Identifier: MIT
//! Request fields read with the JavaScript checks engine.ts applies, so the
//! same inputs pass and fail: `Number.isInteger`, `Number.isFinite`,
//! `typeof x === "string"`, `Array.isArray` and strict booleans. A missing key
//! is `undefined`; a present `null` is not.
//!
//! Wire markers: a single-key object whose key begins with one `$` is a
//! marker (`{"$buffer": n}`, `{"$input": n}`, `{"$number": "NaN"}`). Data keys
//! that begin with `$` cross the wire with one more `$` in both directions
//! (`escape_key`, `unescape_key`), so data never looks like a marker.
use serde_json::{Value, json};
use std::borrow::Cow;

/// `Number.isInteger` (so `1.0` counts). Values beyond the i64 range
/// saturate, so range checks still refuse them.
pub fn as_int(value: &Value) -> Option<i64> {
    let number = as_number(value)?;
    (number.fract() == 0.0).then_some(number as i64)
}

/// `Number.isFinite`.
pub fn as_number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|number| number.is_finite())
}

/// Any JavaScript number, `NaN` and the infinities included (`typeof x ===
/// "number"`). JSON cannot carry those three, so they cross as
/// `{"$number": "NaN" | "Infinity" | "-Infinity"}`.
pub fn as_js_number(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| non_finite(value))
}

fn non_finite(value: &Value) -> Option<f64> {
    match marker(value, "$number")?.as_str()? {
        "NaN" => Some(f64::NAN),
        "Infinity" => Some(f64::INFINITY),
        "-Infinity" => Some(f64::NEG_INFINITY),
        _ => None,
    }
}

/// `typeof x === "number"`.
pub fn is_number(value: &Value) -> bool {
    as_js_number(value).is_some()
}

/// A reply value for a JavaScript number; the non-finite ones use the marker.
pub fn js_number_value(number: f64) -> Value {
    if number.is_finite() {
        return number.into();
    }
    let text = if number.is_nan() {
        "NaN"
    } else if number > 0.0 {
        "Infinity"
    } else {
        "-Infinity"
    };
    json!({ "$number": text })
}

/// `x === undefined || x === null`, the `??` test. `NaN` is not nullish.
pub fn nullish(value: Option<&Value>) -> bool {
    matches!(value, None | Some(Value::Null))
}

pub fn int(request: &Value, key: &str) -> Option<i64> {
    as_int(request.get(key)?)
}

pub fn number(request: &Value, key: &str) -> Option<f64> {
    as_number(request.get(key)?)
}

pub fn string<'a>(request: &'a Value, key: &str) -> Option<&'a str> {
    request.get(key)?.as_str()
}

/// Strictly `true` or `false`.
pub fn boolean(request: &Value, key: &str) -> Option<bool> {
    request.get(key)?.as_bool()
}

pub fn array<'a>(request: &'a Value, key: &str) -> Option<&'a [Value]> {
    request.get(key)?.as_array().map(Vec::as_slice)
}

/// `Array.isArray(x) && x.every(Number.isFinite)`.
pub fn numbers(request: &Value, key: &str) -> Option<Vec<f64>> {
    array(request, key)?.iter().map(as_number).collect()
}

/// JavaScript truthiness; a missing value is `undefined`.
pub fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::String(text)) => !text.is_empty(),
        Some(value) => as_js_number(value).is_none_or(|n| n != 0.0 && !n.is_nan()),
    }
}

/// `String(x)` for a request value; a missing value is `undefined`.
pub fn js_string(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return "undefined".into();
    };
    if let Some(number) = non_finite(value) {
        return js_number(number);
    }
    match value {
        Value::Null => "null".into(),
        Value::Bool(value) => value.to_string(),
        Value::Number(number) => js_number(number.as_f64().unwrap_or(f64::NAN)),
        Value::String(text) => text.clone(),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                item => js_string(Some(item)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// ECMAScript `Number::toString(x)` (radix 10) from the shortest round-trip
/// digits, taking the even last digit when two are equally close.
pub fn js_number(value: f64) -> String {
    if value.is_nan() {
        return "NaN".into();
    }
    if value == 0.0 {
        return "0".into();
    }
    if value.is_infinite() {
        return if value > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    let sign = if value < 0.0 { "-" } else { "" };
    let scientific = format!("{:e}", value.abs());
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return scientific;
    };
    let mut digits = mantissa.replace('.', "");
    let k = digits.len() as i32;
    let n = exponent.parse::<i32>().unwrap_or(0) + 1;
    if let Some(even) = even_tie(value.abs(), &digits, n - k) {
        digits = even;
    }
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        let (whole, fraction) = digits.split_at(n as usize);
        format!("{whole}.{fraction}")
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat(-n as usize))
    } else {
        let (first, rest) = digits.split_at(1);
        let point = if rest.is_empty() { "" } else { "." };
        let exponent_sign = if n > 0 { "+" } else { "-" };
        format!("{first}{point}{rest}e{exponent_sign}{}", (n - 1).abs())
    };
    format!("{sign}{body}")
}

/// The even neighbour of `digits` (last digit worth `10^scale`) when `value`
/// lies exactly halfway between the two and both read back as `value`. Rust's
/// shortest formatting resolves that tie upwards; ECMAScript takes the even one.
fn even_tie(value: f64, digits: &str, scale: i32) -> Option<String> {
    let shortest: u64 = digits.parse().ok()?;
    if shortest % 2 == 0 {
        return None;
    }
    [shortest - 1, shortest + 1].into_iter().find_map(|even| {
        let text = even.to_string();
        let usable = text.len() == digits.len() && !text.ends_with('0');
        let tied = usable && is_half_unit(value, shortest + even, scale);
        let reads_back = || format!("{text}e{scale}").parse::<f64>() == Ok(value);
        (tied && reads_back()).then_some(text)
    })
}

/// Whether `value` (positive, finite) is exactly `odd / 2 * 10^scale`.
fn is_half_unit(value: f64, odd: u64, scale: i32) -> bool {
    let bits = value.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let fraction = bits & ((1 << 52) - 1);
    let (mut mantissa, mut exponent) = if biased == 0 {
        (fraction, -1074)
    } else {
        (fraction | 1 << 52, biased - 1075)
    };
    if mantissa == 0 {
        return false;
    }
    let zeros = mantissa.trailing_zeros();
    mantissa >>= zeros;
    exponent += zeros as i32;
    // odd * 10^scale / 2 = odd * 5^scale * 2^(scale - 1), whose 5-part is odd.
    let odd = u128::from(odd);
    let cofactor = match 5u128.checked_pow(scale.unsigned_abs()) {
        Some(five) if scale >= 0 => odd.checked_mul(five),
        Some(five) if odd % five == 0 => Some(odd / five),
        _ => None,
    };
    cofactor == Some(u128::from(mantissa)) && exponent == scale - 1
}

/// The payload of a single-key marker object such as `{"$buffer": n}`.
pub fn marker<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let fields = value.as_object()?;
    if fields.len() != 1 {
        return None;
    }
    fields.get(key)
}

/// The index of a `{"$buffer": n}` placeholder.
pub fn buffer_index(value: &Value) -> Option<usize> {
    usize::try_from(marker(value, "$buffer")?.as_u64()?).ok()
}

/// A data key as it crosses the wire: one more `$` when it begins with `$`.
pub fn escape_key(key: &str) -> Cow<'_, str> {
    if key.starts_with('$') {
        Cow::Owned(format!("${key}"))
    } else {
        Cow::Borrowed(key)
    }
}

/// A data key read from the wire: one `$` fewer when it begins with `$`.
pub fn unescape_key(key: &str) -> &str {
    key.strip_prefix('$').unwrap_or(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_follow_number_is_integer() {
        let request: Value = serde_json::from_str(
            r#"{"a":1,"b":1.0,"c":1.5,"d":-0.0,"e":"1","f":null,"g":true,"h":1e300,"i":-1e300,"j":2e0}"#,
        )
        .unwrap();
        assert_eq!(int(&request, "a"), Some(1));
        assert_eq!(int(&request, "b"), Some(1));
        assert_eq!(int(&request, "c"), None);
        assert_eq!(int(&request, "d"), Some(0));
        assert_eq!(int(&request, "e"), None);
        assert_eq!(int(&request, "f"), None);
        assert_eq!(int(&request, "g"), None);
        assert_eq!(int(&request, "h"), Some(i64::MAX));
        assert_eq!(int(&request, "i"), Some(i64::MIN));
        assert_eq!(int(&request, "j"), Some(2));
        assert_eq!(int(&request, "missing"), None);
        assert_eq!(int(&json!([1]), "0"), None);
    }

    #[test]
    fn numbers_parse_to_the_double_javascript_sent() {
        // Without serde_json's float_roundtrip the first two come back one ULP off.
        for text in [
            "961.1757480989835",
            "23.172468304866854",
            "4497660818060879.5",
            "0.1",
            "1e-300",
        ] {
            let request: Value = serde_json::from_str(&format!(r#"{{"x":{text}}}"#)).unwrap();
            let sent = text.parse::<f64>().unwrap();
            assert_eq!(
                number(&request, "x").map(f64::to_bits),
                Some(sent.to_bits())
            );
        }
        let request: Value = serde_json::from_str(r#"{"x":4497660818060879.5}"#).unwrap();
        assert_eq!(int(&request, "x"), None, "Number.isInteger refuses it");
    }

    #[test]
    fn finite_numbers_strings_booleans_and_arrays() {
        let request = json!({"n": 2.5, "s": "x", "e": "", "t": true, "f": false, "z": 0,
            "a": [1, 2.5, -3], "m": [1, "2"], "o": {"k": 1}, "null": null,
            "nan": {"$number": "NaN"}, "inf": [{"$number": "Infinity"}]});
        assert_eq!(number(&request, "n"), Some(2.5));
        assert_eq!(number(&request, "s"), None);
        assert_eq!(number(&request, "null"), None);
        assert_eq!(number(&request, "nan"), None);
        assert_eq!(int(&request, "nan"), None);
        assert_eq!(numbers(&request, "inf"), None);
        assert_eq!(string(&request, "s"), Some("x"));
        assert_eq!(string(&request, "e"), Some(""));
        assert_eq!(string(&request, "n"), None);
        assert_eq!(boolean(&request, "t"), Some(true));
        assert_eq!(boolean(&request, "f"), Some(false));
        assert_eq!(boolean(&request, "z"), None);
        assert_eq!(boolean(&request, "null"), None);
        assert_eq!(array(&request, "a").map(<[Value]>::len), Some(3));
        assert_eq!(array(&request, "o"), None);
        assert_eq!(numbers(&request, "a"), Some(vec![1.0, 2.5, -3.0]));
        assert_eq!(numbers(&request, "m"), None);
        assert_eq!(numbers(&request, "o"), None);
        assert_eq!(numbers(&json!({"a": []}), "a"), Some(vec![]));
    }

    #[test]
    fn non_finite_numbers_are_numbers_but_not_nullish() {
        let request = json!({"nan": {"$number": "NaN"}, "inf": {"$number": "Infinity"},
            "neg": {"$number": "-Infinity"}, "bad": {"$number": "nan"},
            "two": {"$number": "NaN", "x": 1}, "null": null, "one": 1});
        assert!(as_js_number(&request["nan"]).is_some_and(f64::is_nan));
        assert_eq!(as_js_number(&request["inf"]), Some(f64::INFINITY));
        assert_eq!(as_js_number(&request["neg"]), Some(f64::NEG_INFINITY));
        assert_eq!(as_js_number(&request["one"]), Some(1.0));
        for key in ["nan", "inf", "neg", "one"] {
            assert!(is_number(&request[key]), "{key}");
            assert!(!nullish(request.get(key)), "{key}");
        }
        for key in ["bad", "two", "null"] {
            assert!(!is_number(&request[key]), "{key}");
        }
        assert!(nullish(request.get("null")) && nullish(request.get("missing")));
        assert_eq!(js_number_value(f64::NAN), request["nan"]);
        assert_eq!(js_number_value(f64::INFINITY), request["inf"]);
        assert_eq!(js_number_value(f64::NEG_INFINITY), request["neg"]);
        assert_eq!(js_number_value(1.5), json!(1.5));
    }

    #[test]
    fn truthiness_matches_javascript() {
        let request = json!({"z": 0, "f": false, "e": "", "n": null, "one": 1, "s": "0",
            "a": [], "o": {}, "neg": -0.0, "nan": {"$number": "NaN"},
            "inf": {"$number": "-Infinity"}});
        for key in ["z", "f", "e", "n", "missing", "neg", "nan"] {
            assert!(!truthy(request.get(key)), "{key}");
        }
        for key in ["one", "s", "a", "o", "inf"] {
            assert!(truthy(request.get(key)), "{key}");
        }
    }

    #[test]
    fn string_conversion_matches_javascript() {
        assert_eq!(js_string(None), "undefined");
        assert_eq!(js_string(Some(&json!(null))), "null");
        assert_eq!(js_string(Some(&json!(true))), "true");
        assert_eq!(js_string(Some(&json!("open"))), "open");
        assert_eq!(js_string(Some(&json!({"a": 1}))), "[object Object]");
        assert_eq!(js_string(Some(&json!([1, null, "x", [2, 3]]))), "1,,x,2,3");
        assert_eq!(js_string(Some(&json!(1.0))), "1");
        assert_eq!(js_string(Some(&json!({"$number": "NaN"}))), "NaN");
        assert_eq!(
            js_string(Some(&json!([{"$number": "-Infinity"}, 2]))),
            "-Infinity,2"
        );
        for (value, expected) in [
            (0.0, "0"),
            (-0.0, "0"),
            (1.5, "1.5"),
            (-42.0, "-42"),
            (0.1, "0.1"),
            (1e21, "1e+21"),
            (1.5e21, "1.5e+21"),
            (123_456_789_012_345_680_000.0, "123456789012345680000"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (-2.5e-8, "-2.5e-8"),
            (0.000_123, "0.000123"),
            (1234.5678, "1234.5678"),
            (f64::MAX, "1.7976931348623157e+308"),
            (5e-324, "5e-324"),
            (f64::NAN, "NaN"),
            (f64::NEG_INFINITY, "-Infinity"),
            // Exact ties between two shortest digit strings take the even one:
            // 273535373942.828125, 1354218820946191.25, -23926957400624.3125.
            (f64::from_bits(0x424f_d7fd_493b_6a00), "273535373942.82812"),
            (f64::from_bits(0x4313_3e9e_8f15_243d), "1354218820946191.2"),
            (f64::from_bits(0xc2b5_c2ed_cbe2_3050), "-23926957400624.312"),
            (f64::from_bits(0x431e_77da_1f41_9af9), "2144007003072190.2"),
        ] {
            assert_eq!(js_number(value), expected, "{value:e}");
        }
    }

    #[test]
    fn markers_and_escaped_keys() {
        assert_eq!(buffer_index(&json!({"$buffer": 3})), Some(3));
        assert_eq!(buffer_index(&json!({"$buffer": 3, "x": 1})), None);
        assert_eq!(buffer_index(&json!({"$buffer": -1})), None);
        assert_eq!(buffer_index(&json!({"$buffer": 1.5})), None);
        assert_eq!(buffer_index(&json!({"$buffer": "0"})), None);
        assert_eq!(buffer_index(&json!({"$$buffer": 0})), None, "escaped data");
        assert_eq!(buffer_index(&json!([0])), None);
        assert_eq!(buffer_index(&json!(null)), None);
        assert_eq!(marker(&json!({"$input": 2}), "$input"), Some(&json!(2)));
        assert_eq!(escape_key("$buffer"), "$$buffer");
        assert_eq!(escape_key("$$x"), "$$$x");
        assert_eq!(escape_key("plain"), "plain");
        assert_eq!(unescape_key("$$buffer"), "$buffer");
        assert_eq!(unescape_key("$$$x"), "$$x");
        assert_eq!(unescape_key("plain"), "plain");
    }
}
