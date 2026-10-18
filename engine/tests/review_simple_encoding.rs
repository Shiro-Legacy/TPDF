// SPDX-License-Identifier: MIT
//! Independent override model, byte ownership and processed-work boundaries.
use tandem_pdf_engine::object::Object;
use tandem_pdf_engine::simple_encoding::{
    Difference, EncodingErrorKind, EncodingLimits, GlyphUnicode, NamedEncoding, SimpleEncoding,
};

#[test]
fn all_slots_own_opaque_bytes_and_survive_transactional_followup() {
    let first = {
        let names: Vec<_> = (0..=255).map(|n| vec![0xff, n, 0, b'A']).collect();
        let entries: Vec<_> = std::iter::once(Difference::Code(0))
            .chain(names.iter().map(|name| Difference::Name(name)))
            .collect();
        SimpleEncoding::from_named(NamedEncoding::Standard)
            .with_differences(&entries, EncodingLimits::default())
            .unwrap()
    };
    let second = first
        .with_differences(
            &[
                Difference::Code(255),
                Difference::Name(b".notdef"),
                Difference::Code(0),
                Difference::Name(b"A"),
            ],
            EncodingLimits::default(),
        )
        .unwrap();
    let clone = second
        .with_differences(
            &[],
            EncodingLimits {
                max_items: 0,
                max_name_bytes: 0,
                max_processed_name_bytes: 0,
            },
        )
        .unwrap();
    drop(second);
    for code in 0..=255 {
        let wanted = [0xff, code, 0, b'A'];
        assert_eq!(first.glyph_name(code), Some(wanted.as_slice()));
        assert_eq!(
            first.lookup_unicode(code),
            GlyphUnicode::UnknownGlyphName(&wanted)
        );
        if code > 0 && code < 255 {
            assert_eq!(clone.glyph_name(code), Some(wanted.as_slice()));
        }
    }
    assert_eq!(clone.lookup_unicode(0), GlyphUnicode::Unicode(&['A']));
    assert_eq!(clone.glyph_name(255), Some(b".notdef".as_slice()));
    assert_eq!(clone.lookup_unicode(255), GlyphUnicode::UndefinedCode);
}

#[test]
fn independent_override_model_matches_batched_and_successive_updates() {
    for seed in 0..32u64 {
        let mut random = seed + 1;
        let mut expected: [Option<Vec<u8>>; 256] = std::array::from_fn(|i| {
            SimpleEncoding::from_named(NamedEncoding::MacExpert)
                .glyph_name(i as u8)
                .map(<[u8]>::to_vec)
        });
        let mut names = Vec::new();
        let mut codes = Vec::new();
        for index in 0..300u16 {
            random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
            let code = (random >> 32) as u8;
            let name = vec![0xff, index as u8, (index >> 8) as u8, code];
            expected[usize::from(code)] = Some(name.clone());
            names.push(name);
            codes.push(code);
        }
        let entries: Vec<_> = codes
            .iter()
            .zip(&names)
            .flat_map(|(&code, name)| [Difference::Code(code), Difference::Name(name)])
            .collect();
        let base = SimpleEncoding::from_named(NamedEncoding::MacExpert);
        let batch = base
            .with_differences(&entries, EncodingLimits::default())
            .unwrap();
        let mut successive = base;
        for chunk in entries.chunks(2) {
            successive = successive
                .with_differences(chunk, EncodingLimits::default())
                .unwrap();
        }
        assert_eq!(batch, successive);
        for (code, name) in expected.iter().enumerate() {
            assert_eq!(batch.glyph_name(code as u8), name.as_deref());
        }
    }
}

#[test]
fn overwritten_work_is_charged_at_exact_default_payload_limit() {
    let name = [b'x'; 127];
    // 2064*127 + 16 = 262144 bytes. Each complete run starts at code zero.
    let mut entries = Vec::new();
    for n in 0..2064 {
        if n % 256 == 0 {
            entries.push(Difference::Code(0));
        }
        entries.push(Difference::Name(&name));
    }
    entries.push(Difference::Name(b"sixteenbyteglyph"));
    let base = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    let limits = EncodingLimits::default();
    assert_eq!(b"sixteenbyteglyph".len(), 16);
    let exact = base.with_differences(&entries, limits).unwrap();
    entries.push(Difference::Name(b"A"));
    let err = base.with_differences(&entries, limits).unwrap_err();
    assert_eq!(err.index, Some(entries.len() - 1));
    assert_eq!(
        err.kind,
        EncodingErrorKind::LimitExceeded("processed name bytes")
    );
    assert_eq!(exact.glyph_name(16), Some(b"sixteenbyteglyph".as_slice()));
    assert_eq!(base, SimpleEncoding::from_named(NamedEncoding::WinAnsi));
}

#[test]
fn overwritten_invalid_entries_and_empty_name_cannot_hide_behind_later_reset() {
    let base = SimpleEncoding::from_named(NamedEncoding::WinAnsi);
    for bad in [
        Object::Integer(i64::MIN),
        Object::Integer(256),
        Object::Real(0.),
        Object::Name(vec![]),
        Object::Array(vec![Object::String(vec![1; 100_000])]),
    ] {
        let expected = match &bad {
            Object::Integer(_) => EncodingErrorKind::CodeOutOfRange,
            Object::Name(_) => EncodingErrorKind::EmptyName,
            _ => EncodingErrorKind::ExpectedCodeOrName,
        };
        let values = Object::Array(vec![
            Object::Integer(65),
            Object::Name(b"B".to_vec()),
            bad,
            Object::Integer(65),
            Object::Name(b"C".to_vec()),
        ]);
        let err = base
            .with_object_differences(&values, EncodingLimits::default())
            .unwrap_err();
        assert_eq!(err.index, Some(2));
        assert_eq!(err.kind, expected);
        assert_eq!(base.glyph_name(65), Some(b"A".as_slice()));
    }
}
