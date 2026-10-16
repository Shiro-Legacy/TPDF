// SPDX-License-Identifier: AGPL-3.0-or-later
//! Device colour drawn through default colour spaces, as MuPDF draws it
//! (`pdf_load_default_colorspaces`, `fz_default_colorspace`): DeviceGray,
//! DeviceRGB and DeviceCMYK paint, images (Indexed bases included) and
//! shadings take the page's /DefaultGray, /DefaultRGB or /DefaultCMYK (a
//! form's own inside it), else the profile of the document's first output
//! intent when its family matches. Annotation appearances take the page's.
//! hayro reads device colour as given and ignores both, so the view names
//! the space in each resources dictionary on the way, rewrites the colour
//! operators and inline images of the content to use the name, and
//! redefines images and shadings with the space, in its private bytes only.
//! The same walk over page resources finds the tiling patterns `tiling`
//! redraws, the embedded CMaps `cmap` corrects and the names forms take
//! from enclosing resources (`inherit`).
use crate::content::{ContentLimits, ContentOperand, ContentParser};
use crate::lexer::{LexLimits, Lexer, TokenKind};
use crate::object::{Dictionary, Object, ObjectId};
use crate::stream::StreamLimits;

use super::super::info::{PageSlot, TreeNode};
use super::Fixes;

/// The names the view gives the default spaces of the gray, RGB and CMYK
/// families in a /ColorSpace resource dictionary.
const NAMES: [&[u8]; 3] = [b"TPDFDefaultGray", b"TPDFDefaultRGB", b"TPDFDefaultCMYK"];
const KEYS: [&[u8]; 3] = [b"DefaultGray", b"DefaultRGB", b"DefaultCMYK"];
const COMPONENTS: [i64; 3] = [1, 3, 4];
/// Decoded content read and rewritten per view, all streams together.
const MAX_CONTENT_BYTES: usize = 16 << 20;
/// Image and shading stream bytes copied per view to redefine their space.
const MAX_COPIED_BYTES: usize = 32 << 20;
/// Forms and patterns nested in forms.
const MAX_DEPTH: usize = 32;
const MAX_PROFILE_BYTES: usize = 4 << 20;

/// The space each device family (gray, RGB, CMYK) is drawn through; None
/// for the device's own.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Defaults([Option<Object>; 3]);

impl Defaults {
    fn is_empty(&self) -> bool {
        self.0.iter().all(Option::is_none)
    }

    /// Each family's own space where `self` has one, else `base`'s.
    fn over(mut self, base: &Defaults) -> Defaults {
        for (own, base) in self.0.iter_mut().zip(&base.0) {
            if own.is_none() {
                own.clone_from(base);
            }
        }
        self
    }
}

/// The view's state for device colour.
#[derive(Default)]
pub(super) struct Recolor {
    intent: Defaults,
    /// Objects already mapped: one that pages or forms with different
    /// defaults share keeps the first.
    pub(super) done: std::collections::BTreeSet<ObjectId>,
    /// Tiling patterns `tiling` looked at.
    pub(super) tiled: std::collections::BTreeSet<ObjectId>,
    /// The resources enclosing the form being walked, outermost first
    /// (`inherit`).
    pub(super) chain: Vec<Dictionary>,
    /// Form content `inherit` read.
    pub(super) scanned: usize,
    content_bytes: usize,
    copied_bytes: usize,
}

impl Recolor {
    pub(super) fn new(doc: &crate::document::Document<'_>) -> Self {
        Self {
            intent: output_intent(doc),
            ..Self::default()
        }
    }
}

