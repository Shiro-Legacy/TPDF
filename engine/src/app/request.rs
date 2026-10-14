// SPDX-License-Identifier: AGPL-3.0-or-later
//! Request fields read with the JavaScript checks engine.ts applies, so the
//! same inputs pass and fail: `Number.isInteger`, `Number.isFinite`,
//! `typeof x === "string"`, `Array.isArray` and strict booleans. A missing key
//! is `undefined`; a present `null` is not.
use serde_json::Value;

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
        Some(Value::Number(number)) => number.as_f64().is_some_and(|n| n != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

/// `String(x)` for a request value; a missing value is `undefined`.
pub fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Number(number)) => js_number(number.as_f64().unwrap_or(f64::NAN)),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| match item {
                Value::Null => String::new(),
                item => js_string(Some(item)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// ECMAScript `Number::toString(x)` (radix 10) from the shortest round-trip digits.
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
    let digits = mantissa.replace('.', "");
    let k = digits.len() as i32;
    let n = exponent.parse::<i32>().unwrap_or(0) + 1;
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

/// The index of a `{"$buffer": n}` placeholder.
pub fn buffer_index(value: &Value) -> Option<usize> {
    let fields = value.as_object()?;
    if fields.len() != 1 {
        return None;
    }
    usize::try_from(fields.get("$buffer")?.as_u64()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn finite_numbers_strings_booleans_and_arrays() {
        let request = json!({"n": 2.5, "s": "x", "e": "", "t": true, "f": false, "z": 0,
            "a": [1, 2.5, -3], "m": [1, "2"], "o": {"k": 1}, "null": null});
        assert_eq!(number(&request, "n"), Some(2.5));
        assert_eq!(number(&request, "s"), None);
        assert_eq!(number(&request, "null"), None);
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
    fn truthiness_matches_javascript() {
        let request = json!({"z": 0, "f": false, "e": "", "n": null, "one": 1, "s": "0",
            "a": [], "o": {}, "neg": -0.0});
        for key in ["z", "f", "e", "n", "missing", "neg"] {
            assert!(!truthy(request.get(key)), "{key}");
        }
        for key in ["one", "s", "a", "o"] {
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
        ] {
            assert_eq!(js_number(value), expected, "{value:e}");
        }
    }

    #[test]
    fn buffer_placeholders_are_exact() {
        assert_eq!(buffer_index(&json!({"$buffer": 3})), Some(3));
        assert_eq!(buffer_index(&json!({"$buffer": 3, "x": 1})), None);
        assert_eq!(buffer_index(&json!({"$buffer": -1})), None);
        assert_eq!(buffer_index(&json!({"$buffer": 1.5})), None);
        assert_eq!(buffer_index(&json!({"$buffer": "0"})), None);
        assert_eq!(buffer_index(&json!([0])), None);
        assert_eq!(buffer_index(&json!(null)), None);
    }
}
