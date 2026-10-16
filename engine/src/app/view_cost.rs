// SPDX-License-Identifier: AGPL-3.0-or-later
//! What hayro parses when it opens the view. It reads every page when the
//! document opens, and each page reads its /Resources and their seven
//! subdictionaries anew into maps of its own, so pages sharing one large
//! resources dictionary cost pages × its size: in memory, about 90 bytes an
//! entry (1,000 pages sharing 4,000 fonts held 365 MB), and in time, the
//! bytes it scans (1,000 pages sharing 2 MB of resources took 33 s). The
//! view is refused before hayro opens it when either sum passes a budget.
use std::collections::HashMap;

use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};
use crate::serialize::serialize_object;

use super::{info, overlay, pagetree};

/// What hayro may read for the pages when the view opens.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
    /// Dictionary entries it keeps in its maps.
    pub(crate) entries: u64,
    /// Bytes of resource dictionaries it scans.
    pub(crate) bytes: u64,
}

/// The app's view: 2^22 entries (about 380 MB of hayro's maps) and 2^27
/// bytes (128 MiB: 60 pages sharing 2 MB took 2.9 s natively).
pub(crate) const VIEW_LIMITS: Limits = Limits {
    entries: 1 << 22,
    bytes: 1 << 27,
};

/// The resource subdictionaries hayro reads for every page.
const PARSED: [&[u8]; 7] = [
    b"ExtGState",
    b"Font",
    b"ColorSpace",
    b"XObject",
    b"Pattern",
    b"Shading",
    b"Properties",
];

/// What one page's resources cost hayro.
#[derive(Clone, Copy, Default)]
struct Cost {
    entries: u64,
    bytes: u64,
}

/// Whether hayro would read more than `limits` for the pages of `doc`.
pub(crate) fn over_budget(doc: &Document<'_>, limits: Limits) -> bool {
    // Resources by object, or by address when written into a page or node.
    let mut shared: HashMap<ObjectId, Cost> = HashMap::new();
    let mut direct: HashMap<usize, Cost> = HashMap::new();
    let mut subdictionaries: HashMap<ObjectId, u64> = HashMap::new();
    let mut total = Cost::default();
    for page in info::pages(doc) {
        let resources = match pagetree::inherited(doc, page.dict, b"Resources") {
            Some(Object::Reference(id)) => *shared
                .entry(*id)
                .or_insert_with(|| cost(doc, doc.resolve_id(*id), &mut subdictionaries)),
            Some(value) => {
                let at = std::ptr::from_ref(value).addr();
                *direct
                    .entry(at)
                    .or_insert_with(|| cost(doc, value, &mut subdictionaries))
            }
            None => Cost::default(),
        };
        total.entries = total
            .entries
            .saturating_add(page.dict.len() as u64)
            .saturating_add(resources.entries);
        total.bytes = total.bytes.saturating_add(resources.bytes);
        if total.entries > limits.entries || total.bytes > limits.bytes {
            return true;
        }
    }
    false
}

/// The entries and bytes hayro reads for a resources dictionary: its own
/// and those of the subdictionaries it parses (scanned again when written
/// as objects of their own).
fn cost(
    doc: &Document<'_>,
    resources: &Object,
    subdictionaries: &mut HashMap<ObjectId, u64>,
) -> Cost {
    let Some(dict) = resources.as_dict() else {
        return Cost::default();
    };
    let mut entries = dict.len();
    let mut bytes = serialized(resources);
    for key in PARSED {
        let value = dict.get(key);
        entries += doc
            .resolve_dict(value.unwrap_or(&Object::Null))
            .map_or(0, Dictionary::len);
        if let Some(Object::Reference(id)) = value {
            let sub = *subdictionaries
                .entry(*id)
                .or_insert_with(|| serialized(doc.resolve_id(*id)));
            bytes = bytes.saturating_add(sub);
        }
    }
    Cost {
        entries: entries as u64,
        bytes,
    }
}

