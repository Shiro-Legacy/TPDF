// SPDX-License-Identifier: AGPL-3.0-or-later
//! Visual order for one text run of `scan-compose`: the Unicode
//! Bidirectional Algorithm (UAX #9) for a single paragraph without explicit
//! embeddings, isolates or bracket pairing. A PDF draws glyphs in visual
//! order and readers (MuPDF, the engine's view, PDFKit) turn right-to-left
//! runs back into logical order, so a Hebrew or Arabic run written in
//! typing order extracts reversed; this gives the order to draw in.
//!
//! No /ActualText: measured, MuPDF and the engine's view reverse a span's
//! ActualText of right-to-left characters again (as drawn text), so the
//! typed text extracted reversed; PDFKit ignores it. No mirrored brackets
//! either: all three copy a mirrored bracket as drawn, so "(חדש)" copied as
//! ")חדש(". Visual order alone extracts and searches as typed in all three.
//!
//! What it does: the paragraph direction from the first strong character;
//! combining marks stay with their base (W1), and in right-to-left text
//! they come before it, since readers reverse character by character;
//! numbers (W4-W7): one `.` `,` `/` `:` (or `+` `-`) between digits joins
//! them, `%`, currency signs and the other ET characters next to a number
//! go with it, and digits after a left-to-right letter are left to right;
//! neutrals between equal directions take that direction (N1, N2); levels
//! (I1, I2) and reversal by levels (L2). What it leaves out: explicit
//! formatting characters (dropped with the other format controls before
//! this runs, or kept as neutrals), bracket pairs (N0), mirroring (L4, see
//! above), Arabic letters before digits (W2: Arabic-Indic digits are
//! treated as European ones), and shaping, which no font here does. Runs
//! without right-to-left characters are returned unchanged.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    L,
    R,
    /// A digit (European or Arabic-Indic).
    En,
    /// `+` `-`: joins two digits (W4).
    Es,
    /// `.` `,` `/` `:` and the Arabic separators: joins two digits (W4).
    Cs,
    /// `#` `%` `°` `‰` currency signs: go with an adjacent number (W5).
    Et,
    /// White space, punctuation, symbols.
    N,
}

/// Whether `c` is a combining mark (it stays with the character before).
pub(super) fn mark(c: char) -> bool {
    let u = u32::from(c);
    [
        (0x0300, 0x036F),
        (0x0483, 0x0489),
        (0x0591, 0x05BD),
        (0x05BF, 0x05BF),
        (0x05C1, 0x05C2),
        (0x05C4, 0x05C5),
        (0x05C7, 0x05C7),
        (0x0610, 0x061A),
        (0x064B, 0x065F),
        (0x0670, 0x0670),
        (0x06D6, 0x06DC),
        (0x06DF, 0x06E4),
        (0x06E7, 0x06E8),
        (0x06EA, 0x06ED),
        (0x0711, 0x0711),
        (0x0730, 0x074A),
        (0x07A6, 0x07B0),
        (0x07EB, 0x07F3),
        (0x0816, 0x082D),
        (0x0859, 0x085B),
        (0x0898, 0x089F),
        (0x08CA, 0x08FF),
        (0x1AB0, 0x1AFF),
        (0x1DC0, 0x1DFF),
        (0x200C, 0x200D),
        (0x20D0, 0x20FF),
        (0xFB1E, 0xFB1E),
        (0xFE00, 0xFE0F),
        (0xFE20, 0xFE2F),
    ]
    .iter()
    .any(|&(a, b)| (a..=b).contains(&u))
}

