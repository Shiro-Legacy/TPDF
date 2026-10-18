// SPDX-License-Identifier: AGPL-3.0-or-later
//! Independent exact-name review of the builtin metrics.
use tandem_pdf_engine::{
    builtin_metrics::{BuiltinFace, BuiltinMetricLookup, BuiltinMetrics},
    simple_encoding::{Difference, EncodingLimits, GlyphUnicode, NamedEncoding, SimpleEncoding},
};

#[test]
fn exact_glyph_metrics_do_not_require_unicode_registry_membership() {
    for face in [BuiltinFace::Helvetica, BuiltinFace::TimesRoman] {
        let mut checked = 0;
        for (name, expected) in face.glyphs() {
            if name == b".notdef" {
                continue;
            }
            let encoding = SimpleEncoding::from_named(NamedEncoding::WinAnsi)
                .with_differences(
                    &[Difference::Code(65), Difference::Name(name)],
                    EncodingLimits::default(),
                )
                .unwrap();
            if matches!(
                encoding.lookup_unicode(65),
                GlyphUnicode::UnknownGlyphName(_)
            ) {
                let metrics = BuiltinMetrics::from_encoding(face, encoding);
                assert_eq!(metrics.lookup(65), BuiltinMetricLookup::Advance(expected));
                checked += 1;
            }
        }
        assert!(checked > 100, "must exercise the metrics-only registry");
    }
}
