// SPDX-License-Identifier: MIT
//! The XDP packets of /AcroForm /XFA: one stream holding the whole
//! `<xdp:xdp>` document, or an array of `(name) stream` pairs
//! (`preamble`, `config`, `template`, `localeSet`, `datasets`, `form`,
//! `postamble`, ...). Reads each packet's text (UTF-8, or UTF-16 with a
//! byte-order mark) within `limits::MAX_PACKET_BYTES`.
//!
//! A single stream is parsed and split into its child packets by local
//! name, each re-serialized without a declaration. In the array form every
//! pair is kept in `entries`, in order, and the first packet of each name
//! is read. A packet whose stream does not decode is treated as absent,
//! the template excepted (then `XfaError::Xml`). Bytes count together
//! against the limit before any text is built.

use crate::document::Document;
use crate::object::{Object, ObjectId};
use crate::stream::{StreamError, StreamErrorKind, StreamLimits};
use crate::stream_object::StreamObjectErrorKind;

use super::limits::MAX_PACKET_BYTES;
use super::{MARKER_KEY, XfaError, xml};

/// The packets a form needs, and every packet as stored.
#[derive(Clone, Debug, Default)]
pub struct Packets {
    /// The `template` packet's XML (`<template ...>`).
    pub template: String,
    /// The `datasets` packet's XML (`<xfa:datasets ...>`), when present.
    pub datasets: Option<String>,
    /// The `config` packet's XML, when present.
    pub config: Option<String>,
    /// The `localeSet` packet's XML, when present.
    pub locale_set: Option<String>,
    /// Every packet of an array-form /XFA as stored: its name and stream
    /// object. Empty for a single-stream /XFA (then `whole` holds it).
    pub entries: Vec<(Vec<u8>, ObjectId)>,
    /// A single-stream /XFA's stream object.
    pub whole: Option<ObjectId>,
}

/// The catalog's /AcroForm /XFA value, resolved, when present.
fn xfa_value<'d>(doc: &'d Document<'_>) -> Option<&'d Object> {
    let catalog = doc.resolve_id(doc.root()).as_dict()?;
    let acro = doc.dict_get(catalog, b"AcroForm").as_dict()?;
    let xfa = acro.get(b"XFA".as_slice())?;
    (!doc.resolve(xfa).is_null()).then_some(xfa)
}

/// Whether `doc` is a dynamic XFA form the materializer should lay out
/// (`xfa::is_dynamic`).
pub fn is_dynamic(doc: &Document<'_>) -> bool {
    let Some(catalog) = doc.resolve_id(doc.root()).as_dict() else {
        return false;
    };
    doc.dict_get(catalog, b"NeedsRendering").as_bool() == Some(true)
        && xfa_value(doc).is_some()
        && doc.dict_get(catalog, MARKER_KEY).is_null()
}