/// The bytes `value` takes in the view's rewrite (none when it cannot be
/// written, which the rewrite reports itself).
fn serialized(value: &Object) -> u64 {
    serialize_object(value, overlay::limits().syntax).map_or(0, |bytes| bytes.len() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentLimits;

    /// A classic-xref PDF of the object bodies numbered from 1.
    fn pdf(objects: &[String]) -> Vec<u8> {
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
        }
        let xref = bytes.len();
        let size = objects.len() + 1;
        bytes.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n")
                .as_bytes(),
        );
        bytes
    }

    /// `pages` pages under one node; `resources` (dictionary text) is object
    /// 3, `on_node` puts the reference on the node instead of each page.
    fn pages_sharing(pages: usize, resources: &str, on_node: bool) -> Vec<u8> {
        let kids: String = (0..pages).map(|i| format!("{} 0 R ", 5 + i)).collect();
        let (node, page) = if on_node {
            ("/Resources 3 0 R", "")
        } else {
            ("", "/Resources 3 0 R")
        };
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            format!("<< /Type /Pages /Kids [{kids}] /Count {pages} {node} >>"),
            resources.to_string(),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        for _ in 0..pages {
            objects.push(format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] {page} >>"
            ));
        }
        pdf(&objects)
    }

    #[test]
    fn pages_sharing_resources_count_their_entries_once_per_page() {
        // Each page: 3 (or 4) entries of its own, and the resources' 2
        // entries plus 5 fonts.
        let fonts: String = (0..5).map(|i| format!("/F{i} 4 0 R ")).collect();
        let resources = format!("<< /Font << {fonts}>> /ProcSet [/PDF] >>");
        for (on_node, per_page) in [(false, 4 + 7), (true, 3 + 7)] {
            let bytes = pages_sharing(6, &resources, on_node);
            let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
            assert!(over_budget(&doc, entries(6 * per_page - 1)), "{on_node}");
            assert!(!over_budget(&doc, entries(6 * per_page)), "{on_node}");
        }
    }

    #[test]
    fn only_the_subdictionaries_hayro_reads_are_counted() {
        // /Font and /XObject count their entries; /Other does not.
        let resources =
            "<< /Font << /A 4 0 R /B 4 0 R >> /XObject << /X 4 0 R >> /Other << /Y 1 /Z 2 >> >>";
        let bytes = pages_sharing(1, resources, false);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert!(over_budget(&doc, entries(4 + 3 + 3 - 1)));
        assert!(!over_budget(&doc, entries(4 + 3 + 3)));
    }

    /// An entry limit of `n` with no byte limit.
    fn entries(n: u64) -> Limits {
        Limits {
            entries: n,
            bytes: u64::MAX,
        }
    }

    /// A byte limit of `n` with no entry limit.
    fn bytes(n: u64) -> Limits {
        Limits {
            entries: u64::MAX,
            bytes: n,
        }
    }

    #[test]
    fn pages_sharing_resources_count_their_bytes_once_per_page() {
        // Two fonts given as direct arrays of 1,000 numbers each: few
        // entries, but hayro scans the arrays again for every page.
        let array = format!("[{}]", "0 ".repeat(1000));
        let direct = format!("<< /Font << /A {array} /B {array} >> >>");
        let bytes_of = |doc: &Document<'_>, number: u32| {
            serialized(doc.resolve_id(ObjectId::new(number, 0).unwrap()))
        };
        let pdf = pages_sharing(10, &direct, true);
        let doc = Document::open(&pdf, DocumentLimits::default()).unwrap();
        let per_page = bytes_of(&doc, 3);
        assert!(per_page > 4000, "{per_page}");
        assert!(over_budget(&doc, bytes(10 * per_page - 1)));
        assert!(!over_budget(&doc, bytes(10 * per_page)));
        assert!(!over_budget(&doc, entries(1000)));
        // A /Font dictionary written as an object of its own is scanned too.
        let pdf = pages_sharing(10, "<< /Font 4 0 R >>", false);
        let doc = Document::open(&pdf, DocumentLimits::default()).unwrap();
        let per_page = bytes_of(&doc, 3) + bytes_of(&doc, 4);
        assert!(over_budget(&doc, bytes(10 * per_page - 1)));
        assert!(!over_budget(&doc, bytes(10 * per_page)));
    }
}
