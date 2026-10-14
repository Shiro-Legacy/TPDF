// SPDX-License-Identifier: AGPL-3.0-or-later
//! Strict page-tree inventory. No rendering or content interpretation is performed.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::document::{Document, DocumentError};
use crate::object::{Object, ObjectId};

type Dictionary = BTreeMap<Vec<u8>, Object>;

#[derive(Clone, Copy, Debug)]
pub struct PageLimits {
    pub max_pages: usize,
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_total_content_refs: usize,
}
impl Default for PageLimits {
    fn default() -> Self {
        Self {
            max_pages: 10_000,
            max_nodes: 20_000,
            max_depth: 64,
            max_total_content_refs: 100_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageInfo {
    pub id: ObjectId,
    pub media_box: [f64; 4],
    pub crop_box: [f64; 4],
    pub rotation: u16,
    pub user_unit: f64,
    pub contents: Vec<ObjectId>,
}
impl PageInfo {
    /// Bounds in the top-left display coordinate system, after cropping,
    /// UserUnit scaling and page rotation. This is geometry, not a render result.
    pub fn display_bounds(&self) -> [f64; 4] {
        let width = (self.crop_box[2] - self.crop_box[0]) * self.user_unit;
        let height = (self.crop_box[3] - self.crop_box[1]) * self.user_unit;
        if self.rotation == 90 || self.rotation == 270 {
            [0.0, 0.0, height, width]
        } else {
            [0.0, 0.0, width, height]
        }
    }
}

#[derive(Debug)]
pub enum PageError {
    Document(DocumentError),
    Invalid(&'static str),
    Limit(&'static str),
    CycleOrSharedNode(ObjectId),
}
impl fmt::Display for PageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Document(error) => write!(f, "{error}"),
            Self::Invalid(reason) => write!(f, "invalid PDF page tree: {reason}"),
            Self::Limit(reason) => write!(f, "PDF page tree limit: {reason}"),
            Self::CycleOrSharedNode(id) => write!(
                f,
                "PDF page tree cycle/shared node {} {}",
                id.number(),
                id.generation()
            ),
        }
    }
}
impl std::error::Error for PageError {}
impl From<DocumentError> for PageError {
    fn from(error: DocumentError) -> Self {
        Self::Document(error)
    }
}

#[derive(Clone, Copy, Default)]
struct Inherited {
    media: Option<[f64; 4]>,
    crop: Option<[f64; 4]>,
    rotation: u16,
}

enum Visit {
    Enter {
        id: ObjectId,
        parent: Option<ObjectId>,
        inherited: Inherited,
        depth: usize,
    },
    Exit {
        start: usize,
        expected: usize,
    },
}

fn resolve<'d>(doc: &'d Document<'_>, mut value: &'d Object) -> Result<&'d Object, PageError> {
    let mut seen = BTreeSet::new();
    while let Object::Reference(id) = value {
        if !seen.insert(*id) {
            return Err(PageError::CycleOrSharedNode(*id));
        }
        if seen.len() > 64 {
            return Err(PageError::Limit("indirect value chain"));
        }
        value = &doc.object(*id)?.value;
    }
    Ok(value)
}

fn dictionary<'d>(doc: &'d Document<'_>, id: ObjectId) -> Result<&'d Dictionary, PageError> {
    let object = doc.object(id)?;
    if object.stream.is_some() {
        return Err(PageError::Invalid("page-tree node cannot be a stream"));
    }
    match &object.value {
        Object::Dictionary(dict) => Ok(dict),
        _ => Err(PageError::Invalid("expected a dictionary object")),
    }
}

fn optional<'d>(
    doc: &'d Document<'_>,
    dict: &'d Dictionary,
    key: &[u8],
) -> Result<Option<&'d Object>, PageError> {
    match dict.get(key) {
        Some(value) => match resolve(doc, value)? {
            Object::Null => Ok(None),
            value => Ok(Some(value)),
        },
        None => Ok(None),
    }
}

fn reference(value: Option<&Object>, reason: &'static str) -> Result<ObjectId, PageError> {
    match value {
        Some(Object::Reference(id)) => Ok(*id),
        _ => Err(PageError::Invalid(reason)),
    }
}

