//! Small JavaScript semantics the port must reproduce exactly: number to
//! string (`String(n)`), `Math.round`, and the whitespace set of `\s` and
//! `String#trim`.

/// JavaScript's `String(n)` for a number: `3` → "3", `2.5` → "2.5",
/// `-0` → "0", `1e21` → "1e+21", `1e-7` → "1e-7", NaN → "NaN".
pub fn js_num(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if n == 0.0 {
        return "0".into();
    }
    let abs = n.abs();
    if (1e-6..1e21).contains(&abs) {
        // Rust's Display prints the shortest round-tripping digits without an
        // exponent, as JS does in this range.
        return format!("{n}");
    }
    // Exponent form: Rust's `{:e}` gives the shortest digits ("1.5e-7", "1e21");
    // JS writes a sign on a positive exponent ("1e+21").
    let text = format!("{n:e}");
    match text.split_once('e') {
        Some((mantissa, exp)) if !exp.starts_with('-') => format!("{mantissa}e+{exp}"),
        _ => text,
    }
}

/// JavaScript's `Math.round`: halves round toward +∞ (`-2.5` → `-2`, `2.5` → `3`).
pub fn js_round(x: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    let floor = x.floor();
    let rounded = if x - floor >= 0.5 { floor + 1.0 } else { floor };
    // Math.round(-0.2) is -0; callers that print it go through js_num ("0").
    if rounded == 0.0 && x < 0.0 {
        -0.0
    } else {
        rounded
    }
}

/// A character in JavaScript's `\s` (and what `String#trim` removes): Unicode
/// White_Space except U+0085, plus U+FEFF.
pub fn is_js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// `String#trim`.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

/// A JS regex word character (`\w` without the `u` flag): `[A-Za-z0-9_]`.
pub fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_as_js() {
        assert_eq!(js_num(3.0), "3");
        assert_eq!(js_num(2.5), "2.5");
        assert_eq!(js_num(-0.0), "0");
        assert_eq!(js_num(12.5), "12.5");
        assert_eq!(js_num(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(js_num(1e21), "1e+21");
        assert_eq!(js_num(1.5e-7), "1.5e-7");
        assert_eq!(js_num(123456789012.0), "123456789012");
        assert_eq!(js_num(f64::NAN), "NaN");
    }

    #[test]
    fn round_halves_up() {
        assert_eq!(js_round(2.5), 3.0);
        assert_eq!(js_round(-2.5), -2.0);
        assert_eq!(js_round(0.49999999999999994), 0.0);
        assert_eq!(js_num(js_round(-0.2)), "0");
        assert_eq!(js_round(119.99), 120.0);
    }

    #[test]
    fn trims_js_whitespace() {
        assert_eq!(js_trim("\u{feff} a \n"), "a");
        assert_eq!(js_trim("\u{85}a"), "\u{85}a");
    }
}
