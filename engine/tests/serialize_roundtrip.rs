// SPDX-License-Identifier: MIT
//! Every object the writer prints reads back as the same object, in both
//! serialize styles: generated values (empty names in every position among
//! them), then the shapes an empty name used to merge with.
use tandem_pdf_engine::object::{Dictionary, Object, ObjectId};
use tandem_pdf_engine::parser::{ParseLimits, parse_complete};
use tandem_pdf_engine::serialize::{SerializeStyle, WriteLimits, serialize_object_with_style};

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn bytes(&mut self, max: u64, nul: bool) -> Vec<u8> {
        let len = match self.below(4) {
            0 => 0,
            _ => self.below(max + 1),
        };
        (0..len)
            .map(|_| match self.below(3) {
                // Delimiters, whitespace, '#' and the escapes' own bytes.
                0 => b"/()<>[]{}%# \n\r\t\\"[self.below(16) as usize],
                1 => b'0' + self.below(10) as u8,
                _ => self.below(256) as u8,
            })
            .map(|b| if b == 0 && !nul { 1 } else { b })
            .collect()
    }
}

fn value(rng: &mut Rng, depth: usize) -> Object {
    let kinds = if depth >= 4 { 9 } else { 11 };
    match rng.below(kinds) {
        0 => Object::Null,
        1 => Object::Boolean(rng.below(2) == 0),
        2 => Object::Integer(match rng.below(3) {
            0 => rng.below(10) as i64,
            1 => -(rng.below(100_000) as i64),
            _ => rng.next() as i64,
        }),
        3 => Object::Real(match rng.below(4) {
            0 => rng.below(1000) as f64 / 8.0,
            1 => -(rng.below(1_000_000) as f64) / 1000.0,
            2 => f64::from(f32::from_bits(rng.next() as u32 & 0x7EFF_FFFF)),
            // Any finite binary64, sign included.
            _ => loop {
                let v = f64::from_bits(rng.next());
                if v.is_finite() {
                    break v;
                }
            },
        }),
        // Empty names are frequent: every neighbour must keep its own token.
        4 | 5 => Object::Name(if rng.below(2) == 0 {
            Vec::new()
        } else {
            rng.bytes(6, false)
        }),
        6 => Object::String(rng.bytes(12, true)),
        7 | 8 => Object::Reference(
            ObjectId::new(rng.below(100_000) as u32 + 1, rng.below(3) as u16).unwrap(),
        ),
        9 => Object::Array((0..rng.below(6)).map(|_| value(rng, depth + 1)).collect()),
        _ => {
            let mut dictionary = Dictionary::new();
            for _ in 0..rng.below(6) {
                let key = match rng.below(3) {
                    0 => Vec::new(),
                    _ => rng.bytes(5, false),
                };
                dictionary.insert(key, value(rng, depth + 1));
            }
            Object::Dictionary(dictionary)
        }
    }
}

/// What the compact style promises to keep (spec 01 section 3.7.5): every
/// real reads back exactly, except that an integral real with |v| < 2^63 is
/// written as an integer and reads back as the equal Integer.
/// 2^63, exactly.
const TWO_63: f64 = 9_223_372_036_854_775_808_u64 as f64;

