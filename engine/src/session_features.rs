// SPDX-License-Identifier: AGPL-3.0-or-later
//! Feature admission and annotation appearance selection. No actions execute.
//! Inspect catalog/page entries, not similarly named keys in unrelated objects.
use super::{Result, SessionError, err};
use crate::{
    document::Document,
    object::{Dictionary, Object, ObjectId},
    page_program::AnnotationAppearance,
    pages::PageInfo,
};
use std::collections::BTreeSet;

fn resolve<'a>(doc: &'a Document<'_>, mut value: &'a Object) -> Result<&'a Object> {
    let mut seen = BTreeSet::new();
    while let Object::Reference(id) = value {
        if seen.len() >= 64 || !seen.insert(*id) {
            return Err(err(
                "invalid-document",
                "cyclic or excessive feature reference chain",
            ));
        }
        let object = doc.object(*id).map_err(|e| err("invalid-document", e))?;
        if object.stream.is_some() {
            return Err(err(
                "invalid-document",
                "expected a feature value, found a stream",
            ));
        }
        value = &object.value;
    }
    Ok(value)
}
fn dictionary<'a>(doc: &'a Document<'_>, value: &'a Object) -> Result<&'a Dictionary> {
    match resolve(doc, value)? {
        Object::Dictionary(dict) => Ok(dict),
        _ => Err(err("invalid-document", "expected a feature dictionary")),
    }
}
fn optional<'a>(
    doc: &'a Document<'_>,
    dict: &'a Dictionary,
    key: &[u8],
) -> Result<Option<&'a Object>> {
    match dict.get(key) {
        None => Ok(None),
        Some(value) => match resolve(doc, value)? {
            Object::Null => Ok(None),
            value => Ok(Some(value)),
        },
    }
}
fn number(doc: &Document<'_>, value: &Object) -> Result<f64> {
    match resolve(doc, value)? {
        Object::Integer(n) => Ok(*n as f64),
        Object::Real(n) if n.is_finite() => Ok(*n),
        _ => Err(err(
            "invalid-document",
            "expected a finite annotation number",
        )),
    }
}

/// Document-level admission. Hard failures stop viewing; the returned
/// restriction (form fields present) only withholds editing and saving until
/// form values and appearances can be preserved transactionally (M6).
pub(super) fn check_document(doc: &Document<'_>) -> Result<Option<SessionError>> {
    let root = doc
        .object(doc.root())
        .map_err(|e| err("invalid-document", e))?;
    let catalog = dictionary(doc, &root.value)?;
    let mut restriction = None;
    if let Some(form) = optional(doc, catalog, b"AcroForm")? {
        let form = dictionary(doc, form)?;
        let fields = match optional(doc, form, b"Fields")? {
            None => 0,
            Some(Object::Array(fields)) => fields.len(),
            _ => return Err(err("invalid-document", "AcroForm Fields must be an array")),
        };
        let xfa = optional(doc, form, b"XFA")?.is_some();
        if fields > 0 || xfa {
            // NeedAppearances is honored at display time: text and choice
            // fields are regenerated from their values (page_annotations).
            restriction = Some(err(
                "unsupported-forms",
                if xfa {
                    "This PDF has XFA form data. Its AcroForm appearances can be viewed, but forms cannot be edited or saved yet.".to_string()
                } else {
                    format!(
                        "This PDF has {fields} top-level form fields. They can be viewed, but documents with form fields cannot be edited or saved yet."
                    )
                },
            ));
        }
    }
    if let Some(layers) = optional(doc, catalog, b"OCProperties")? {
        let layers = dictionary(doc, layers)?;
        // Layers render in their default state. Editing stays off: edit
        // validation cannot yet see text in hidden layers that an edit could move.
        if !layers.is_empty() && restriction.is_none() {
            restriction = Some(err(
                "unsupported-layers",
                "This PDF has layers. It can be viewed with its default layer state, but layered documents cannot be edited or saved yet.",
            ));
        }
    }
    Ok(restriction)
}

const HIDDEN: i64 = 1 << 1;
const NO_VIEW: i64 = 1 << 5;

