// SPDX-License-Identifier: AGPL-3.0-or-later
//! What hayro parses when it opens the view. It reads every page when the
//! document opens, and each page reads its /Resources and their seven
//! subdictionaries into maps of its own, so pages sharing one large
//! resources dictionary cost pages × entries in time and memory (about 90
//! bytes an entry: 1,000 pages sharing 4,000 fonts held 365 MB). The view is
//! refused before hayro opens it when that sum passes a budget.
use std::collections::HashMap;

use crate::document::Document;
use crate::object::{Dictionary, Object, ObjectId};

use super::{info, pagetree};

/// Dictionary entries hayro may read for the pages when the view opens
/// (about 380 MB of its maps).
pub(crate) const MAX_VIEW_ENTRIES: u64 = 1 << 22;

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

/// Whether hayro would read more than `limit` entries for the pages of
/// `doc` ([`MAX_VIEW_ENTRIES`] for the app's view).
pub(crate) fn over_budget(doc: &Document<'_>, limit: u64) -> bool {
    let mut shared: HashMap<ObjectId, u64> = HashMap::new();
    let mut total = 0u64;
    for page in info::pages(doc) {
        let resources = match pagetree::inherited(doc, page.dict, b"Resources") {
            Some(Object::Reference(id)) => *shared
                .entry(*id)
                .or_insert_with(|| doc.resolve_id(*id).as_dict().map_or(0, |d| entries(doc, d))),
            Some(value) => doc.resolve(value).as_dict().map_or(0, |d| entries(doc, d)),
            None => 0,
        };
        total = total
            .saturating_add(page.dict.len() as u64)
            .saturating_add(resources);
        if total > limit {
            return true;
        }
    }
    false
}

/// Entries of a resources dictionary and of the subdictionaries hayro reads.
fn entries(doc: &Document<'_>, resources: &Dictionary) -> u64 {
    let parsed: usize = PARSED
        .iter()
        .filter_map(|key| doc.dict_get(resources, key).as_dict())
        .map(Dictionary::len)
        .sum();
    (resources.len() + parsed) as u64
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
            assert!(over_budget(&doc, 6 * per_page - 1), "{on_node}");
            assert!(!over_budget(&doc, 6 * per_page), "{on_node}");
        }
    }

    #[test]
    fn only_the_subdictionaries_hayro_reads_are_counted() {
        // /Font and /XObject count their entries; /Other does not.
        let resources =
            "<< /Font << /A 4 0 R /B 4 0 R >> /XObject << /X 4 0 R >> /Other << /Y 1 /Z 2 >> >>";
        let bytes = pages_sharing(1, resources, false);
        let doc = Document::open(&bytes, DocumentLimits::default()).unwrap();
        assert!(over_budget(&doc, 4 + 3 + 3 - 1));
        assert!(!over_budget(&doc, 4 + 3 + 3));
    }
}