fn compact_model(value: &Object) -> Object {
    match value {
        Object::Real(v) if v.fract() == 0.0 && (-TWO_63..TWO_63).contains(v) => {
            Object::Integer(*v as i64)
        }
        Object::Array(items) => Object::Array(items.iter().map(compact_model).collect()),
        Object::Dictionary(entries) => Object::Dictionary(
            entries
                .iter()
                .map(|(key, item)| (key.clone(), compact_model(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn round_trip(value: &Object, style: SerializeStyle) -> Object {
    let bytes = serialize_object_with_style(value, WriteLimits::default(), style).unwrap();
    parse_complete(&bytes, ParseLimits::default()).unwrap_or_else(|error| {
        panic!(
            "{style:?} output of {value:?} did not parse ({error}): {}",
            String::from_utf8_lossy(&bytes)
        )
    })
}

#[test]
fn every_object_reads_back_identically_in_both_styles() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut empty_names = 0;
    for _ in 0..20_000 {
        let value = value(&mut rng, 0);
        let printed =
            serialize_object_with_style(&value, WriteLimits::default(), SerializeStyle::Canonical)
                .unwrap();
        empty_names += printed
            .windows(2)
            .filter(|w| w[0] == b'/' && matches!(w[1], b' ' | b']'))
            .count();
        assert_eq!(round_trip(&value, SerializeStyle::Canonical), value);
        assert_eq!(
            round_trip(&value, SerializeStyle::Compact),
            compact_model(&value),
            "{value:?}"
        );
    }
    assert!(empty_names > 1_000, "{empty_names}");
}

#[test]
fn an_empty_name_keeps_a_delimiter_before_a_regular_token() {
    let empty = || Object::Name(Vec::new());
    let reference = Object::Reference(ObjectId::new(12, 0).unwrap());
    let cases = [
        // An empty key before each kind of value.
        Object::Dictionary(Dictionary::from([(Vec::new(), Object::Integer(5))])),
        Object::Dictionary(Dictionary::from([(Vec::new(), reference.clone())])),
        Object::Dictionary(Dictionary::from([(Vec::new(), Object::Boolean(true))])),
        Object::Dictionary(Dictionary::from([(Vec::new(), Object::Null)])),
        Object::Dictionary(Dictionary::from([(Vec::new(), Object::Real(0.5))])),
        Object::Dictionary(Dictionary::from([(Vec::new(), empty())])),
        // An empty value before the next key and in arrays.
        Object::Dictionary(Dictionary::from([
            (b"A".to_vec(), empty()),
            (b"B".to_vec(), Object::Integer(1)),
        ])),
        Object::Array(vec![
            empty(),
            Object::Integer(1),
            empty(),
            reference,
            empty(),
        ]),
        Object::Array(vec![empty(), empty(), Object::Real(-1.25), empty()]),
    ];
    for value in cases {
        for style in [SerializeStyle::Canonical, SerializeStyle::Compact] {
            assert_eq!(round_trip(&value, style), value, "{style:?}");
        }
    }
    let compact = |value: &Object| {
        serialize_object_with_style(value, WriteLimits::default(), SerializeStyle::Compact).unwrap()
    };
    assert_eq!(
        compact(&Object::Dictionary(Dictionary::from([(
            Vec::new(),
            Object::Integer(5)
        )]))),
        b"<</ 5>>"
    );
    assert_eq!(
        compact(&Object::Array(vec![empty(), empty(), Object::Integer(3)])),
        b"[// 3]"
    );
}

/// A whole document with empty names in a dictionary key, a value, an
/// array and a stream dictionary, written in every style the writer has
/// (the app's compact save, the canonical rewrite, the repair's output),
/// reopens with the same objects.
#[test]
fn documents_with_empty_names_reopen_identically_in_every_writer_style() {
    use tandem_pdf_engine::document::{Document, DocumentLimits};
    use tandem_pdf_engine::repair::{RepairLimits, rebuild};
    use tandem_pdf_engine::writer::{
        DocumentWriteLimits, Encryption, WriteOptions, rewrite_document, write_document,
    };
    let objects: [&[u8]; 4] = [
        b"<</Type/Catalog/Pages 2 0 R/ 3 0 R>>",
        b"<</Type/Pages/Kids[3 0 R]/Count 1/A/ /B[/ 1 0 R/ 2.5/]>>",
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 10 10]/Contents 4 0 R/ true>>",
        b"<</ 7/Length 2>>\nstream\nq\nendstream",
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<</Size 5/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    let limits = DocumentLimits::default();
    let source = Document::open(&bytes, limits).unwrap();
    let app = WriteOptions {
        encryption: Encryption::Plaintext,
        ..WriteOptions::APP
    };
    let writes = [
        write_document(&source, DocumentWriteLimits::default(), app).unwrap(),
        rewrite_document(&source, DocumentWriteLimits::default()).unwrap(),
        rebuild(&bytes, RepairLimits::default()).unwrap().bytes,
    ];
    for written in writes {
        let reopened = Document::open(&written, limits).unwrap();
        for object in source.objects() {
            let copy = reopened.object(object.id).unwrap();
            assert_eq!(
                copy.value,
                object.value,
                "{}",
                String::from_utf8_lossy(&written)
            );
            assert_eq!(
                reopened.stream_data(object.id).unwrap(),
                source.stream_data(object.id).unwrap()
            );
        }
    }
}