/// The first output intent's /DestOutputProfile (MuPDF reads no other) as
/// the default space of its family, when its /N matches the profile. A
/// profile whose device model is sRGB is left out: hayro draws it as
/// DeviceRGB.
fn output_intent(doc: &crate::document::Document<'_>) -> Defaults {
    let mut defaults = Defaults::default();
    let first = (doc.resolve_id(doc.root()).as_dict())
        .and_then(|catalog| doc.dict_get(catalog, b"OutputIntents").as_array())
        .and_then(|intents| intents.first())
        .and_then(|intent| doc.resolve_dict(intent));
    let Some(Object::Reference(id)) = first.and_then(|i| i.get(b"DestOutputProfile".as_slice()))
    else {
        return defaults;
    };
    let Some(n) = (doc.stream_dict(*id).ok()).and_then(|d| doc.dict_get(d, b"N").as_i64()) else {
        return defaults;
    };
    let limits = StreamLimits {
        max_decoded_bytes: MAX_PROFILE_BYTES,
        max_total_decoded_bytes: 2 * MAX_PROFILE_BYTES,
        ..StreamLimits::default()
    };
    let Ok(profile) = doc.decoded_stream(*id, limits) else {
        return defaults;
    };
    let family = match profile.get(16..20) {
        Some(b"GRAY") => 0,
        Some(b"RGB ") => 1,
        Some(b"CMYK") => 2,
        _ => return defaults,
    };
    if n != COMPONENTS[family] || profile.get(52..56) == Some(b"sRGB") {
        return defaults;
    }
    defaults.0[family] = Some(Object::Array(vec![
        Object::name("ICCBased"),
        Object::Reference(*id),
    ]));
    defaults
}

/// The family of a device colour space name, abbreviations included (hayro
/// reads them anywhere).
fn device_family(name: &[u8]) -> Option<usize> {
    match name {
        b"DeviceGray" | b"G" => Some(0),
        b"DeviceRGB" | b"RGB" => Some(1),
        b"DeviceCMYK" | b"CMYK" => Some(2),
        _ => None,
    }
}

/// What a content stream belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A page's /Contents: the page's resources.
    Content,
    /// A form or appearance: its own resources, else its caller's.
    Form,
    /// A tiling pattern: its own resources, else none.
    Pattern,
}

/// Where the resources a page draws with are written.
enum Holder {
    /// A resources object.
    Object(ObjectId),
    /// Directly in the page or /Pages node object.
    In(ObjectId),
}