/// A stored appearance stream, or one generated for display only.
pub(super) enum PlannedAnnotation {
    Stored(AnnotationAppearance),
    Generated {
        annotation: Option<ObjectId>,
        rect: [f64; 4],
        appearance: crate::generated_appearance::GeneratedAppearance,
    },
}

/// Select the normal appearance of every visible annotation on a page, in
/// page order. Annotations without an appearance draw nothing only when that
/// is certain (borderless links, popups, empty plain text fields); otherwise the page is refused
/// rather than shown without content a viewer would draw.
pub(super) fn page_annotations(
    doc: &Document<'_>,
    page: &PageInfo,
    page_index: usize,
) -> Result<Vec<PlannedAnnotation>> {
    let need_appearances = (|| {
        let root = doc.object(doc.root()).ok()?;
        let catalog = dictionary(doc, &root.value).ok()?;
        let form = dictionary(doc, optional(doc, catalog, b"AcroForm").ok()??).ok()?;
        Some(matches!(
            optional(doc, form, b"NeedAppearances").ok()?,
            Some(Object::Boolean(true))
        ))
    })()
    .unwrap_or(false);
    let obj = doc
        .object(page.id)
        .map_err(|e| err("invalid-document", e))?;
    let page_dict = dictionary(doc, &obj.value)?;
    let mut appearances = Vec::new();
    let layers = crate::optional_content::OptionalContent::load(doc);
    let Some(annots) = optional(doc, page_dict, b"Annots")? else {
        return Ok(appearances);
    };
    let Object::Array(annots) = annots else {
        return Err(err("invalid-document", "page Annots must be an array"));
    };
    if annots.len() > 10_000 {
        return Err(err("too-large", "annotation admission budget"));
    }
    for entry in annots {
        let annot = dictionary(doc, entry)?;
        let subtype = match optional(doc, annot, b"Subtype")? {
            Some(Object::Name(name)) => name.as_slice(),
            _ => return Err(err("invalid-document", "annotation Subtype must be a name")),
        };
        let refuse = |why: &str| {
            err(
                "unsupported-annotation",
                format!(
                    "Page {} has a {} annotation {why}.",
                    page_index + 1,
                    String::from_utf8_lossy(subtype)
                ),
            )
        };
        let flags = match optional(doc, annot, b"F")? {
            None => 0,
            Some(Object::Integer(n)) => *n,
            Some(_) => return Err(err("invalid-document", "annotation F must be an integer")),
        };
        if flags & (HIDDEN | NO_VIEW) != 0 {
            continue;
        }
        if let Some(oc) = annot.get(b"OC".as_slice()) {
            if !layers.visible(doc, oc) {
                continue;
            }
        }
        let rect = annotation_rect(doc, annot)?;
        if rect[2] <= rect[0] || rect[3] <= rect[1] {
            continue;
        }
        let regenerate = need_appearances
            && subtype == b"Widget"
            && matches!(inherited(doc, annot, b"FT")?, Some(Object::Name(t)) if t == b"Tx" || t == b"Ch");
        let stream = match optional(doc, annot, b"AP")? {
            _ if regenerate => None,
            None => None,
            Some(ap) => normal_appearance(doc, annot, dictionary(doc, ap)?)?,
        };
        let Some(stream) = stream else {
            if !regenerate
                && annot
                    .get(b"AP".as_slice())
                    .is_some_and(|v| !matches!(v, Object::Null))
            {
                // A state dictionary without a matching /AS draws nothing.
                continue;
            }
            match subtype {
                b"Popup" => continue,
                b"Link" if link_border_width(doc, annot)? == 0. => continue,
                _ => match crate::generated_appearance::generate(doc, annot, rect) {
                    Ok(None) => continue,
                    Ok(Some(appearance)) => appearances.push(PlannedAnnotation::Generated {
                        annotation: match entry {
                            Object::Reference(id) => Some(*id),
                            _ => None,
                        },
                        rect,
                        appearance,
                    }),
                    Err(why) => {
                        return Err(refuse(&format!(
                            "without an appearance stream ({why}), which the Rust preview cannot generate yet"
                        )));
                    }
                },
            }
            continue;
        };
        let annotation = match entry {
            Object::Reference(id) => *id,
            _ => stream,
        };
        appearances.push(PlannedAnnotation::Stored(AnnotationAppearance {
            annotation,
            stream,
            rect,
        }));
    }
    Ok(appearances)
}

