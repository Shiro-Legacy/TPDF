// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use tandem_pdf_engine::document::{Document, DocumentLimits};
use tandem_pdf_engine::object::{Dictionary, Object};
use tandem_pdf_engine::parser::{DuplicateKeyPolicy, ParseLimits, Parser, parse_complete};
use tandem_pdf_engine::repair::{RepairLimits, rebuild};
use tandem_pdf_engine::serialize::{SerializeStyle, WriteLimits, serialize_object_with_style};
use tandem_pdf_engine::writer::{DocumentWriteLimits, WriteOptions, write_document};

fn names(dictionary: &Dictionary) -> Vec<&str> {
    dictionary
        .keys()
        .map(|key| std::str::from_utf8(key).unwrap())
        .collect()
}

fn parse(input: &[u8]) -> Object {
    parse_complete(input, ParseLimits::default()).unwrap()
}

fn print(value: &Object, style: SerializeStyle) -> String {
    let limits = WriteLimits {
        max_output_bytes: 64 << 20,
        max_depth: 64,
        max_objects: 10_000_000,
    };
    String::from_utf8(serialize_object_with_style(value, limits, style).unwrap()).unwrap()
}

/// A classic-xref PDF of the given objects.
fn pdf(objects: &[(u32, &[u8])]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (number, body) in objects {
        offsets.push(out.len());
        out.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[test]
fn entries_keep_insertion_order() {
    let mut d = Dictionary::new();
    assert!(d.is_empty());
    for (key, value) in [("Z", 1), ("A", 2), ("M", 3)] {
        assert_eq!(d.insert(key.into(), Object::Integer(value)), None);
    }
    assert_eq!(names(&d), ["Z", "A", "M"]);
    // Replacing keeps the position.
    assert_eq!(
        d.insert(b"A".to_vec(), Object::Null),
        Some(Object::Integer(2))
    );
    assert_eq!(names(&d), ["Z", "A", "M"]);
    assert_eq!(d.get(b"A".as_slice()), Some(&Object::Null));
    // Removing keeps the others in order; a key added again goes last.
    assert_eq!(d.remove(b"Z".as_slice()), Some(Object::Integer(1)));
    assert_eq!(d.remove(b"Z".as_slice()), None);
    assert_eq!(names(&d), ["A", "M"]);
    d.insert(b"Z".to_vec(), Object::Integer(4));
    assert_eq!(names(&d), ["A", "M", "Z"]);
    assert_eq!(d.len(), 3);
    assert!(d.contains_key(&b"M".to_vec()));
    assert_eq!(d[b"M".as_slice()], Object::Integer(3));
    // Indexing an absent key reads null instead of panicking.
    assert_eq!(d[b"Q".as_slice()], Object::Null);

    let values: Vec<_> = d.values().cloned().collect();
    assert_eq!(
        values,
        [Object::Null, Object::Integer(3), Object::Integer(4)]
    );
    let reversed: Vec<_> = d.iter().rev().map(|(k, _)| k.clone()).collect();
    assert_eq!(reversed, [b"Z".to_vec(), b"M".to_vec(), b"A".to_vec()]);
    assert_eq!(d.iter().len(), 3);
    for (_, value) in &mut d {
        *value = Object::Boolean(true);
    }
    for value in d.values_mut() {
        assert_eq!(*value, Object::Boolean(true));
        *value = Object::Integer(0);
    }
    for (key, value) in d.iter_mut() {
        *value = Object::Name(key.clone());
    }
    let borrowed: Vec<_> = (&d)
        .into_iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let owned: Vec<_> = d.clone().into_iter().collect();
    assert_eq!(borrowed, owned);
    assert_eq!(
        d.clone().into_keys().collect::<Vec<_>>(),
        [b"A", b"M", b"Z"]
    );
    assert_eq!(
        d.clone().into_values().collect::<Vec<_>>(),
        ["A", "M", "Z"].map(Object::name)
    );
    assert_eq!(
        format!("{d:?}"),
        "{[65]: Name([65]), [77]: Name([77]), [90]: Name([90])}"
    );

    // Construction keeps the order given; a repeated key keeps its first
    // position with its last value.
    let built = Object::dictionary([("B", Object::Null), ("A", Object::Null)]);
    assert_eq!(names(built.as_dict().unwrap()), ["B", "A"]);
    let collected: Dictionary = [("Y", 1), ("X", 2), ("Y", 3)]
        .into_iter()
        .map(|(k, v)| (k.as_bytes().to_vec(), Object::Integer(v)))
        .collect();
    assert_eq!(names(&collected), ["Y", "X"]);
    assert_eq!(collected[b"Y".as_slice()], Object::Integer(3));
    let from_map: Dictionary =
        BTreeMap::from([(b"B".to_vec(), Object::Null), (b"A".to_vec(), Object::Null)]).into();
    assert_eq!(names(&from_map), ["A", "B"]);
    let from_array =
        Dictionary::from([(b"B".to_vec(), Object::Null), (b"A".to_vec(), Object::Null)]);
    assert_eq!(names(&from_array), ["B", "A"]);
    let mut extended = from_array.clone();
    extended.extend([
        (b"C".to_vec(), Object::Null),
        (b"B".to_vec(), Object::Integer(1)),
    ]);
    assert_eq!(names(&extended), ["B", "A", "C"]);
    assert_eq!(
        from_array
            .sorted_entries()
            .iter()
            .map(|(k, _)| k.as_slice())
            .collect::<Vec<_>>(),
        [b"A", b"B"]
    );
    extended.clear();
    assert!(extended.is_empty());
    // Dictionaries do not make objects larger than a `BTreeMap` did.
    #[cfg(target_pointer_width = "64")]
    assert_eq!(std::mem::size_of::<Object>(), 32);
}

#[test]
fn equality_ignores_order() {
    let a = Object::dictionary([
        ("A", Object::Integer(1)),
        (
            "B",
            Object::dictionary([("X", Object::Null), ("Y", Object::Null)]),
        ),
    ]);
    let b = Object::dictionary([
        (
            "B",
            Object::dictionary([("Y", Object::Null), ("X", Object::Null)]),
        ),
        ("A", Object::Integer(1)),
    ]);
    assert_eq!(a, b);
    let c = Object::dictionary([("B", Object::Null), ("A", Object::Integer(1))]);
    assert_ne!(a, c);
    let d = Object::dictionary([("A", Object::Integer(1))]);
    assert_ne!(a, d);
    assert_ne!(d, a);
    assert_eq!(Dictionary::new(), Dictionary::default());
}

/// Random operations agree with a list kept as MuPDF keeps a dictionary:
/// insertion order until a put finds more than 100 entries, key order after.
#[test]
fn random_operations_match_a_list_model() {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    for pool in [48, 130, 400] {
        let mut d = Dictionary::new();
        let mut model: Vec<(Vec<u8>, i64)> = Vec::new();
        let mut sorted = false;
        for step in 0..30_000i64 {
            let key = format!("K{}", next(pool)).into_bytes();
            match next(100) {
                0..60 => {
                    if !sorted && model.len() > 100 {
                        sorted = true;
                        model.sort_by(|a, b| a.0.cmp(&b.0));
                    }
                    let at = model.iter().position(|(k, _)| *k == key);
                    let old = d.insert(key.clone(), Object::Integer(step));
                    assert_eq!(old, at.map(|i| Object::Integer(model[i].1)));
                    match at {
                        Some(i) => model[i].1 = step,
                        None if sorted => {
                            let i = model.partition_point(|(k, _)| *k < key);
                            model.insert(i, (key, step));
                        }
                        None => model.push((key, step)),
                    }
                }
                60..80 => {
                    let at = model.iter().position(|(k, _)| *k == key);
                    let old = d.remove(key.as_slice());
                    assert_eq!(old, at.map(|i| Object::Integer(model.remove(i).1)));
                }
                80 => {
                    let bound = (next(7) + 3) as i64;
                    d.retain(|_, v| v.as_i64().unwrap() % bound != 0);
                    model.retain(|(_, v)| v % bound != 0);
                }
                _ => {
                    let at = model.iter().position(|(k, _)| *k == key);
                    if let Some(value) = d.get_mut(key.as_slice()) {
                        *value = Object::Integer(-step);
                    }
                    if let Some(i) = at {
                        model[i].1 = -step;
                    }
                }
            }
            assert_eq!(d.len(), model.len());
            let probe = format!("K{}", next(pool)).into_bytes();
            assert_eq!(
                d.get(probe.as_slice()),
                model
                    .iter()
                    .find(|(k, _)| *k == probe)
                    .map(|(_, v)| Object::Integer(*v))
                    .as_ref()
            );
            if step % 97 == 0 {
                let actual: Vec<_> = d
                    .iter()
                    .map(|(k, v)| (k.clone(), v.as_i64().unwrap()))
                    .collect();
                assert_eq!(actual, model, "pool {pool} step {step}");
            }
        }
        // The smallest pool cannot pass 100 entries; the others do.
        assert_eq!(sorted, pool != 48, "pool {pool}");
    }
}

/// MuPDF 1.28.1 (saveToBuffer of a parsed file, and puts through its API)
/// sorts a dictionary at the first put that finds more than 100 entries.
#[test]
fn large_dictionaries_sort_as_mupdf_sorts_them() {
    let descending = |n: usize, extra: &str| {
        let mut text = String::from("<<");
        for i in (0..n).rev() {
            text.push_str(&format!("/K{i} {i}"));
        }
        text.push_str(extra);
        text.push_str(">>");
        Parser::new(text.as_bytes(), ParseLimits::default())
            .with_duplicate_key_policy(DuplicateKeyPolicy::KeepLast)
            .parse_complete()
            .unwrap()
    };
    let head = |value: &Object| print(value, SerializeStyle::Compact)[..40].to_string();
    for (n, extra, expected) in [
        (100, "", "<</K99 99/K98 98/K97 97/K96 96/K95 95/K9"),
        (100, "/K3 -3", "<</K99 99/K98 98/K97 97/K96 96/K95 95/K9"),
        (101, "", "<</K100 100/K99 99/K98 98/K97 97/K96 96/"),
        (102, "", "<</K0 0/K1 1/K10 10/K100 100/K101 101/K1"),
        (101, "/K3 -3", "<</K0 0/K1 1/K10 10/K100 100/K11 11/K12 "),
    ] {
        assert_eq!(head(&descending(n, extra)), expected, "{n} {extra}");
    }
    // Lookups never sort; a new key in a 100-entry dictionary goes last, and
    // the next put sorts.
    let mut value = descending(100, "");
    let d = value.as_dict_mut().unwrap();
    assert!(d.get(b"K5".as_slice()).is_some() && d.get(b"Nope".as_slice()).is_none());
    d.insert(b"Z1".to_vec(), Object::Integer(1));
    assert_eq!(names(d)[100], "Z1");
    assert_eq!(names(d)[0], "K99");
    d.insert(b"A0".to_vec(), Object::Integer(1));
    assert_eq!(head(&value), "<</A0 1/K0 0/K1 1/K10 10/K11 11/K12 12/K");
    // A sorted dictionary stays sorted when it shrinks.
    let d = value.as_dict_mut().unwrap();
    d.retain(|key, _| key.len() < 3);
    d.insert(b"B".to_vec(), Object::Null);
    assert_eq!(
        names(d),
        [
            "A0", "B", "K0", "K1", "K2", "K3", "K4", "K5", "K6", "K7", "K8", "K9", "Z1"
        ]
    );
}

/// MuPDF's `pdf_dict_put`: a repeated key keeps its first position with its
/// last value. MuPDF 1.28.1 saves <</B 1/A 2/B 3/C 4/A 5>> as <</B 3/A 5/C 4>>.
#[test]
fn keep_last_duplicates_keep_the_first_position() {
    let value = Parser::new(b"<</B 1/A 2/B 3/C 4/A 5>>", ParseLimits::default())
        .with_duplicate_key_policy(DuplicateKeyPolicy::KeepLast)
        .parse_complete()
        .unwrap();
    assert_eq!(print(&value, SerializeStyle::Compact), "<</B 3/A 5/C 4>>");
    assert!(parse_complete(b"<</B 1/A 2/B 3>>", ParseLimits::default()).is_err());
}

#[test]
fn canonical_sorts_and_compact_keeps_source_order() {
    let value = parse(b"<< /Z 1 /Type /T /A << /Y 2 /B 3 >> >>");
    assert_eq!(
        print(&value, SerializeStyle::Canonical),
        "<< /A << /B 3 /Y 2 >> /Type /T /Z 1 >>"
    );
    assert_eq!(
        print(&value, SerializeStyle::Compact),
        "<</Type/T/Z 1/A<</Y 2/B 3>>>>"
    );
    // MuPDF 1.28.1 saveToBuffer('incremental=no,garbage=yes') output: a
    // non-null /Type moves first, then a non-null /Subtype.
    for (input, expected) in [
        (
            "<</Pages 2 0 R/Type/Catalog/Z 3 0 R>>",
            "<</Type/Catalog/Pages 2 0 R/Z 3 0 R>>",
        ),
        (
            "<</Kids[]/Count 0/Type/Pages>>",
            "<</Type/Pages/Kids[]/Count 0>>",
        ),
        (
            "<</Subtype/S/Type null/A<</Type/T>>>>",
            "<</Subtype/S/Type null/A<</Type/T>>>>",
        ),
        ("<</Z 1/Subtype 1/A 1>>", "<</Z 1/Subtype 1/A 1>>"),
        (
            "<</Subtype/W/X 1/Type/Annot>>",
            "<</Type/Annot/Subtype/W/X 1>>",
        ),
        ("<</X 1/Type/T/Subtype/S>>", "<</Type/T/Subtype/S/X 1>>"),
        ("<</A 1/B null/C 2>>", "<</A 1/B null/C 2>>"),
        (
            "<</Type/Sig/Filter/F/SubFilter/s>>",
            "<</Type/Sig/Filter/F/SubFilter/s>>",
        ),
        (
            "<</FT/Sig/T(Signature1)/F 4/Rect[0 0 0 0]/Subtype/Widget/Type/Annot>>",
            "<</Type/Annot/Subtype/Widget/FT/Sig/T(Signature1)/F 4/Rect[0 0 0 0]>>",
        ),
        (
            "<</X 1/Subtype/S/Type null>>",
            "<</X 1/Subtype/S/Type null>>",
        ),
        ("<</X 1/Type 5/Subtype/S>>", "<</Type 5/Subtype/S/X 1>>"),
        (
            "<</X 1/Type(str)/Subtype/S>>",
            "<</Type(str)/Subtype/S/X 1>>",
        ),
        (
            "<</X 1/Subtype null/Type/T>>",
            "<</Type/T/X 1/Subtype null>>",
        ),
        ("<</X 1/Subtype 3/Type/T>>", "<</Type/T/Subtype 3/X 1>>"),
        ("<</X 1/Type[/A]/Subtype/S>>", "<</Type[/A]/Subtype/S/X 1>>"),
        (
            "<</X 1/Subtype 5 0 R/Type/T>>",
            "<</Type/T/Subtype 5 0 R/X 1>>",
        ),
    ] {
        let value = parse(input.as_bytes());
        assert_eq!(print(&value, SerializeStyle::Compact), expected, "{input}");
    }
}

/// MuPDF drops null objects when it collects garbage and prints references
/// to them as null, so a /Type that references one is null and stays put.
#[test]
fn app_save_keeps_source_order_and_drops_null_objects() {
    let bytes = pdf(&[
        (1, b"<</Pages 2 0 R/Type/Catalog/E[3 0 R 4 0 R 5 0 R]>>"),
        (2, b"<</Kids[]/Count 0/Type/Pages>>"),
        (3, b"<</X 1/Type 5 0 R/Subtype/S>>"),
        (4, b"<</X 1/Type 6 0 R/Subtype/S>>"),
        (5, b"null"),
        (6, b"/Name"),
    ]);
    let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let written = write_document(&doc, DocumentWriteLimits::default(), WriteOptions::APP).unwrap();
    let text = String::from_utf8_lossy(&written);
    for expected in [
        "1 0 obj\n<</Type/Catalog/Pages 2 0 R/E[3 0 R 4 0 R null]>>\nendobj\n",
        "2 0 obj\n<</Type/Pages/Kids[]/Count 0>>\nendobj\n",
        "3 0 obj\n<</X 1/Type null/Subtype/S>>\nendobj\n",
        "4 0 obj\n<</Type 6 0 R/Subtype/S/X 1>>\nendobj\n",
        "6 0 obj\n/Name\nendobj\n",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
    assert!(!text.contains("5 0 obj"), "{text}");
}

/// A stream keeps /Length where it is, or gets it last, in both the app save
/// and the repair output; repair keeps every dictionary's key order.
#[test]
fn stream_length_and_repair_keep_positions() {
    let bytes = pdf(&[
        (1, b"<</Pages 2 0 R/Type/Catalog>>"),
        (2, b"<</Kids[3 0 R]/Count 1/Type/Pages>>"),
        (
            3,
            b"<</Parent 2 0 R/Type/Page/MediaBox[0 0 10 10]/Contents 4 0 R/Z 5 0 R>>",
        ),
        (4, b"<</B 1/Length 2/A 2>>\nstream\nq\n\nendstream"),
        (5, b"<</Length 2/Filter/AHx>>\nstream\n41\nendstream"),
    ]);
    let mut doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
    let mut owned = Dictionary::new();
    owned.insert(b"Z".to_vec(), Object::Integer(1));
    owned.insert(b"A".to_vec(), Object::Integer(2));
    let new = doc
        .allocate(Object::Dictionary(owned), Some(b"xyz".to_vec()))
        .unwrap();
    let page = doc.object(id(3)).unwrap().value.clone();
    let mut page = page.as_dict().unwrap().clone();
    page.insert(b"New".to_vec(), Object::Reference(new));
    doc.replace_value(id(3), Object::Dictionary(page)).unwrap();
    let written = write_document(&doc, DocumentWriteLimits::default(), WriteOptions::APP).unwrap();
    let text = String::from_utf8_lossy(&written);
    for expected in [
        "3 0 obj\n<</Type/Page/Parent 2 0 R/MediaBox[0 0 10 10]/Contents 4 0 R/Z 5 0 R/New 6 0 R>>",
        "4 0 obj\n<</B 1/Length 2/A 2>>\nstream\n",
        "5 0 obj\n<</Length 2/Filter/AHx>>\nstream\n",
        "6 0 obj\n<</Z 1/A 2/Length 3>>\nstream\nxyz\nendstream",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }

    // Damage the xref; the rebuilt file keeps each dictionary's order, and
    // the stream's /Length stays in place with its recovered value.
    let mut damaged = bytes.clone();
    let at = damaged.windows(4).position(|w| w == b"xref").unwrap();
    damaged.truncate(at);
    damaged.extend_from_slice(b"trailer\n<< /Root 1 0 R >>\n%%EOF\n");
    let repaired = rebuild(&damaged, RepairLimits::default()).unwrap();
    let doc = Document::open(&repaired.bytes, DocumentLimits::default()).unwrap();
    let page = doc.object(id(3)).unwrap().value.as_dict().unwrap().clone();
    assert_eq!(
        names(&page),
        ["Parent", "Type", "MediaBox", "Contents", "Z"]
    );
    let content = doc.object(id(4)).unwrap().value.as_dict().unwrap().clone();
    assert_eq!(names(&content), ["B", "Length", "A"]);
    let written = write_document(&doc, DocumentWriteLimits::default(), WriteOptions::APP).unwrap();
    let text = String::from_utf8_lossy(&written);
    assert!(
        text.contains("4 0 obj\n<</B 1/Length 2/A 2>>\nstream\n"),
        "{text}"
    );
}

fn id(n: u32) -> tandem_pdf_engine::object::ObjectId {
    tandem_pdf_engine::object::ObjectId::new(n, 0).unwrap()
}

/// Building, parsing, comparing and printing a dictionary with hundreds of
/// thousands of keys is O(n log n), not quadratic.
#[test]
fn huge_dictionaries_stay_fast() {
    const N: usize = 200_000;
    let start = Instant::now();
    let mut text = b"<<".to_vec();
    for i in 0..N {
        text.extend_from_slice(format!("/K{i} {i}").as_bytes());
    }
    // Every key again, newest values, for the KeepLast policy.
    let mut repeated = text.clone();
    for i in (0..N).rev() {
        repeated.extend_from_slice(format!("/K{i} -{i}").as_bytes());
    }
    text.extend_from_slice(b">>");
    repeated.extend_from_slice(b">>");
    let limits = ParseLimits {
        max_objects: 10 * N,
        max_decoded_bytes: 64 << 20,
        ..ParseLimits::default()
    };
    let unique = parse_complete(&text, limits).unwrap();
    let unique = unique.as_dict().unwrap();
    assert_eq!(unique.len(), N);
    let last = Parser::new(&repeated, limits)
        .with_duplicate_key_policy(DuplicateKeyPolicy::KeepLast)
        .parse_complete()
        .unwrap();
    let last = last.as_dict().unwrap();
    assert_eq!(last.len(), N);
    // Past 100 entries a dictionary is in key order, as MuPDF sorts it.
    let mut previous = None;
    for (key, value) in last {
        assert!(previous < Some(key));
        previous = Some(key);
        let i: i64 = std::str::from_utf8(&key[1..]).unwrap().parse().unwrap();
        assert_eq!(value.as_i64(), Some(-i));
        assert_eq!(unique.get(key.as_slice()), Some(&Object::Integer(i)));
    }
    let error = parse_complete(&repeated, limits).unwrap_err();
    assert!(
        error.to_string().contains("duplicate dictionary key"),
        "{error}"
    );

    // Built in reverse, it is equal and prints in key order.
    let mut reversed = Dictionary::new();
    for i in (0..N).rev() {
        reversed.insert(format!("K{i}").into_bytes(), Object::Integer(i as i64));
    }
    assert_eq!(&reversed, unique);
    let compact = print(
        &Object::Dictionary(reversed.clone()),
        SerializeStyle::Compact,
    );
    assert!(compact.starts_with("<</K0 0/K1 1/K10 10/"));
    let canonical = print(
        &Object::Dictionary(reversed.clone()),
        SerializeStyle::Canonical,
    );
    assert!(canonical.starts_with("<< /K0 0 /K1 1 /K10 10 "));
    // Removal and retain work at this size too.
    for i in (0..N).step_by(N / 50) {
        assert_eq!(
            reversed.remove(format!("K{i}").as_bytes()),
            Some(Object::Integer(i as i64))
        );
    }
    reversed.retain(|key, _| key.ends_with(b"7"));
    assert_eq!(reversed.len(), N / 10);
    assert_eq!(reversed.get(b"K17".as_slice()), Some(&Object::Integer(17)));
    assert_eq!(reversed.get(b"K18".as_slice()), None);
    let elapsed = start.elapsed();
    // Quadratic work at this size takes minutes even in a release build.
    assert!(elapsed < Duration::from_secs(60), "{elapsed:?}");
}