fn class(c: char) -> Class {
    let u = u32::from(c);
    if c.is_ascii_digit() || (0x0660..=0x0669).contains(&u) || (0x06F0..=0x06F9).contains(&u) {
        return Class::En;
    }
    match c {
        '+' | '-' | '\u{207A}' | '\u{207B}' | '\u{208A}' | '\u{208B}' | '\u{2212}' => {
            return Class::Es;
        }
        '.' | ',' | '/' | ':' | '\u{A0}' | '\u{60C}' | '\u{66B}' | '\u{66C}' | '\u{202F}' => {
            return Class::Cs;
        }
        '#'
        | '$'
        | '%'
        | '\u{A2}'..='\u{A5}'
        | '\u{B0}'
        | '\u{B1}'
        | '\u{66A}'
        | '\u{2030}'
        | '\u{2031}'
        | '\u{2032}'..='\u{2034}'
        | '\u{20A0}'..='\u{20CF}'
        | '\u{2212}'..='\u{2213}' => {
            return Class::Et;
        }
        _ => {}
    }
    let rtl = [
        (0x0590, 0x08FF),
        (0xFB1D, 0xFDFF),
        (0xFE70, 0xFEFF),
        (0x1_0800, 0x1_0FFF),
        (0x1_E800, 0x1_EFFF),
    ];
    if rtl.iter().any(|&(a, b)| (a..=b).contains(&u)) {
        // Arabic punctuation (comma, semicolon, question mark) and symbols in
        // the blocks are still right to left, as their bidi class AL says.
        return Class::R;
    }
    if c.is_alphanumeric() {
        Class::L
    } else {
        Class::N
    }
}

/// Whether `c` is drawn with no glyph and no advance when the CJK font
/// lacks it: a combining mark, or a zero-width format character (RLM).
pub(super) fn zero_width(c: char) -> bool {
    mark(c) || matches!(c, '\u{200B}'..='\u{200F}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}')
}

/// Whether `text` has a right-to-left character.
pub(super) fn has_rtl(text: &[char]) -> bool {
    text.iter().any(|&c| class(c) == Class::R)
}

/// W4-W6 on the clusters' classes: a single separator between two digits
/// joins them, ET next to a number goes with it, every other separator is
/// a neutral.
fn numbers(classes: &mut [Class]) {
    for i in 1..classes.len().saturating_sub(1) {
        let joins = match classes[i] {
            Class::Es => matches!(classes[i - 1], Class::En),
            Class::Cs => true,
            _ => false,
        };
        if joins && classes[i - 1] == Class::En && classes[i + 1] == Class::En {
            classes[i] = Class::En;
        }
    }
    let mut i = 0;
    while i < classes.len() {
        if classes[i] != Class::Et {
            i += 1;
            continue;
        }
        let end = (i..classes.len())
            .find(|&j| classes[j] != Class::Et)
            .unwrap_or(classes.len());
        let next_to_number =
            (i > 0 && classes[i - 1] == Class::En) || classes.get(end) == Some(&Class::En);
        if next_to_number {
            classes[i..end].iter_mut().for_each(|c| *c = Class::En);
        }
        i = end;
    }
    for c in classes.iter_mut() {
        if matches!(c, Class::Es | Class::Cs | Class::Et) {
            *c = Class::N;
        }
    }
}

