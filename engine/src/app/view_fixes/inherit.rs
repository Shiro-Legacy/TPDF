// SPDX-License-Identifier: AGPL-3.0-or-later
//! Names a form uses that its own /Resources lack. MuPDF looks a resource
//! name up through the resources of every enclosing content, the form's
//! own first, then its caller's, out to the page's; hayro looks only in the
//! form's own. When a form's content names a resource its own lack, the
//! view adds the entry the enclosing resources give it.
use std::collections::BTreeSet;

use crate::content::{ContentLimits, ContentOperand, ContentParser};
use crate::lexer::LexLimits;
use crate::object::{Dictionary, Object, ObjectId};

use super::Fixes;

const CATEGORIES: [&[u8]; 7] = [
    b"XObject",
    b"ExtGState",
    b"Font",
    b"ColorSpace",
    b"Pattern",
    b"Shading",
    b"Properties",
];
/// Form content read per view to find the names it uses.
const MAX_SCANNED: usize = 1 << 20;
/// Enclosing names looked up per view to find a form's missing ones.
const MAX_CHECKS: usize = 1 << 22;

impl Fixes<'_, '_> {
    /// A resources category dictionary as the view has it.
    fn category<'s>(&'s self, resources: &'s Dictionary, key: &[u8]) -> Option<&'s Dictionary> {
        resources
            .get(key)
            .and_then(|c| self.view_resolve(c).as_dict())
    }

    /// Adds to `own`, the resources of form `id`, each entry its content
    /// uses that only the enclosing resources (`recolor.chain`) have.
    /// Whether it added one.
    pub(super) fn inherit_names(&mut self, id: ObjectId, own: &mut Dictionary) -> bool {
        let chain = std::mem::take(&mut self.recolor.chain);
        let added = self.inherit_from(id, own, &chain);
        self.recolor.chain = chain;
        added
    }

    fn inherit_from(&mut self, id: ObjectId, own: &mut Dictionary, chain: &[Dictionary]) -> bool {
        // Whether an enclosing name is missing here, each name looked up
        // charged to the view's budget (None once it runs out).
        let mut checks = self.recolor.checked;
        let mut lacks = || {
            for outer in chain {
                for key in CATEGORIES {
                    let mine = self.category(own, key);
                    for name in self
                        .category(outer, key)
                        .into_iter()
                        .flat_map(Dictionary::keys)
                    {
                        checks += 1;
                        if checks > MAX_CHECKS {
                            return None;
                        }
                        if !mine.is_some_and(|m| m.contains_key(name.as_slice())) {
                            return Some(true);
                        }
                    }
                }
            }
            Some(false)
        };
        if chain.is_empty() || self.recolor.scanned > MAX_SCANNED {
            return false;
        }
        let lacking = lacks();
        self.recolor.checked = checks;
        if lacking != Some(true) {
            return false;
        }
        let Some(content) = self.decoded(id) else {
            return false;
        };
        self.recolor.scanned += content.len();
        let mut added = false;
        for (index, name) in used_names(&content) {
            let key = CATEGORIES[index];
            if self
                .category(own, key)
                .is_some_and(|m| m.contains_key(name.as_slice()))
            {
                continue;
            }
            let found = (chain.iter().rev())
                .find_map(|outer| self.category(outer, key)?.get(name.as_slice()).cloned());
            let Some(value) = found else {
                continue;
            };
            let mut entries = self.category(own, key).cloned().unwrap_or_default();
            entries.insert(name, value);
            own.insert(key.to_vec(), Object::Dictionary(entries));
            added = true;
        }
        added
    }
}

/// The resource names `content` uses, by category (index into
/// `CATEGORIES`), each once; what follows a syntax error is not read.
fn used_names(content: &[u8]) -> BTreeSet<(usize, Vec<u8>)> {
    let bound = content.len().saturating_add(16);
    let limits = ContentLimits {
        max_input_bytes: bound,
        max_operations: bound,
        max_nodes: bound,
        max_owned_bytes: bound,
        lex: LexLimits {
            max_tokens: bound,
            ..LexLimits::default()
        },
        ..ContentLimits::default()
    };
    let device = |name: &[u8]| {
        matches!(
            name,
            b"DeviceGray" | b"DeviceRGB" | b"DeviceCMYK" | b"Pattern" | b"G" | b"RGB" | b"CMYK"
        )
    };
    let mut names = BTreeSet::new();
    let mut parser = ContentParser::new(content, limits);
    while let Ok(Some(op)) = parser.next_operation() {
        let name = |at: usize| match op.operands.get(at) {
            Some(ContentOperand::Name(name)) => Some(name.clone()),
            _ => None,
        };
        let last = op.operands.len().saturating_sub(1);
        let used = match op.operator.as_slice() {
            b"Do" => name(0).map(|n| (0, n)),
            b"gs" => name(0).map(|n| (1, n)),
            b"Tf" => name(0).map(|n| (2, n)),
            b"cs" | b"CS" => name(0).filter(|n| !device(n)).map(|n| (3, n)),
            b"scn" | b"SCN" => name(last).map(|n| (4, n)),
            b"sh" => name(0).map(|n| (5, n)),
            b"BDC" | b"DP" => name(1).map(|n| (6, n)),
            b"BI" => match op.operands.first() {
                Some(ContentOperand::Dictionary(entries)) => [b"CS".as_slice(), b"ColorSpace"]
                    .iter()
                    .find_map(|k| match entries.get(*k) {
                        Some(ContentOperand::Name(n)) if !device(n) => Some((3, n.clone())),
                        _ => None,
                    }),
                _ => None,
            },
            _ => None,
        };
        names.extend(used);
    }
    names
}