/// The packets of `doc`'s /AcroForm /XFA.
pub fn read(doc: &Document<'_>) -> Result<Packets, XfaError> {
    let xfa = xfa_value(doc).ok_or(XfaError::NoXfa)?;
    let mut packets = Packets::default();
    let mut budget = Budget { used: 0 };
    match doc.resolve(xfa) {
        Object::Array(items) => {
            let mut pairs = items.chunks_exact(2);
            for pair in pairs.by_ref() {
                let name = match doc.resolve(&pair[0]) {
                    Object::String(s) => s.clone(),
                    Object::Name(n) => n.clone(),
                    _ => continue,
                };
                let Some(id) = pair[1].as_reference() else {
                    continue;
                };
                packets.entries.push((name, id));
            }
            let entries = packets.entries.clone();
            for (name, id) in &entries {
                let wanted = match name.as_slice() {
                    b"template" => packets.template.is_empty(),
                    b"datasets" => packets.datasets.is_none(),
                    b"config" => packets.config.is_none(),
                    b"localeSet" => packets.locale_set.is_none(),
                    _ => false,
                };
                if !wanted {
                    continue;
                }
                let text = match budget.text(doc, *id) {
                    Ok(text) => text,
                    Err(Fail::Limit) => return Err(XfaError::Limit("XFA packet bytes")),
                    Err(Fail::Decode(e)) if name == b"template" => {
                        return Err(XfaError::Xml(format!("template stream: {e}")));
                    }
                    Err(Fail::Decode(_)) => continue,
                };
                match name.as_slice() {
                    b"template" => packets.template = text,
                    b"datasets" => packets.datasets = Some(text),
                    b"config" => packets.config = Some(text),
                    _ => packets.locale_set = Some(text),
                }
            }
        }
        _ => {
            let id = xfa.as_reference().ok_or(XfaError::NoXfa)?;
            packets.whole = Some(id);
            let text = match budget.text(doc, id) {
                Ok(text) => text,
                Err(Fail::Limit) => return Err(XfaError::Limit("XFA packet bytes")),
                Err(Fail::Decode(e)) => return Err(XfaError::Xml(format!("XFA stream: {e}"))),
            };
            let root = xml::parse(&text).map_err(|e| XfaError::Xml(format!("XDP: {e}")))?;
            for packet in root.elements() {
                let slot = match packet.local() {
                    "template" if packets.template.is_empty() => {
                        packets.template = xml::serialize_fragment(packet);
                        continue;
                    }
                    "datasets" => &mut packets.datasets,
                    "config" => &mut packets.config,
                    "localeSet" => &mut packets.locale_set,
                    _ => continue,
                };
                if slot.is_none() {
                    *slot = Some(xml::serialize_fragment(packet));
                }
            }
        }
    }
    if packets.template.trim().is_empty() {
        return Err(XfaError::NoXfa);
    }
    Ok(packets)
}

enum Fail {
    Limit,
    Decode(String),
}

/// The bytes of every packet read so far.
struct Budget {
    used: usize,
}

impl Budget {
    /// A packet stream's text, its bytes counted.
    fn text(&mut self, doc: &Document<'_>, id: ObjectId) -> Result<String, Fail> {
        let remaining = MAX_PACKET_BYTES.saturating_sub(self.used);
        if remaining == 0 {
            return Err(Fail::Limit);
        }
        let defaults = StreamLimits::default();
        let limits = StreamLimits {
            max_decoded_bytes: defaults.max_decoded_bytes.min(remaining),
            max_total_decoded_bytes: defaults.max_total_decoded_bytes.min(remaining),
            ..defaults
        };
        let bytes = match doc.decoded_stream(id, limits) {
            Ok(bytes) => bytes,
            // A stream cut short by the budget is the limit, not a fault.
            Err(e) if remaining < defaults.max_decoded_bytes && is_output_limit(&e.kind) => {
                return Err(Fail::Limit);
            }
            Err(e) => return Err(Fail::Decode(e.to_string())),
        };
        self.used = self.used.saturating_add(bytes.len());
        if self.used > MAX_PACKET_BYTES {
            return Err(Fail::Limit);
        }
        Ok(decode_text(&bytes))
    }
}

fn is_output_limit(kind: &StreamObjectErrorKind) -> bool {
    matches!(
        kind,
        StreamObjectErrorKind::Decode(StreamError {
            kind: StreamErrorKind::DecodedOutputLimit { .. }
                | StreamErrorKind::TotalOutputLimit { .. },
            ..
        })
    )
}