/// A field attribute from the widget or its nearest ancestor (/Parent chain).
fn inherited<'a>(
    doc: &'a Document<'_>,
    mut field: &'a Dictionary,
    key: &[u8],
) -> Result<Option<&'a Object>> {
    for _ in 0..64 {
        if let Some(value) = field.get(key) {
            return Ok(Some(value));
        }
        match optional(doc, field, b"Parent")? {
            None => return Ok(None),
            Some(parent) => field = dictionary(doc, parent)?,
        }
    }
    Err(err(
        "invalid-document",
        "form field Parent chain is too deep",
    ))
}

fn annotation_rect(doc: &Document<'_>, annot: &Dictionary) -> Result<[f64; 4]> {
    let Some(Object::Array(rect)) = optional(doc, annot, b"Rect")? else {
        return Err(err("invalid-document", "annotation Rect must be an array"));
    };
    if rect.len() != 4 {
        return Err(err(
            "invalid-document",
            "annotation Rect must have four numbers",
        ));
    }
    let v = [
        number(doc, &rect[0])?,
        number(doc, &rect[1])?,
        number(doc, &rect[2])?,
        number(doc, &rect[3])?,
    ];
    Ok([
        v[0].min(v[2]),
        v[1].min(v[3]),
        v[0].max(v[2]),
        v[1].max(v[3]),
    ])
}

/// The /N appearance: a stream directly, or the subdictionary entry named
/// by /AS. Only indirect streams qualify.
fn normal_appearance(
    doc: &Document<'_>,
    annot: &Dictionary,
    ap: &Dictionary,
) -> Result<Option<ObjectId>> {
    let Some(normal) = ap.get(b"N".as_slice()) else {
        return Ok(None);
    };
    let stream_id = |value: &Object| -> Result<Option<ObjectId>> {
        let Object::Reference(id) = value else {
            return Ok(None);
        };
        let object = doc.object(*id).map_err(|e| err("invalid-document", e))?;
        Ok(object.stream.is_some().then_some(*id))
    };
    if let Some(id) = stream_id(normal)? {
        return Ok(Some(id));
    }
    let states = dictionary(doc, normal)?;
    let Some(Object::Name(state)) = optional(doc, annot, b"AS")? else {
        return Ok(None);
    };
    match states.get(state.as_slice()) {
        Some(value) => stream_id(value),
        None => Ok(None),
    }
}

// ISO 32000-1, 12.5.4/12.5.6.5: BS takes precedence over Border; omitted
// widths default to 1, not zero.
fn link_border_width(doc: &Document<'_>, annot: &Dictionary) -> Result<f64> {
    if let Some(style) = optional(doc, annot, b"BS")? {
        let style = dictionary(doc, style)?;
        return optional(doc, style, b"W")?
            .map(|v| number(doc, v))
            .transpose()
            .map(|w| w.unwrap_or(1.));
    }
    let Some(border) = optional(doc, annot, b"Border")? else {
        return Ok(1.);
    };
    let Object::Array(border) = border else {
        return Err(err("invalid-document", "link Border must be an array"));
    };
    if !(3..=4).contains(&border.len()) {
        return Err(err(
            "invalid-document",
            "link Border must have three or four entries",
        ));
    }
    for radius in &border[..2] {
        if number(doc, radius)? < 0. {
            return Err(err("invalid-document", "negative link border radius"));
        }
    }
    number(doc, &border[2])
}

/// Until structure-tree ActualText/Alt is updated transactionally, tagged
/// documents may be viewed and saved unchanged, but cannot expose text edits.
pub(super) fn check_text_edit(doc: &Document<'_>) -> Result<()> {
    let root = doc
        .object(doc.root())
        .map_err(|e| err("invalid-document", e))?;
    let catalog = dictionary(doc, &root.value)?;
    if optional(doc, catalog, b"StructTreeRoot")?.is_some() {
        return Err(err(
            "not-candidate",
            "Text editing in tagged PDFs needs structure-tree updates. This document can be viewed and saved unchanged.",
        ));
    }
    Ok(())
}
