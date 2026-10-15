// SPDX-License-Identifier: AGPL-3.0-or-later
//! Matrix and number helpers of the edit ops (content-edit.ts), in PDF
//! row-vector order: `[a b c d e f]` maps `(x, y)` to
//! `(a x + c y + e, b x + d y + f)`.
use super::scan::Orient;
use crate::app::request::js_number;

pub const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `left x right` for linear parts `[a b c d]`.
pub fn multiply(left: &[f64; 4], right: &[f64; 4]) -> [f64; 4] {
    let [a, b, c, d] = *left;
    let [e, f, g, h] = *right;
    [a * e + b * g, a * f + b * h, c * e + d * g, c * f + d * h]
}

/// `left x right` for whole matrices.
pub fn affine(left: &[f64; 6], right: &[f64; 6]) -> [f64; 6] {
    let [a, b, c, d] = multiply(
        &[left[0], left[1], left[2], left[3]],
        &[right[0], right[1], right[2], right[3]],
    );
    [
        a,
        b,
        c,
        d,
        left[4] * right[0] + left[5] * right[2] + right[4],
        left[4] * right[1] + left[5] * right[3] + right[5],
    ]
}

/// The inverse of a linear part, None when it is not invertible.
pub fn invert(m: &[f64; 4]) -> Option<[f64; 4]> {
    let [a, b, c, d] = *m;
    let det = a * d - b * c;
    if !det.is_finite() || det.abs() < 1e-9 {
        return None;
    }
    Some([d / det, -b / det, -c / det, a / det])
}

/// The bounding box of `rect`'s corners under `m`.
pub fn map_rect(m: &[f64; 6], rect: &[f64; 4]) -> [f64; 4] {
    let corners = [
        [rect[0], rect[1]],
        [rect[2], rect[1]],
        [rect[0], rect[3]],
        [rect[2], rect[3]],
    ];
    let mut out = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for [x, y] in corners {
        let (px, py) = (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
        out = [
            out[0].min(px),
            out[1].min(py),
            out[2].max(px),
            out[3].max(py),
        ];
    }
    out
}

/// content-edit.ts `editViewBox`: a user-space box in view space.
pub fn view_box(rect: &[f64; 4], m: &[f64; 6]) -> [f64; 4] {
    map_rect(m, rect)
}

/// content-edit.ts `editOrientOf`.
pub fn orient_of(m: &[f64; 6]) -> Option<Orient> {
    if m.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let [a, b, c, d, _, _] = *m;
    let sign = |v: f64| if v > 0.0 { 1.0 } else { -1.0 };
    if b == 0.0 && c == 0.0 && a != 0.0 && d != 0.0 {
        return Some(Orient {
            swap: false,
            sx: sign(a),
            sy: sign(d),
        });
    }
    if a == 0.0 && d == 0.0 && b != 0.0 && c != 0.0 {
        return Some(Orient {
            swap: true,
            sx: sign(c),
            sy: sign(b),
        });
    }
    None
}

/// JavaScript `Math.round`: halves round up.
pub fn js_round(value: f64) -> f64 {
    let floor = value.floor();
    if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

/// engine-core.ts `round2`.
pub fn round2(value: f64) -> f64 {
    js_round(value * 100.0) / 100.0
}

/// content-edit.ts `editNumber`: at most four decimals, `-0` as `0`.
pub fn edit_number(value: f64) -> String {
    let rounded = js_round(value * 10000.0) / 10000.0;
    if rounded == 0.0 {
        return "0".into();
    }
    js_number(rounded)
}

/// The largest magnitude written into content: the integer limit of
/// ISO 32000 Annex C (2^31 - 1). Within it `edit_number` never writes an
/// exponent (JavaScript notation from 1e21, which PDF has no syntax for) or
/// `Infinity`, and a number written without a fraction reads as a 32-bit
/// integer.
pub const MAX_WRITTEN: f64 = 2_147_483_647.0;

/// Whether `value` is written as a plain PDF number (see `MAX_WRITTEN`).
pub fn writable(value: f64) -> bool {
    value.is_finite() && value.abs() <= MAX_WRITTEN
}

/// The value an `edit_number` string reads back as.
pub fn number_value(text: &str) -> f64 {
    text.parse().unwrap_or(f64::NAN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_written_as_javascript_would() {
        assert_eq!(edit_number(1.0), "1");
        assert_eq!(edit_number(-0.00001), "0");
        assert_eq!(edit_number(2.5), "2.5");
        assert_eq!(edit_number(1.23456), "1.2346");
        // The products below are not exact halves in binary; values from node.
        assert_eq!(edit_number(-1.00005), "-1.0001");
        assert_eq!(edit_number(-2.00005), "-2");
        assert_eq!(round2(-2.555), -2.56);
        assert_eq!(js_round(-2.5), -2.0);
    }

    #[test]
    fn writable_numbers_never_take_an_exponent() {
        assert!(writable(MAX_WRITTEN) && writable(-MAX_WRITTEN) && writable(0.0));
        for value in [1e21, -1e25, f64::INFINITY, f64::NAN, MAX_WRITTEN + 1.0] {
            assert!(!writable(value), "{value}");
        }
        assert!(!edit_number(MAX_WRITTEN).contains('e'));
        assert!(!writable(number_value(&edit_number(1e21))));
    }
}