impl<'d> Fixes<'d, '_> {
    /// The view's value of a non-stream object.
    fn current(&self, id: ObjectId) -> Option<Object> {
        match self.objects.get(&id) {
            Some(value) => Some(value.clone()),
            None => {
                let object = self.doc.object(id).ok()?;
                object.stream.is_none().then(|| object.value.clone())
            }
        }
    }

    /// `value` resolved as the view sees it.
    pub(super) fn view_resolve<'s>(&'s self, value: &'s Object) -> &'s Object {
        match value {
            Object::Reference(id) => match self.objects.get(id) {
                Some(value) => value,
                None => self.doc.resolve_id(*id),
            },
            value => value,
        }
    }

    /// A stream's dictionary as the view has it.
    pub(super) fn stream_dict(&self, id: ObjectId) -> Option<&Dictionary> {
        match self.streams.get(&id) {
            Some((dict, _)) => Some(dict),
            None => self.doc.stream_dict(id).ok(),
        }
    }

    /// The defaults a resources dictionary sets: each /Default space whose
    /// component count is its family's (ICCBased, CalGray, CalRGB).
    fn resource_defaults(&self, resources: &Dictionary) -> Defaults {
        let mut defaults = Defaults::default();
        let spaces = resources.get(b"ColorSpace".as_slice());
        let Some(spaces) = spaces.and_then(|s| self.view_resolve(s).as_dict()) else {
            return defaults;
        };
        for family in 0..3 {
            let Some(value) = spaces.get(KEYS[family]) else {
                continue;
            };
            let space = self.doc.resolve(value).as_array().unwrap_or_default();
            let n = match space.first().and_then(Object::as_name) {
                Some(b"ICCBased") => (space.get(1).and_then(Object::as_reference))
                    .and_then(|id| self.doc.stream_dict(id).ok())
                    .and_then(|dict| self.doc.dict_get(dict, b"N").as_i64()),
                Some(b"CalGray") => Some(1),
                Some(b"CalRGB") => Some(3),
                _ => None,
            };
            if n == Some(COMPONENTS[family]) {
                defaults.0[family] = Some(value.clone());
            }
        }
        defaults
    }

    /// Walks the resources, content and annotation appearances of a page
    /// written in its own object: device colour when the page has defaults
    /// (resources, content, images, shadings), tiling patterns always
    /// (`tiling`).
    pub(super) fn walk_page(&mut self, slot: PageSlot<'_>, path: &[TreeNode<'d>]) {
        let PageSlot::Object(page_id) = slot else {
            return;
        };
        let Some(value) = self.current(page_id) else {
            return;
        };
        let Some(dict) = value.as_dict() else {
            return;
        };
        // hayro inherits along the /Kids path; the view wrote MuPDF's
        // inherited /Resources onto pages whose /Parent differs.
        let holder = match dict.get(b"Resources".as_slice()) {
            Some(Object::Reference(id)) => Some(Holder::Object(*id)),
            Some(_) => Some(Holder::In(page_id)),
            None => (path.iter().rev())
                .find_map(|node| node.dict.get(b"Resources".as_slice()).map(|r| (node, r)))
                .and_then(|(node, resources)| match resources {
                    Object::Reference(id) => Some(Holder::Object(*id)),
                    _ => node.id.map(Holder::In),
                }),
        };
        let resources = match &holder {
            Some(Holder::Object(id)) => self.current(*id),
            Some(Holder::In(id)) => (self.current(*id).as_ref())
                .and_then(Object::as_dict)
                .and_then(|holder| holder.get(b"Resources".as_slice()).cloned()),
            None => None,
        };
        let page_resources = resources.as_ref().and_then(Object::as_dict).cloned();
        let own = (page_resources.as_ref())
            .map(|resources| self.resource_defaults(resources))
            .unwrap_or_default();
        let mut defaults = own.over(&self.recolor.intent);
        if self.recolor.content_bytes > MAX_CONTENT_BYTES {
            defaults = Defaults::default();
        }
        let named = match holder {
            Some(Holder::Object(id)) => self.map_resources_object(id, &defaults, 0),
            Some(Holder::In(id)) => self.map_resources_in(id, &defaults),
            // No resources anywhere: the page gets the names alone.
            None if !defaults.is_empty() => self.map_resources_in(page_id, &defaults),
            None => [false; 3],
        };
        let page_value = self.current(page_id);
        let page_dict = page_value.as_ref().and_then(Object::as_dict);
        let contents = (page_dict.and_then(|p| p.get(b"Contents".as_slice()))).cloned();
        let annots = page_dict.and_then(|p| p.get(b"Annots".as_slice())).cloned();
        if named.iter().any(|&n| n) {
            for id in self.stream_list(contents.as_ref()) {
                self.map_stream(id, &defaults, named, Kind::Content, 0);
            }
        }
        // Appearances draw with the page's resources when they have none.
        let Some(annots) = annots else {
            return;
        };
        let items = (self.view_resolve(&annots).as_array().unwrap_or_default()).to_vec();
        // An appearance's own resources fall back on the page's (`inherit`).
        let chained = page_resources.is_some();
        self.recolor.chain.extend(page_resources);
        for item in items {
            let appearances = match self.view_resolve(&item) {
                Object::Dictionary(annot) => (annot.get(b"AP".as_slice()))
                    .and_then(|ap| self.view_resolve(ap).as_dict())
                    .and_then(|ap| ap.get(b"N".as_slice()))
                    .map(|normal| self.stream_list(Some(normal))),
                _ => None,
            };
            for id in appearances.unwrap_or_default() {
                self.map_stream(id, &defaults, named, Kind::Form, 1);
            }
        }
        if chained {
            self.recolor.chain.pop();
        }
    }

    /// The stream objects `value` names: a stream, or an array or a
    /// dictionary (appearance states) of them.
    fn stream_list(&self, value: Option<&Object>) -> Vec<ObjectId> {
        let is_stream = |id: &ObjectId| self.stream_dict(*id).is_some();
        match value {
            Some(Object::Reference(id)) if is_stream(id) => vec![*id],
            Some(value) => {
                let items: Vec<&Object> = match self.view_resolve(value) {
                    Object::Array(items) => items.iter().collect(),
                    Object::Dictionary(states) => states.values().collect(),
                    _ => Vec::new(),
                };
                (items.into_iter())
                    .filter_map(Object::as_reference)
                    .filter(is_stream)
                    .collect()
            }
            None => Vec::new(),
        }
    }

    /// Maps the /Resources written directly in object `id` (a page or a
    /// /Pages node), creating them when absent. Whether the names are in.
    fn map_resources_in(&mut self, id: ObjectId, defaults: &Defaults) -> [bool; 3] {
        let Some(mut value) = self.current(id) else {
            return [false; 3];
        };
        let Some(holder) = value.as_dict_mut() else {
            return [false; 3];
        };
        let mut resources = match holder.remove(b"Resources".as_slice()) {
            Some(Object::Dictionary(resources)) => resources,
            _ => Dictionary::new(),
        };
        let (named, changed) = self.map_resources(&mut resources, defaults, 0);
        if changed {
            holder.insert(b"Resources".to_vec(), Object::Dictionary(resources));
            self.objects.insert(id, value);
        }
        named
    }

    /// Maps a resources object once. Whether the names are in.
    fn map_resources_object(
        &mut self,
        id: ObjectId,
        defaults: &Defaults,
        depth: usize,
    ) -> [bool; 3] {
        if self.recolor.done.insert(id) {
            if let Some(mut value) = self.current(id) {
                if let Some(resources) = value.as_dict_mut() {
                    if self.map_resources(resources, defaults, depth).1 {
                        self.objects.insert(id, value);
                    }
                }
            }
        }
        let value = self.current(id);
        self.names_in(value.as_ref().and_then(Object::as_dict), defaults)
    }

    /// Which families of `defaults` a resources dictionary names.
    fn names_in(&self, resources: Option<&Dictionary>, defaults: &Defaults) -> [bool; 3] {
        let spaces = (resources.and_then(|r| r.get(b"ColorSpace".as_slice())))
            .and_then(|spaces| self.view_resolve(spaces).as_dict());
        std::array::from_fn(|family| {
            defaults.0[family].is_some()
                && spaces.is_some_and(|spaces| spaces.contains_key(NAMES[family]))
        })
    }

    /// Names the default spaces in `resources` and maps the forms, images,
    /// patterns and shadings it holds: whether the names are in, and
    /// whether `resources` changed.
    fn map_resources(
        &mut self,
        resources: &mut Dictionary,
        defaults: &Defaults,
        depth: usize,
    ) -> ([bool; 3], bool) {
        let colour = !defaults.is_empty();
        let mut changed = false;
        let add = |spaces: &mut Dictionary| {
            for (name, space) in NAMES.iter().zip(&defaults.0) {
                if let Some(space) = space {
                    spaces.insert(name.to_vec(), space.clone());
                }
            }
        };
        match resources.get_mut(b"ColorSpace".as_slice()) {
            _ if !colour => {}
            Some(Object::Reference(id)) => {
                let id = *id;
                if self.recolor.done.insert(id) {
                    if let Some(mut value) = self.current(id) {
                        if let Some(spaces) = value.as_dict_mut() {
                            add(spaces);
                            self.objects.insert(id, value);
                        }
                    }
                }
            }
            Some(Object::Dictionary(spaces)) => {
                add(spaces);
                changed = true;
            }
            _ => {
                let mut spaces = Dictionary::new();
                add(&mut spaces);
                resources.insert(b"ColorSpace".to_vec(), Object::Dictionary(spaces));
                changed = true;
            }
        }
        // A form without resources draws with these.
        let named = self.names_in(Some(resources), defaults);
        if depth < MAX_DEPTH {
            let xobjects = (resources.get(b"XObject".as_slice()))
                .and_then(|x| self.view_resolve(x).as_dict())
                .map(|x| {
                    x.values()
                        .filter_map(Object::as_reference)
                        .collect::<Vec<_>>()
                });
            self.recolor.chain.push(resources.clone());
            for id in xobjects.unwrap_or_default() {
                match (self.stream_dict(id)).map(|d| self.doc.dict_get(d, b"Subtype")) {
                    Some(Object::Name(kind)) if kind == b"Form" => {
                        self.map_stream(id, defaults, named, Kind::Form, depth + 1)
                    }
                    Some(Object::Name(kind)) if kind == b"Image" && colour => {
                        self.map_image(id, defaults)
                    }
                    _ => {}
                }
            }
            self.recolor.chain.pop();
            let fonts = (resources.get(b"Font".as_slice()))
                .and_then(|f| self.view_resolve(f).as_dict())
                .map(|f| f.values().cloned().collect::<Vec<_>>());
            for font in fonts.unwrap_or_default() {
                self.fix_cmap_wmode(&font);
            }
            for key in [b"Pattern".as_slice(), b"Shading"] {
                if key == b"Shading" && !colour {
                    continue;
                }
                if let Some(holder) = resources.get_mut(key) {
                    changed |= self.map_space_holder(holder, defaults, depth, key == b"Pattern");
                }
            }
        }
        (named, changed)
    }

    /// Maps each pattern or shading of a /Pattern or /Shading resource
    /// dictionary, editing `holder` in place when it is direct. Whether
    /// `holder` changed.
    fn map_space_holder(
        &mut self,
        holder: &mut Object,
        defaults: &Defaults,
        depth: usize,
        patterns: bool,
    ) -> bool {
        let Object::Reference(id) = holder else {
            return self.map_space_entries(holder, defaults, depth, patterns);
        };
        let id = *id;
        if !self.recolor.done.insert(id) {
            return false;
        }
        if let Some(mut value) = self.current(id) {
            if self.map_space_entries(&mut value, defaults, depth, patterns) {
                self.objects.insert(id, value);
            }
        }
        false
    }

    /// `map_space_holder` on a dictionary's entries. Whether one changed.
    fn map_space_entries(
        &mut self,
        value: &mut Object,
        defaults: &Defaults,
        depth: usize,
        patterns: bool,
    ) -> bool {
        let Some(dict) = value.as_dict_mut() else {
            return false;
        };
        let mut changed = false;
        for value in dict.values_mut() {
            changed |= match patterns {
                true => self.map_pattern(value, defaults, depth),
                false => self.map_shading(value, defaults),
            };
        }
        changed
    }

    /// Maps a pattern: a tiling pattern's content as a form's (then its
    /// tiles, `tiling`), a shading pattern's shading. True when `value`
    /// (direct) changed.
    fn map_pattern(&mut self, value: &mut Object, defaults: &Defaults, depth: usize) -> bool {
        if let Object::Reference(id) = value {
            let id = *id;
            if let Some(dict) = self.stream_dict(id) {
                // An uncoloured pattern's own colour is ignored.
                let colored = self.doc.dict_get(dict, b"PaintType").as_i64() != Some(2);
                let none = Defaults::default();
                let defaults = if colored { defaults } else { &none };
                self.map_stream(id, defaults, [false; 3], Kind::Pattern, depth + 1);
                self.fix_tiling(id);
                return false;
            }
            if defaults.is_empty() || !self.recolor.done.insert(id) {
                return false;
            }
            let Some(mut pattern) = self.current(id) else {
                return false;
            };
            let shading = (pattern.as_dict_mut()).and_then(|p| p.get_mut(b"Shading".as_slice()));
            if let Some(shading) = shading {
                if self.map_shading(shading, defaults) {
                    self.objects.insert(id, pattern);
                }
            }
            return false;
        }
        match (value.as_dict_mut()).and_then(|p| p.get_mut(b"Shading".as_slice())) {
            Some(shading) if !defaults.is_empty() => self.map_shading(shading, defaults),
            _ => false,
        }
    }

    /// Maps a shading's colour space. True when `value` (direct) changed.
    fn map_shading(&mut self, value: &mut Object, defaults: &Defaults) -> bool {
        let Object::Reference(id) = value else {
            return self.map_space_of(value.as_dict_mut(), b"ColorSpace", defaults);
        };
        let id = *id;
        if !self.recolor.done.insert(id) {
            return false;
        }
        if self.stream_dict(id).is_some() {
            self.redefine_raw(id, defaults);
        } else if let Some(mut shading) = self.current(id) {
            if self.map_space_of(shading.as_dict_mut(), b"ColorSpace", defaults) {
                self.objects.insert(id, shading);
            }
        }
        false
    }

    /// Replaces `dict`'s colour space under `key` by its default. Whether it did.
    fn map_space_of(&self, dict: Option<&mut Dictionary>, key: &[u8], defaults: &Defaults) -> bool {
        let Some(dict) = dict else {
            return false;
        };
        let mapped = (dict.get(key)).and_then(|space| self.mapped_space(space, defaults));
        match mapped {
            Some(space) => {
                dict.insert(key.to_vec(), space);
                true
            }
            None => false,
        }
    }

    /// A colour space with its device space, or an Indexed space's device
    /// base, replaced by the family's default; None when it has none.
    fn mapped_space(&self, space: &Object, defaults: &Defaults) -> Option<Object> {
        match self.doc.resolve(space) {
            Object::Name(name) => defaults.0[device_family(name)?].clone(),
            Object::Array(items) if items.len() == 4 => {
                let indexed = matches!(items[0].as_name(), Some(b"Indexed" | b"I"));
                let family = device_family(self.doc.resolve(&items[1]).as_name()?)?;
                let base = defaults.0[family].clone().filter(|_| indexed)?;
                let mut items = items.clone();
                items[1] = base;
                Some(Object::Array(items))
            }
            _ => None,
        }
    }

    /// Redefines an image whose colour space (or Indexed base) is a device
    /// space with a default, copying its stored bytes.
    fn map_image(&mut self, id: ObjectId, defaults: &Defaults) {
        if !self.recolor.done.insert(id) {
            return;
        }
        let Some(dict) = self.stream_dict(id) else {
            return;
        };
        let doc = self.doc;
        if doc.dict_get(dict, b"ImageMask").as_bool() == Some(true) {
            return;
        }
        self.redefine_raw(id, defaults);
    }

    /// Redefines stream `id` with its /ColorSpace mapped and its bytes as
    /// stored, within the copy budget.
    fn redefine_raw(&mut self, id: ObjectId, defaults: &Defaults) {
        let (mut dict, data) = match self.streams.get(&id) {
            Some((dict, data)) => (dict.clone(), data.clone()),
            None => {
                let Ok(dict) = self.doc.stream_dict(id) else {
                    return;
                };
                let Ok(Some(data)) = self.doc.stream_data(id) else {
                    return;
                };
                if self.recolor.copied_bytes.saturating_add(data.len()) > MAX_COPIED_BYTES
                    || (self.mapped_space(self.doc.dict_get(dict, b"ColorSpace"), defaults))
                        .is_none()
                {
                    return;
                }
                self.recolor.copied_bytes += data.len();
                (dict.clone(), data.to_vec())
            }
        };
        if self.map_space_of(Some(&mut dict), b"ColorSpace", defaults) {
            self.streams.insert(id, (dict, data));
        }
    }

    /// Maps a content stream: a form's, pattern's or appearance's own
    /// resources and defaults first, then its colour operators.
    /// `inherited`: the families named in the resources a form without its
    /// own draws with.
    fn map_stream(
        &mut self,
        id: ObjectId,
        defaults: &Defaults,
        inherited: [bool; 3],
        kind: Kind,
        depth: usize,
    ) {
        if depth > MAX_DEPTH || !self.recolor.done.insert(id) {
            return;
        }
        let Some(mut dict) = self.stream_dict(id).cloned() else {
            return;
        };
        // A page content stream's dictionary holds no resources; a pattern
        // without its own has none (hayro, MuPDF).
        let resources = match kind {
            Kind::Content => None,
            Kind::Form => dict.get(b"Resources".as_slice()).cloned(),
            Kind::Pattern => (dict.get(b"Resources".as_slice()).cloned())
                .or_else(|| (!defaults.is_empty()).then(|| Object::Dictionary(Dictionary::new()))),
        };
        let mut changed = false;
        // Names the content takes from the enclosing resources (`inherit`).
        let resources = match resources {
            Some(Object::Reference(r)) if kind == Kind::Form && !self.recolor.done.contains(&r) => {
                if let Some(mut value) = self.current(r) {
                    if (value.as_dict_mut()).is_some_and(|own| self.inherit_names(id, own)) {
                        self.objects.insert(r, value);
                    }
                }
                Some(Object::Reference(r))
            }
            Some(Object::Dictionary(mut own)) if kind == Kind::Form => {
                changed = self.inherit_names(id, &mut own);
                Some(Object::Dictionary(own))
            }
            other => other,
        };
        let own = (resources.as_ref())
            .and_then(|r| self.view_resolve(r).as_dict())
            .map(|r| self.resource_defaults(r))
            .unwrap_or_default();
        let defaults = own.over(defaults);
        let named = match resources {
            Some(Object::Reference(resources)) => {
                self.map_resources_object(resources, &defaults, depth)
            }
            Some(Object::Dictionary(mut resources)) => {
                let (named, edited) = self.map_resources(&mut resources, &defaults, depth);
                changed |= edited;
                if changed {
                    dict.insert(b"Resources".to_vec(), Object::Dictionary(resources));
                }
                named
            }
            _ if kind == Kind::Pattern => [false; 3],
            _ => inherited,
        };
        if !changed && !named.iter().any(|&n| n) {
            return;
        }
        let Some(content) = self.decoded(id) else {
            return;
        };
        let rewritten = rewrite(&content, named);
        if !changed && rewritten.is_none() {
            return;
        }
        for key in [b"Filter".as_slice(), b"DecodeParms", b"DL"] {
            dict.remove(key);
        }
        let data = rewritten.unwrap_or(content);
        self.streams.insert(id, (dict, data));
    }

    /// A stream's decoded bytes within the content budget.
    pub(super) fn decoded(&mut self, id: ObjectId) -> Option<Vec<u8>> {
        if let Some((_, data)) = self.streams.get(&id) {
            return Some(data.clone());
        }
        let left = MAX_CONTENT_BYTES.checked_sub(self.recolor.content_bytes)?;
        let limits = StreamLimits {
            max_decoded_bytes: left,
            max_total_decoded_bytes: left.saturating_mul(2),
            ..StreamLimits::default()
        };
        let data = self.doc.decoded_stream(id, limits).ok()?;
        self.recolor.content_bytes += data.len().max(1);
        Some(data)
    }
}