fn number(doc: &Document<'_>, value: &Object) -> Result<f64, PageError> {
    let result = match resolve(doc, value)? {
        Object::Integer(value) => *value as f64,
        Object::Real(value) => *value,
        _ => return Err(PageError::Invalid("expected numeric page geometry")),
    };
    if !result.is_finite() {
        return Err(PageError::Invalid("nonfinite page geometry"));
    }
    Ok(result)
}

fn rectangle(doc: &Document<'_>, value: &Object) -> Result<[f64; 4], PageError> {
    let Object::Array(items) = resolve(doc, value)? else {
        return Err(PageError::Invalid("page box must be an array"));
    };
    if items.len() != 4 {
        return Err(PageError::Invalid("page box must contain four numbers"));
    }
    let rect = [
        number(doc, &items[0])?,
        number(doc, &items[1])?,
        number(doc, &items[2])?,
        number(doc, &items[3])?,
    ];
    let rect = [
        rect[0].min(rect[2]),
        rect[1].min(rect[3]),
        rect[0].max(rect[2]),
        rect[1].max(rect[3]),
    ];
    if rect[2] <= rect[0] || rect[3] <= rect[1] {
        return Err(PageError::Invalid("page box must have positive dimensions"));
    }
    Ok(rect)
}

fn contents(
    doc: &Document<'_>,
    dict: &Dictionary,
    remaining: usize,
) -> Result<Vec<ObjectId>, PageError> {
    let Some(value) = dict.get(b"Contents".as_slice()) else {
        return Ok(Vec::new());
    };
    let refs: Vec<ObjectId> = match value {
        Object::Null => Vec::new(),
        Object::Reference(id) if doc.object(*id)?.stream.is_some() => {
            if remaining == 0 {
                return Err(PageError::Limit("total content references"));
            }
            vec![*id]
        }
        other => match resolve(doc, other)? {
            Object::Null => Vec::new(),
            Object::Array(items) => {
                if items.len() > remaining {
                    return Err(PageError::Limit("total content references"));
                }
                items
                    .iter()
                    .map(|item| {
                        reference(
                            Some(item),
                            "Contents array entries must be indirect streams",
                        )
                    })
                    .collect::<Result<_, _>>()?
            }
            _ => {
                return Err(PageError::Invalid(
                    "Contents must be a stream or stream array",
                ));
            }
        },
    };
    for id in &refs {
        if doc.object(*id)?.stream.is_none() {
            return Err(PageError::Invalid("Contents target is not a stream"));
        }
    }
    Ok(refs)
}