/// `text` (logical order) in visual order, left to right; None when it has
/// no right-to-left character. In right-to-left text a cluster's combining
/// marks come before its base, so a reader that reverses the run character
/// by character gets the base first again.
pub(super) fn visual(text: &[char]) -> Option<Vec<char>> {
    if !has_rtl(text) {
        return None;
    }
    // Clusters: a character and the combining marks after it (W1).
    let mut clusters: Vec<(usize, usize)> = Vec::new();
    for (i, &c) in text.iter().enumerate() {
        match clusters.last_mut() {
            Some((_, end)) if mark(c) => *end = i + 1,
            _ => clusters.push((i, i + 1)),
        }
    }
    let mut classes: Vec<Class> = clusters
        .iter()
        .map(|&(start, _)| class(text[start]))
        .collect();
    let strong = |c: Class| matches!(c, Class::L | Class::R);
    let paragraph = classes
        .iter()
        .copied()
        .find(|&c| strong(c))
        .unwrap_or(Class::L);
    numbers(&mut classes);
    // W7: a digit after L (or at the start of an LTR paragraph) is L.
    let mut last = paragraph;
    for c in classes.iter_mut() {
        match *c {
            Class::L | Class::R => last = *c,
            Class::En if last == Class::L => *c = Class::L,
            _ => {}
        }
    }
    // N1, N2: digits count as R next to neutrals.
    let direction = |c: Class| if c == Class::En { Class::R } else { c };
    let mut i = 0;
    while i < classes.len() {
        if classes[i] != Class::N {
            i += 1;
            continue;
        }
        let end = (i..classes.len())
            .find(|&j| classes[j] != Class::N)
            .unwrap_or(classes.len());
        let before = if i == 0 {
            paragraph
        } else {
            direction(classes[i - 1])
        };
        let after = classes.get(end).map_or(paragraph, |&c| direction(c));
        let resolved = if before == after { before } else { paragraph };
        classes[i..end].iter_mut().for_each(|c| *c = resolved);
        i = end;
    }
    // I1, I2.
    let base: u8 = if paragraph == Class::R { 1 } else { 0 };
    let levels: Vec<u8> = (classes.iter())
        .map(|&c| match (base, c) {
            (0, Class::R) => 1,
            (0, Class::En) => 2,
            (0, _) => 0,
            (_, Class::R) => 1,
            _ => 2,
        })
        .collect();
    // L2: from the highest level down to 1, reverse every run at or above it.
    let mut order: Vec<usize> = (0..clusters.len()).collect();
    let highest = levels.iter().copied().max().unwrap_or(0);
    for level in (1..=highest).rev() {
        let mut i = 0;
        while i < order.len() {
            if levels[order[i]] < level {
                i += 1;
                continue;
            }
            let end = (i..order.len())
                .find(|&j| levels[order[j]] < level)
                .unwrap_or(order.len());
            order[i..end].reverse();
            i = end;
        }
    }
    let mut out = Vec::with_capacity(text.len());
    for index in order {
        let cluster = &text[clusters[index].0..clusters[index].1];
        if levels[index] % 2 == 1 {
            out.extend(cluster.iter().rev());
        } else {
            out.extend_from_slice(cluster);
        }
    }
    // A line that starts with a mark (a right-to-left run whose last letter
    // has one): readers give a mark the direction of the character before
    // it, and with none it stays left to right, at the start of the line.
    // A zero-width RIGHT-TO-LEFT MARK before it keeps it with its letter.
    if out.first().is_some_and(|&c| mark(c)) {
        out.insert(0, '\u{200F}');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(text: &str) -> Option<String> {
        visual(&text.chars().collect::<Vec<_>>()).map(|chars| chars.into_iter().collect())
    }

    #[test]
    fn runs_reorder_as_uax9_for_the_simple_cases() {
        assert_eq!(v("hello"), None);
        assert_eq!(v("שלום עולם").as_deref(), Some("םלוע םולש"));
        // Digits keep their order; brackets are not mirrored.
        assert_eq!(v("שנת 2026 (חדש)").as_deref(), Some(")שדח( 2026 תנש"));
        // An LTR paragraph with an RTL word, and digits after it.
        assert_eq!(v("abc שלום 123").as_deref(), Some("abc 123 םולש"));
        // An RTL paragraph with an LTR word keeps the word's order.
        assert_eq!(v("שלום world טוב").as_deref(), Some("בוט world םולש"));
        // A combining mark comes before its base in right-to-left text.
        assert_eq!(
            v("שָׁלוֹם").as_deref(),
            Some("\u{5DD}\u{5B9}\u{5D5}\u{5DC}\u{5C1}\u{5B8}\u{5E9}")
        );
        assert_eq!(v("مرحبا").as_deref(), Some("ابحرم"));
        // A final mark would start the line: an RLM goes before it.
        assert_eq!(v("بِ").as_deref(), Some("\u{200F}\u{650}\u{628}"));
    }

    #[test]
    fn numbers_keep_their_separators_signs_and_currency() {
        assert_eq!(v("מחיר 3.50").as_deref(), Some("3.50 ריחמ"));
        assert_eq!(v("תאריך 12/05/2026").as_deref(), Some("12/05/2026 ךיראת"));
        assert_eq!(v("סך 1,000 ש").as_deref(), Some("ש 1,000 ךס"));
        assert_eq!(v("הנחה 50% היום").as_deref(), Some("םויה 50% החנה"));
        assert_eq!(v("מחיר ₪25 או $30").as_deref(), Some("$30 וא ₪25 ריחמ"));
        assert_eq!(v("שעה 10:30").as_deref(), Some("10:30 העש"));
        // A separator not between digits stays neutral: the sentence's dot.
        assert_eq!(v("עלות 5.").as_deref(), Some(".5 תולע"));
        // Two separators in a row are no longer one number.
        assert_eq!(v("א 1..2").as_deref(), Some("2..1 א"));
    }
}