/// `content` with its device colour operators (`g`, `rg`, `k`, their
/// stroking forms, `cs`/`CS` of a device space) and inline images of the
/// families `named` set using the default space's resource name; None when
/// nothing changes. What follows a syntax error the content parser stops
/// at is kept as it is.
pub(super) fn rewrite(content: &[u8], named: [bool; 3]) -> Option<Vec<u8>> {
    if !named.iter().any(|&n| n) {
        return None;
    }
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
    let mut parser = ContentParser::new(content, limits);
    let mut out: Option<Vec<u8>> = None;
    let mut copied = 0;
    while let Ok(Some(op)) = parser.next_operation() {
        let Some(replacement) = replacement(content, &op, named) else {
            continue;
        };
        let out = out.get_or_insert_with(|| Vec::with_capacity(content.len() + 256));
        out.extend_from_slice(content.get(copied..op.span.start)?);
        out.extend_from_slice(&replacement);
        copied = op.span.end;
    }
    let mut out = out?;
    out.extend_from_slice(content.get(copied..)?);
    Some(out)
}

/// The bytes replacing one operation, if it uses a mapped device space.
fn replacement(
    content: &[u8],
    op: &crate::content::Operation,
    named: [bool; 3],
) -> Option<Vec<u8>> {
    let numbers = |n: i64| {
        op.operands.len() as i64 == n
            && (op.operands.iter())
                .all(|o| matches!(o, ContentOperand::Integer(_) | ContentOperand::Real(_)))
    };
    let (family, stroke) = match op.operator.as_slice() {
        b"g" => (0, false),
        b"G" => (0, true),
        b"rg" => (1, false),
        b"RG" => (1, true),
        b"k" => (2, false),
        b"K" => (2, true),
        b"cs" | b"CS" => {
            let [ContentOperand::Name(name)] = op.operands.as_slice() else {
                return None;
            };
            let family = device_family(name).filter(|&f| named[f])?;
            let mut out = vec![b'/'];
            out.extend_from_slice(NAMES[family]);
            out.push(b' ');
            out.extend_from_slice(&op.operator);
            return Some(out);
        }
        b"BI" => return inline_image(content, op, named),
        _ => return None,
    };
    if !named[family] || !numbers(COMPONENTS[family]) {
        return None;
    }
    let operands = content.get(op.span.start..op.span.end.checked_sub(op.operator.len())?)?;
    let (set, paint) = match stroke {
        true => (b" CS ", b" SC"),
        false => (b" cs ", b" sc"),
    };
    let mut out = vec![b'/'];
    out.extend_from_slice(NAMES[family]);
    out.extend_from_slice(set);
    out.extend_from_slice(operands.trim_ascii_end());
    out.extend_from_slice(paint);
    Some(out)
}