pub fn collect_pages(doc: &Document<'_>, limits: PageLimits) -> Result<Vec<PageInfo>, PageError> {
    let catalog = dictionary(doc, doc.root())?;
    if !matches!(catalog.get(b"Type".as_slice()), Some(Object::Name(name)) if name == b"Catalog") {
        return Err(PageError::Invalid("Root must have Type Catalog"));
    }
    let root = reference(
        catalog.get(b"Pages".as_slice()),
        "catalog Pages must be an indirect reference",
    )?;
    let mut stack = vec![Visit::Enter {
        id: root,
        parent: None,
        inherited: Inherited::default(),
        depth: 0,
    }];
    let mut seen = BTreeSet::new();
    let mut pages = Vec::new();
    let mut content_refs = 0usize;
    while let Some(visit) = stack.pop() {
        let (id, parent, mut inherited, depth) = match visit {
            Visit::Exit { start, expected } => {
                if pages.len() - start != expected {
                    return Err(PageError::Invalid("Pages Count disagrees with descendants"));
                }
                continue;
            }
            Visit::Enter {
                id,
                parent,
                inherited,
                depth,
            } => (id, parent, inherited, depth),
        };
        if depth > limits.max_depth.min(256) {
            return Err(PageError::Limit("tree depth"));
        }
        if seen.len() >= limits.max_nodes {
            return Err(PageError::Limit("tree nodes"));
        }
        if !seen.insert(id) {
            return Err(PageError::CycleOrSharedNode(id));
        }
        let dict = dictionary(doc, id)?;
        if let Some(parent) = parent {
            if reference(
                dict.get(b"Parent".as_slice()),
                "non-root node needs a Parent reference",
            )? != parent
            {
                return Err(PageError::Invalid("Parent reference disagrees with tree"));
            }
        } else if optional(doc, dict, b"Parent")?.is_some() {
            return Err(PageError::Invalid("page-tree root must not have Parent"));
        }
        if let Some(value) = optional(doc, dict, b"MediaBox")? {
            inherited.media = Some(rectangle(doc, value)?);
        }
        if let Some(value) = optional(doc, dict, b"CropBox")? {
            inherited.crop = Some(rectangle(doc, value)?);
        }
        if let Some(value) = optional(doc, dict, b"Rotate")? {
            let Object::Integer(rotation) = resolve(doc, value)? else {
                return Err(PageError::Invalid("Rotate must be an integer"));
            };
            if rotation % 90 != 0 {
                return Err(PageError::Invalid("Rotate must be a multiple of 90"));
            }
            inherited.rotation = rotation.rem_euclid(360) as u16;
        }
        match dict.get(b"Type".as_slice()) {
            Some(Object::Name(name)) if name == b"Pages" => {
                let count = match dict.get(b"Count".as_slice()) {
                    Some(value) => match resolve(doc, value)? {
                        Object::Integer(count) => usize::try_from(*count)
                            .map_err(|_| PageError::Invalid("invalid Pages Count"))?,
                        _ => return Err(PageError::Invalid("Pages Count must be an integer")),
                    },
                    None => return Err(PageError::Invalid("Pages needs Count")),
                };
                if count > limits.max_pages {
                    return Err(PageError::Limit("page count"));
                }
                let Some(kids) = dict.get(b"Kids".as_slice()) else {
                    return Err(PageError::Invalid("Pages needs Kids"));
                };
                let Object::Array(kids) = resolve(doc, kids)? else {
                    return Err(PageError::Invalid("Kids must be an array"));
                };
                if kids.len() > limits.max_nodes.saturating_sub(seen.len()) {
                    return Err(PageError::Limit("queued tree nodes"));
                }
                if stack.len().saturating_add(kids.len()).saturating_add(1)
                    > limits.max_nodes.saturating_mul(2).saturating_add(1)
                {
                    return Err(PageError::Limit("queued tree work"));
                }
                stack.push(Visit::Exit {
                    start: pages.len(),
                    expected: count,
                });
                for kid in kids.iter().rev() {
                    stack.push(Visit::Enter {
                        id: reference(Some(kid), "Kids entries must be indirect references")?,
                        parent: Some(id),
                        inherited,
                        depth: depth + 1,
                    });
                }
            }
            Some(Object::Name(name)) if name == b"Page" => {
                if parent.is_none() {
                    return Err(PageError::Invalid(
                        "catalog Pages target must be a Pages node",
                    ));
                }
                if pages.len() >= limits.max_pages {
                    return Err(PageError::Limit("pages"));
                }
                let media_box = inherited
                    .media
                    .ok_or(PageError::Invalid("Page has no inherited MediaBox"))?;
                let mut crop_box = inherited.crop.unwrap_or(media_box);
                crop_box = [
                    crop_box[0].max(media_box[0]),
                    crop_box[1].max(media_box[1]),
                    crop_box[2].min(media_box[2]),
                    crop_box[3].min(media_box[3]),
                ];
                if crop_box[2] <= crop_box[0] || crop_box[3] <= crop_box[1] {
                    return Err(PageError::Invalid("CropBox does not intersect MediaBox"));
                }
                let user_unit = match optional(doc, dict, b"UserUnit")? {
                    Some(value) => number(doc, value)?,
                    None => 1.0,
                };
                if user_unit <= 0.0 || user_unit > 75_000.0 {
                    return Err(PageError::Invalid(
                        "UserUnit outside supported positive range",
                    ));
                }
                let page_contents = contents(
                    doc,
                    dict,
                    limits.max_total_content_refs.saturating_sub(content_refs),
                )?;
                content_refs += page_contents.len();
                let page = PageInfo {
                    id,
                    media_box,
                    crop_box,
                    rotation: inherited.rotation,
                    user_unit,
                    contents: page_contents,
                };
                if page.display_bounds().iter().any(|v| !v.is_finite()) {
                    return Err(PageError::Invalid("page dimensions overflow"));
                }
                pages.push(page);
            }
            _ => return Err(PageError::Invalid("page-tree Type must be Page or Pages")),
        }
    }
    Ok(pages)
}