/// UTF-16 with a byte-order mark, else UTF-8 (a BOM stripped, malformed
/// sequences replaced).
pub fn decode_text(bytes: &[u8]) -> String {
    let utf16 = |rest: &[u8], big: bool| {
        let units = rest.chunks_exact(2).map(|p| {
            if big {
                u16::from_be_bytes([p[0], p[1]])
            } else {
                u16::from_le_bytes([p[0], p[1]])
            }
        });
        char::decode_utf16(units)
            .map(|c| c.unwrap_or('\u{fffd}'))
            .collect()
    };
    match bytes {
        [0xFE, 0xFF, rest @ ..] => utf16(rest, true),
        [0xFF, 0xFE, rest @ ..] => utf16(rest, false),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentLimits;

    /// A PDF of these object bodies (object `i + 1` is `bodies[i]`), the
    /// first the catalog. A body given as `Err(bytes)` is a stream.
    fn pdf(bodies: &[Result<&str, Vec<u8>>]) -> Vec<u8> {
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for body in bodies {
            offsets.push(bytes.len());
            let number = offsets.len();
            bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
            match body {
                Ok(text) => bytes.extend_from_slice(text.as_bytes()),
                Err(data) => {
                    bytes.extend_from_slice(
                        format!("<< /Length {} >>\nstream\n", data.len()).as_bytes(),
                    );
                    bytes.extend_from_slice(data);
                    bytes.extend_from_slice(b"\nendstream");
                }
            }
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
        );
        for offset in &offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                offsets.len() + 1
            )
            .as_bytes(),
        );
        bytes
    }

    const TEMPLATE: &str = "<template xmlns=\"http://www.xfa.org/schema/xfa-template/3.6/\"><subform name=\"form1\"/></template>";
    const DATASETS: &str = "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><xfa:data><form1/></xfa:data></xfa:datasets>";

    fn utf16(text: &str, big: bool) -> Vec<u8> {
        let mut out = if big {
            vec![0xFE, 0xFF]
        } else {
            vec![0xFF, 0xFE]
        };
        for unit in text.encode_utf16() {
            out.extend_from_slice(&if big {
                unit.to_be_bytes()
            } else {
                unit.to_le_bytes()
            });
        }
        out
    }

    #[test]
    fn reads_the_array_form_in_order() {
        let catalog = "<< /Type /Catalog /Pages 2 0 R /AcroForm 3 0 R /NeedsRendering true >>";
        let bytes = pdf(&[
            Ok(catalog),
            Ok("<< /Type /Pages /Kids [] /Count 0 >>"),
            Ok(
                "<< /Fields [] /XFA [(preamble) 4 0 R (config) 5 0 R (template) 6 0 R (datasets) 7 0 R (postamble) 8 0 R] >>",
            ),
            Err(b"<xdp:xdp xmlns:xdp=\"http://ns.adobe.com/xdp/\">".to_vec()),
            Err(b"\xEF\xBB\xBF<config/>".to_vec()),
            Err(utf16(TEMPLATE, false)),
            Err(utf16(DATASETS, true)),
            Err(b"</xdp:xdp>".to_vec()),
        ]);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert!(is_dynamic(&doc));
        let packets = read(&doc).unwrap();
        let names: Vec<&[u8]> = packets.entries.iter().map(|(n, _)| n.as_slice()).collect();
        assert_eq!(
            names,
            [
                &b"preamble"[..],
                b"config",
                b"template",
                b"datasets",
                b"postamble"
            ]
        );
        assert_eq!(packets.entries[2].1.number(), 6);
        assert_eq!(packets.template, TEMPLATE);
        assert_eq!(packets.datasets.as_deref(), Some(DATASETS));
        assert_eq!(packets.config.as_deref(), Some("<config/>"));
        assert_eq!(packets.locale_set, None);
        assert_eq!(packets.whole, None);
    }

    #[test]
    fn splits_a_single_stream() {
        let xdp = format!(
            "<?xml version=\"1.0\"?>\n<xdp:xdp xmlns:xdp=\"http://ns.adobe.com/xdp/\">\n<config><a/></config>\n{TEMPLATE}\n<localeSet><l/></localeSet>{DATASETS}</xdp:xdp>"
        );
        let bytes = pdf(&[
            Ok("<< /Type /Catalog /Pages 2 0 R /AcroForm << /XFA 3 0 R >> /NeedsRendering true >>"),
            Ok("<< /Type /Pages /Kids [] /Count 0 >>"),
            Err(xdp.into_bytes()),
        ]);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert!(is_dynamic(&doc));
        let packets = read(&doc).unwrap();
        assert!(packets.entries.is_empty());
        assert_eq!(packets.whole.map(|id| id.number()), Some(3));
        assert_eq!(packets.template, TEMPLATE);
        assert_eq!(packets.datasets.as_deref(), Some(DATASETS));
        assert_eq!(packets.config.as_deref(), Some("<config><a/></config>"));
        assert_eq!(
            packets.locale_set.as_deref(),
            Some("<localeSet><l/></localeSet>")
        );
    }

    #[test]
    fn dynamic_needs_the_flag_the_packets_and_no_marker() {
        let open = |catalog: &str| {
            let bytes = pdf(&[
                Ok(catalog),
                Ok("<< /Type /Pages /Kids [] /Count 0 >>"),
                Err(format!("<xdp:xdp>{TEMPLATE}</xdp:xdp>").into_bytes()),
            ]);
            let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
            (is_dynamic(&doc), read(&doc).is_ok())
        };
        let with = |extra: &str| {
            format!("<< /Type /Catalog /Pages 2 0 R /AcroForm << /XFA 3 0 R >> {extra} >>")
        };
        assert_eq!(open(&with("/NeedsRendering true")), (true, true));
        assert_eq!(open(&with("")), (false, true));
        assert_eq!(open(&with("/NeedsRendering false")), (false, true));
        assert_eq!(
            open(&with("/NeedsRendering true /TPDFXfa 1")),
            (false, true)
        );
        let none =
            "<< /Type /Catalog /Pages 2 0 R /AcroForm << /Fields [] >> /NeedsRendering true >>";
        assert_eq!(open(none), (false, false));
        assert_eq!(
            open("<< /Type /Catalog /Pages 2 0 R /NeedsRendering true >>"),
            (false, false)
        );
    }

    #[test]
    fn no_template_is_no_xfa() {
        let bytes = pdf(&[
            Ok("<< /Type /Catalog /Pages 2 0 R /AcroForm << /XFA [(datasets) 3 0 R (bad) 4] >> >>"),
            Ok("<< /Type /Pages /Kids [] /Count 0 >>"),
            Err(DATASETS.as_bytes().to_vec()),
        ]);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert_eq!(read(&doc).unwrap_err(), XfaError::NoXfa);
        let broken = pdf(&[
            Ok("<< /Type /Catalog /Pages 2 0 R /AcroForm << /XFA 3 0 R >> >>"),
            Ok("<< /Type /Pages /Kids [] /Count 0 >>"),
            Err(b"<xdp:xdp><template>".to_vec()),
        ]);
        let doc = Document::open(&broken, DocumentLimits::default()).unwrap();
        assert!(matches!(read(&doc), Err(XfaError::Xml(_))));
    }

    #[test]
    fn text_decodes_by_byte_order_mark() {
        assert_eq!(decode_text(&utf16("aé€", true)), "aé€");
        assert_eq!(decode_text(&utf16("aé€", false)), "aé€");
        assert_eq!(decode_text(b"\xEF\xBB\xBFx"), "x");
        assert_eq!(decode_text(b"x\xFFy"), "x\u{fffd}y");
        // An odd trailing byte of UTF-16 is dropped.
        assert_eq!(decode_text(&[0xFF, 0xFE, b'a', 0, b'b']), "a");
    }

    /// The owner's bench form, when it is on this machine (never in CI).
    #[test]
    fn bench_form_packets() {
        let path = "/Users/jiaqihou/Documents/tpdf-wt/scratch/xfa/Status_Certificate.pdf";
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert!(is_dynamic(&doc));
        let packets = read(&doc).unwrap();
        assert_eq!(packets.entries.len(), 7);
        assert!(
            packets.template.trim_start().starts_with("<template"),
            "{:?}",
            &packets.template[..80.min(packets.template.len())]
        );
        assert!(
            packets
                .datasets
                .as_deref()
                .is_some_and(|d| d.contains("<xfa:data"))
        );
        assert!(packets.config.is_some() && packets.locale_set.is_some());
    }
}