/// An inline image whose /CS (or /ColorSpace) is a mapped device space,
/// with the value replaced by the default space's resource name (hayro
/// resolves an inline image's named space through the resources).
fn inline_image(
    content: &[u8],
    op: &crate::content::Operation,
    named: [bool; 3],
) -> Option<Vec<u8>> {
    let ContentOperand::Dictionary(entries) = op.operands.first()? else {
        return None;
    };
    let mask = [b"IM".as_slice(), b"ImageMask"]
        .iter()
        .any(|k| entries.get(*k) == Some(&ContentOperand::Boolean(true)));
    let family = [b"CS".as_slice(), b"ColorSpace"]
        .iter()
        .find_map(|k| match entries.get(*k) {
            Some(ContentOperand::Name(name)) => device_family(name),
            _ => None,
        });
    let family = family.filter(|&f| named[f] && !mask)?;
    let image = content.get(op.span.clone())?;
    let mut lexer = Lexer::new(image, LexLimits::default());
    // BI, then key and value pairs at depth 0 up to ID.
    let (mut depth, mut key, mut value_of_cs) = (0usize, true, false);
    let _ = lexer.next_token().ok()??;
    loop {
        let token = lexer.next_token().ok()??;
        match &token.kind {
            TokenKind::Keyword(word) if depth == 0 && word == b"ID" => return None,
            TokenKind::ArrayStart | TokenKind::DictStart => depth += 1,
            TokenKind::ArrayEnd | TokenKind::DictEnd => depth = depth.checked_sub(1)?,
            TokenKind::Name(name) if depth == 0 && key => {
                value_of_cs = matches!(name.as_slice(), b"CS" | b"ColorSpace");
                key = false;
                continue;
            }
            TokenKind::Name(_) if depth == 0 && value_of_cs => {
                let mut out = image.get(..token.span.start)?.to_vec();
                out.push(b'/');
                out.extend_from_slice(NAMES[family]);
                out.extend_from_slice(image.get(token.span.end..)?);
                return Some(out);
            }
            _ => {}
        }
        if depth == 0 {
            key = true;
            value_of_cs = false;
        }
    }
}
