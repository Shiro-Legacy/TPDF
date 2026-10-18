// SPDX-License-Identifier: MIT
//! Device colour drawn through default colour spaces, as reference viewers
//! draw it: DeviceGray,
//! DeviceRGB and DeviceCMYK paint, images (Indexed bases included) and
//! shadings take the page's /DefaultGray, /DefaultRGB or /DefaultCMYK (a
//! form's own inside it), else the profile of the document's first output
//! intent when its family matches. Annotation appearances take the page's.
//! hayro reads device colour as given and ignores both, so the view names
//! the space in each resources dictionary on the way, rewrites the colour
//! operators and inline images of the content to use the name, and
//! redefines images and shadings with the space, in its private bytes only.
//! The same walk over page resources finds the tiling patterns `tiling`
//! redraws, the embedded CMaps `cmap` corrects, the substituted CID fonts
//! `cidfont` corrects, the transfer functions `gstate` leaves out and the
//! names forms take from enclosing resources (`inherit`).
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::content::{ContentLimits, ContentOperand, ContentParser};
use crate::lexer::{LexLimits, Lexer, TokenKind};
use crate::object::{Dictionary, Object, ObjectId};
use crate::stream::StreamLimits;
use crate::stream_object::decode_document_stream_with_usage;

use super::super::info::{PageSlot, TreeNode};
use super::Fixes;

/// The names the view gives the default spaces of the gray, RGB and CMYK
/// families in a /ColorSpace resource dictionary.
const NAMES: [&[u8]; 3] = [b"TPDFDefaultGray", b"TPDFDefaultRGB", b"TPDFDefaultCMYK"];
const KEYS: [&[u8]; 3] = [b"DefaultGray", b"DefaultRGB", b"DefaultCMYK"];
const DEVICE: [&str; 3] = ["DeviceGray", "DeviceRGB", "DeviceCMYK"];
const COMPONENTS: [i64; 3] = [1, 3, 4];
/// Decoded content read and rewritten per view, all streams together.
const MAX_CONTENT_BYTES: usize = 16 << 20;
/// Image and shading stream bytes copied per view to redefine their space.
const MAX_COPIED_BYTES: usize = 32 << 20;
/// Forms and patterns nested in forms.
const MAX_DEPTH: usize = 32;
const MAX_PROFILE_BYTES: usize = 4 << 20;
/// Rewritten content a `Memo` keeps.
const MAX_MEMO_BYTES: usize = 64 << 20;

/// A memo entry: the decoded length, the decode's total output, and the
/// rewritten content when the rewrite changed it.
type Rewrite = (usize, usize, Option<Arc<Vec<u8>>>);

/// Content rewrites kept from one view to the next. A stream's content
/// changes only when a change touches the stream (its filters direct), so
/// a view after an edit rewrites only what the edit touched; each view is
/// what it would be without the memo, budgets included.
#[derive(Default)]
pub(crate) struct Memo {
    /// By stream and the families named.
    content: BTreeMap<(ObjectId, [bool; 3]), Rewrite>,
    bytes: usize,
}

impl Memo {
    /// Forgets the streams among `ids`.
    pub(crate) fn forget(&mut self, ids: impl IntoIterator<Item = ObjectId>) {
        for id in ids {
            let keys: Vec<_> = (self.content.range((id, [false; 3])..=(id, [true; 3])))
                .map(|(key, _)| *key)
                .collect();
            for key in keys {
                if let Some((_, _, Some(data))) = self.content.remove(&key) {
                    self.bytes = self.bytes.saturating_sub(data.len());
                }
            }
        }
    }

    fn keep(&mut self, key: (ObjectId, [bool; 3]), entry: Rewrite) {
        let size = entry.2.as_ref().map_or(0, |data| data.len());
        if self.bytes.saturating_add(size) > MAX_MEMO_BYTES {
            *self = Memo::default();
        }
        self.bytes += size;
        self.content.insert(key, entry);
    }
}

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
    /// The families whose names the rewrite of a content stream without
    /// resources of its own (a page's, a form's or an appearance's) uses
    /// from whatever resources draw it.
    uses: BTreeMap<ObjectId, [bool; 3]>,
    /// The streams `cidfont` shares among fonts.
    pub(super) no_glyphs: Option<ObjectId>,
    pub(super) mac_to_unicode: Option<ObjectId>,
    /// /Annots array objects walked, with the names their appearances use
    /// from a page's resources.
    annots: BTreeMap<ObjectId, [bool; 3]>,
    /// Page resources walked, by where they are written (an object, or in
    /// a page or /Pages node object).
    pages: BTreeMap<(ObjectId, bool), PageColour>,
    /// Images `runlength` looked at.
    pub(super) ended: std::collections::BTreeSet<ObjectId>,
    /// Tiling patterns `tiling` looked at, and the cell copies it drew.
    pub(super) tiled: std::collections::BTreeSet<ObjectId>,
    pub(super) tile_copies: usize,
    /// The resources enclosing the form being walked, outermost first
    /// (`inherit`).
    pub(super) chain: Vec<Arc<Dictionary>>,
    /// Form content `inherit` read and enclosing names it looked up.
    pub(super) scanned: usize,
    pub(super) checked: usize,
    content_bytes: usize,
    copied_bytes: usize,
    pub(super) memo: Memo,
}

/// What a page's resources give what it draws: the defaults, the families
/// they name, and the dictionary itself (`inherit`).
#[derive(Clone)]
struct PageColour {
    defaults: Defaults,
    named: [bool; 3],
    resources: Option<Arc<Dictionary>>,
}

impl Recolor {
    pub(super) fn new(doc: &crate::document::Document<'_>, memo: Memo) -> Self {
        Self {
            intent: output_intent(doc),
            memo,
            ..Self::default()
        }
    }
}

/// The first output intent's /DestOutputProfile (no other is read) as
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

fn either(a: [bool; 3], b: [bool; 3]) -> [bool; 3] {
    std::array::from_fn(|f| a[f] || b[f])
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
    pub(super) fn current(&self, id: ObjectId) -> Option<Object> {
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

    /// The view's value of a non-stream object, borrowed.
    fn view_value(&self, id: ObjectId) -> Option<&Object> {
        match self.objects.get(&id) {
            Some(value) => Some(value),
            None => {
                let object = self.doc.object(id).ok()?;
                object.stream.is_none().then_some(&object.value)
            }
        }
    }

    fn view_dict(&self, id: ObjectId) -> Option<&Dictionary> {
        self.view_value(id).and_then(Object::as_dict)
    }

    /// Walks the resources, content and annotation appearances of a page
    /// written in its own object: device colour when the page has defaults
    /// (resources, content, images, shadings), tiling patterns always
    /// (`tiling`).
    pub(super) fn walk_page(&mut self, slot: PageSlot<'_>, path: &[TreeNode<'d>]) {
        let PageSlot::Object(page_id) = slot else {
            return;
        };
        let Some(dict) = self.view_value(page_id).and_then(Object::as_dict) else {
            return;
        };
        // hayro inherits along the /Kids path; the view wrote the /Parent
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
        let contents = (dict.get(b"Contents".as_slice())).cloned();
        let annots = dict.get(b"Annots".as_slice()).cloned();
        let page = self.page_colour(holder.as_ref(), page_id);
        // Every content stream is mapped (or found mapped) whatever the page
        // names: a stream several pages share keeps its first rewrite, whose
        // names each page then defines.
        let mut needed = [false; 3];
        for id in self.stream_list(contents.as_ref()) {
            let uses = self.map_stream(id, &page.defaults, page.named, Kind::Content, 0);
            needed = either(needed, uses);
        }
        if let Some(annots) = annots {
            needed = either(needed, self.walk_appearances(&annots, &page));
        }
        let missing = std::array::from_fn(|f| needed[f] && !page.named[f]);
        if missing.iter().any(|&m| m) {
            let target = holder.unwrap_or(Holder::In(page_id));
            self.name_at(target, &page.defaults, missing);
        }
    }

    /// The resources a page draws with, mapped once per place they are
    /// written; a page without any gets the names alone.
    fn page_colour(&mut self, holder: Option<&Holder>, page_id: ObjectId) -> PageColour {
        let key = match holder {
            Some(Holder::Object(id)) => Some((*id, false)),
            Some(Holder::In(id)) => Some((*id, true)),
            None => None,
        };
        if let Some(page) = key.and_then(|key| self.recolor.pages.get(&key)) {
            return page.clone();
        }
        let resources = match holder {
            Some(Holder::Object(id)) => self.view_value(*id).and_then(Object::as_dict),
            Some(Holder::In(id)) => (self.view_value(*id).and_then(Object::as_dict))
                .and_then(|holder| holder.get(b"Resources".as_slice()))
                .and_then(Object::as_dict),
            None => None,
        };
        let own =
            (resources.map(|resources| self.resource_defaults(resources))).unwrap_or_default();
        let resources = resources.cloned().map(Arc::new);
        let defaults = own.over(&self.recolor.intent);
        let named = match holder {
            Some(Holder::Object(id)) => self.map_resources_object(*id, &defaults, 0),
            Some(Holder::In(id)) => self.map_resources_in(*id, &defaults),
            None if !defaults.is_empty() => self.map_resources_in(page_id, &defaults),
            None => [false; 3],
        };
        let page = PageColour {
            defaults,
            named,
            resources,
        };
        if let Some(key) = key {
            self.recolor.pages.insert(key, page.clone());
        }
        page
    }

    /// Maps the normal appearances of a page's annotations, which draw with
    /// the page's resources when they have none; an /Annots array object
    /// several pages share is walked once. The families whose names they
    /// use from the page's resources.
    fn walk_appearances(&mut self, annots: &Object, page: &PageColour) -> [bool; 3] {
        if let Object::Reference(array) = annots {
            if let Some(&uses) = self.recolor.annots.get(array) {
                return uses;
            }
        }
        let items = (self.view_resolve(annots).as_array().unwrap_or_default()).to_vec();
        // An appearance's own resources fall back on the page's (`inherit`).
        let chained = page.resources.is_some();
        self.recolor.chain.extend(page.resources.clone());
        let mut needed = [false; 3];
        for item in items {
            let appearances = match self.view_resolve(&item) {
                Object::Dictionary(annot) => (annot.get(b"AP".as_slice()))
                    .and_then(|ap| self.view_resolve(ap).as_dict())
                    .and_then(|ap| ap.get(b"N".as_slice()))
                    .map(|normal| self.stream_list(Some(normal))),
                _ => None,
            };
            for id in appearances.unwrap_or_default() {
                let uses = self.map_stream(id, &page.defaults, page.named, Kind::Form, 1);
                needed = either(needed, uses);
            }
        }
        if chained {
            self.recolor.chain.pop();
        }
        if let Object::Reference(array) = annots {
            self.recolor.annots.insert(*array, needed);
        }
        needed
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

    /// Defines the names of the families `missing` in the resources at
    /// `holder` (written directly in a page or node when `Holder::In`,
    /// created when absent): each family's default, else its device space.
    fn name_at(&mut self, holder: Holder, defaults: &Defaults, missing: [bool; 3]) {
        let (id, direct) = match holder {
            Holder::Object(id) => (id, false),
            Holder::In(id) => (id, true),
        };
        // Pages sharing the resources find the names there after the first.
        let resources = match direct {
            false => self.view_dict(id),
            true => (self
                .view_dict(id)
                .and_then(|holder| holder.get(b"Resources".as_slice())))
            .and_then(Object::as_dict),
        };
        let spaces = (resources.and_then(|r| r.get(b"ColorSpace".as_slice())))
            .and_then(|spaces| self.view_resolve(spaces).as_dict());
        if (0..3).all(|f| !missing[f] || spaces.is_some_and(|s| s.contains_key(NAMES[f]))) {
            return;
        }
        let Some(mut value) = self.current(id) else {
            return;
        };
        let Some(dict) = value.as_dict_mut() else {
            return;
        };
        let changed = match direct {
            false => self.name_spaces(dict, defaults, missing, false),
            true => {
                let mut resources = match dict.remove(b"Resources".as_slice()) {
                    Some(Object::Dictionary(resources)) => resources,
                    _ => Dictionary::new(),
                };
                self.name_spaces(&mut resources, defaults, missing, false);
                dict.insert(b"Resources".to_vec(), Object::Dictionary(resources));
                true
            }
        };
        if changed {
            self.objects.insert(id, value);
        }
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
        self.names_in(self.view_value(id).and_then(Object::as_dict), defaults)
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
        let mut changed = self.name_spaces(resources, defaults, [false; 3], true);
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
            self.recolor.chain.push(Arc::new(resources.clone()));
            let mut needed = [false; 3];
            for id in xobjects.unwrap_or_default() {
                match (self.stream_dict(id)).map(|d| self.doc.dict_get(d, b"Subtype")) {
                    Some(Object::Name(kind)) if kind == b"Form" => {
                        let uses = self.map_stream(id, defaults, named, Kind::Form, depth + 1);
                        needed = either(needed, uses);
                    }
                    Some(Object::Name(kind)) if kind == b"Image" => {
                        self.fix_run_length(id);
                        if colour {
                            self.map_image(id, defaults);
                        }
                    }
                    _ => {}
                }
            }
            self.recolor.chain.pop();
            // A form without resources rewritten where other resources
            // drew it first finds its names here too.
            let missing = std::array::from_fn(|f| needed[f] && !named[f]);
            if missing.iter().any(|&m| m) {
                changed |= self.name_spaces(resources, defaults, missing, false);
            }
            let fonts = (resources.get(b"Font".as_slice()))
                .and_then(|f| self.view_resolve(f).as_dict())
                .map(|f| f.values().cloned().collect::<Vec<_>>());
            for font in fonts.unwrap_or_default() {
                self.fix_cmap_wmode(&font);
                self.fix_cid_fallback(&font);
            }
            if let Some(holder) = resources.get_mut(b"ExtGState".as_slice()) {
                changed |= self.map_gstates(holder);
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

    /// Defines in `resources`' /ColorSpace the name of each family that
    /// `defaults` has, as its default, and of each family in `also`, as its
    /// default or else its device space, where the name is not there yet;
    /// with `entries`, also maps the device spaces its entries name
    /// (`mapped_entry`). A /ColorSpace object is mapped once. Whether
    /// `resources` changed.
    fn name_spaces(
        &mut self,
        resources: &mut Dictionary,
        defaults: &Defaults,
        also: [bool; 3],
        entries: bool,
    ) -> bool {
        let reference = match resources.get(b"ColorSpace".as_slice()) {
            Some(Object::Reference(id)) => Some(*id).filter(|id| self.view_dict(*id).is_some()),
            _ => None,
        };
        // A /ColorSpace object keeps the entries of the first resources
        // that reach it, defaults or not.
        let first = reference.is_some_and(|id| self.recolor.done.insert(id));
        let wanted: [bool; 3] = std::array::from_fn(|f| defaults.0[f].is_some() || also[f]);
        if !wanted.iter().any(|&w| w) {
            return false;
        }
        let space = |family: usize| match &defaults.0[family] {
            Some(space) => space.clone(),
            None => Object::name(DEVICE[family]),
        };
        let fill = |fixes: &Self, spaces: &mut Dictionary, entries: bool| {
            let mut changed = false;
            if entries && !defaults.is_empty() {
                let keys: Vec<Vec<u8>> = spaces.keys().cloned().collect();
                for key in keys {
                    if NAMES.contains(&key.as_slice()) || KEYS.contains(&key.as_slice()) {
                        continue;
                    }
                    let mapped = (spaces.get(key.as_slice()))
                        .and_then(|value| fixes.mapped_entry(value, defaults));
                    if let Some(mapped) = mapped {
                        spaces.insert(key, mapped);
                        changed = true;
                    }
                }
            }
            for family in 0..3 {
                if wanted[family] && !spaces.contains_key(NAMES[family]) {
                    spaces.insert(NAMES[family].to_vec(), space(family));
                    changed = true;
                }
            }
            changed
        };
        if let Some(id) = reference {
            let spaces = self.view_dict(id);
            let lacking =
                (0..3).any(|f| wanted[f] && !spaces.is_some_and(|s| s.contains_key(NAMES[f])));
            if !(lacking || (first && entries)) {
                return false;
            }
            if let Some(mut value) = self.current(id) {
                if value
                    .as_dict_mut()
                    .is_some_and(|spaces| fill(self, spaces, first && entries))
                {
                    self.objects.insert(id, value);
                }
            }
            return false;
        }
        match resources.get_mut(b"ColorSpace".as_slice()) {
            Some(Object::Dictionary(spaces)) => fill(self, spaces, entries),
            _ => {
                let mut spaces = Dictionary::new();
                fill(self, &mut spaces, false);
                resources.insert(b"ColorSpace".to_vec(), Object::Dictionary(spaces));
                true
            }
        }
    }

    /// A /ColorSpace resource naming a device space (directly or by
    /// reference), or a pattern space over one, with the device space
    /// replaced by its family's default; None when it has none. Reference
    /// viewers map these, but not an Indexed base or a Separation or DeviceN
    /// alternate that `cs` selects.
    fn mapped_entry(&self, value: &Object, defaults: &Defaults) -> Option<Object> {
        match self.view_resolve(value) {
            Object::Name(name) => defaults.0[device_family(name)?].clone(),
            Object::Array(items) if items.len() == 2 && items[0].as_name() == Some(b"Pattern") => {
                let family = device_family(self.view_resolve(&items[1]).as_name()?)?;
                let base = defaults.0[family].clone()?;
                Some(Object::Array(vec![items[0].clone(), base]))
            }
            _ => None,
        }
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
                let mapped = self.mapped_space(self.doc.dict_get(dict, b"ColorSpace"), defaults);
                if mapped.is_none() || self.copy_budget(data.len()).is_none() {
                    return;
                }
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
    /// own draws with. The families whose names its rewrite uses from the
    /// resources that draw it (none when it has its own).
    fn map_stream(
        &mut self,
        id: ObjectId,
        defaults: &Defaults,
        inherited: [bool; 3],
        kind: Kind,
        depth: usize,
    ) -> [bool; 3] {
        if depth > MAX_DEPTH {
            return [false; 3];
        }
        if !self.recolor.done.insert(id) {
            return self.recolor.uses.get(&id).copied().unwrap_or_default();
        }
        let Some(mut dict) = self.stream_dict(id).cloned() else {
            return [false; 3];
        };
        // A page content stream's dictionary holds no resources; a pattern
        // without its own has none (hayro, reference viewers).
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
        let (named, from_caller) = match resources {
            Some(Object::Reference(resources)) => (
                self.map_resources_object(resources, &defaults, depth),
                false,
            ),
            Some(Object::Dictionary(mut resources)) => {
                let (named, edited) = self.map_resources(&mut resources, &defaults, depth);
                changed |= edited;
                if changed {
                    dict.insert(b"Resources".to_vec(), Object::Dictionary(resources));
                }
                (named, false)
            }
            _ if kind == Kind::Pattern => ([false; 3], false),
            _ => (inherited, true),
        };
        if !changed && !named.iter().any(|&n| n) {
            return [false; 3];
        }
        let Some(data) = self.content_for(id, named, changed) else {
            return [false; 3];
        };
        for key in [b"Filter".as_slice(), b"DecodeParms", b"DL"] {
            dict.remove(key);
        }
        self.streams.insert(id, (dict, data));
        if !from_caller {
            return [false; 3];
        }
        self.recolor.uses.insert(id, named);
        named
    }

    /// What a redefinition of content stream `id` holds: its content with
    /// the colour operators of the families `named` rewritten (`rewrite`,
    /// kept in the memo). None when the content cannot be read within the
    /// budget, or when the rewrite changes nothing and `always` is false.
    fn content_for(&mut self, id: ObjectId, named: [bool; 3], always: bool) -> Option<Vec<u8>> {
        let key = (id, named);
        let kept = match self.recolor.memo.content.get(&key) {
            Some(entry) if self.memoizable(id) => Some(entry.clone()),
            _ => None,
        };
        let redefined = self.streams.get(&id).map(|(_, data)| data.clone());
        // The rewrite, the content when read, and the content's length.
        let (rewritten, content, length) = match (kept, redefined) {
            // Another fix's bytes, read without a charge.
            (_, Some(content)) => {
                let length = content.len();
                (
                    rewrite(&content, named).map(Arc::new),
                    Some(content),
                    length,
                )
            }
            (Some((length, total, rewritten)), None) => {
                self.charge(length, total)?;
                (rewritten, None, length)
            }
            (None, None) => {
                let memoizable = self.memoizable(id);
                let (content, total) = self.decode(id)?;
                self.charge(content.len(), total)?;
                let rewritten = rewrite(&content, named).map(Arc::new);
                if memoizable {
                    let entry = (content.len(), total, rewritten.clone());
                    self.recolor.memo.keep(key, entry);
                }
                let length = content.len();
                (rewritten, Some(content), length)
            }
        };
        match rewritten {
            Some(data) => {
                // What the rewrite adds counts as read content.
                let grown = data.len().saturating_sub(length);
                self.recolor.content_bytes = self.recolor.content_bytes.saturating_add(grown);
                Some(data.to_vec())
            }
            None if !always => None,
            None => match content {
                Some(content) => Some(content),
                None => Some(self.decode(id)?.0),
            },
        }
    }

    /// Whether the memo may keep stream `id`: decoded from its own bytes
    /// with filters written in its dictionary.
    fn memoizable(&self, id: ObjectId) -> bool {
        let direct = |dict: &Dictionary| {
            [b"Filter".as_slice(), b"DecodeParms"]
                .iter()
                .all(|key| !matches!(dict.get(*key), Some(Object::Reference(_))))
        };
        !self.streams.contains_key(&id) && self.doc.stream_dict(id).is_ok_and(direct)
    }

    /// Stream `id` decoded within what the content budget has left, with
    /// the decode's total output; nothing charged.
    fn decode(&self, id: ObjectId) -> Option<(Vec<u8>, usize)> {
        let left = MAX_CONTENT_BYTES.checked_sub(self.recolor.content_bytes)?;
        let limits = StreamLimits {
            max_decoded_bytes: left,
            max_total_decoded_bytes: left.saturating_mul(2),
            ..StreamLimits::default()
        };
        let decoded = decode_document_stream_with_usage(self.doc, id, limits).ok()?;
        Some((decoded.bytes, decoded.usage.total_decoded_bytes))
    }

    /// Charges `length` stream bytes copied into the view to its copy
    /// budget; None past it.
    pub(super) fn copy_budget(&mut self, length: usize) -> Option<()> {
        let total = self.recolor.copied_bytes.checked_add(length)?;
        if total > MAX_COPIED_BYTES {
            return None;
        }
        self.recolor.copied_bytes = total;
        Some(())
    }

    /// Charges a decode of `length` bytes and `total` output to the content
    /// budget; None when `decode` would have refused it.
    fn charge(&mut self, length: usize, total: usize) -> Option<()> {
        let left = MAX_CONTENT_BYTES.checked_sub(self.recolor.content_bytes)?;
        if length > left || total > left.saturating_mul(2) {
            return None;
        }
        self.recolor.content_bytes += length.max(1);
        Some(())
    }

    /// A stream's decoded bytes within the content budget.
    pub(super) fn decoded(&mut self, id: ObjectId) -> Option<Vec<u8>> {
        if let Some((_, data)) = self.streams.get(&id) {
            return Some(data.clone());
        }
        let (data, total) = self.decode(id)?;
        self.charge(data.len(), total)?;
        Some(data)
    }
}

/// `content` with its device colour operators (`g`, `rg`, `k`, their
/// stroking forms, `cs`/`CS` of a device space) and inline images of the
/// families `named` set using the default space's resource name; None when
/// nothing changes. A colour operator sets the named space only when the
/// content has not already set it (as far as `q` and `Q` tell), else just
/// the colour, so the content barely grows. What follows a syntax error the
/// content parser stops at is kept as it is.
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
    let mut spaces = Spaces::default();
    while let Ok(Some(op)) = parser.next_operation() {
        let Some(replacement) = spaces.replacement(content, &op, named) else {
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

/// The colour space of fill and of stroke as far as the rewrite knows it.
#[derive(Clone, Copy, PartialEq)]
enum Space {
    Unknown,
    /// The default space of a family, by its resource name.
    Named(usize),
    Other,
}

/// The rewrite's view of the graphics state: fill and stroke spaces, saved
/// by `q` (up to a depth; past it a `Q` restores nothing known). hayro lets
/// a form's surplus `Q` restore its caller's states, so after `Do` neither
/// the current spaces nor those saved before it are known.
struct Spaces {
    current: [Space; 2],
    saved: Vec<[Space; 2]>,
    lost: usize,
    /// Saved states below this depth were saved before a `Do`.
    stale: usize,
}

impl Default for Spaces {
    fn default() -> Self {
        Self {
            current: [Space::Unknown; 2],
            saved: Vec::new(),
            lost: 0,
            stale: 0,
        }
    }
}

const MAX_SAVED: usize = 4096;

impl Spaces {
    /// The bytes replacing one operation, if it uses a mapped device space.
    fn replacement(
        &mut self,
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
            b"q" => {
                match self.saved.len() < MAX_SAVED {
                    true => self.saved.push(self.current),
                    false => self.lost += 1,
                }
                return None;
            }
            b"Q" => {
                self.current = match self.lost.checked_sub(1) {
                    Some(lost) => {
                        self.lost = lost;
                        [Space::Unknown; 2]
                    }
                    None => match self.saved.pop() {
                        Some(saved) if self.saved.len() >= self.stale => saved,
                        _ => [Space::Unknown; 2],
                    },
                };
                self.stale = self.stale.min(self.saved.len());
                return None;
            }
            b"Do" => {
                self.current = [Space::Unknown; 2];
                self.stale = self.saved.len();
                return None;
            }
            b"g" => (0, false),
            b"G" => (0, true),
            b"rg" => (1, false),
            b"RG" => (1, true),
            b"k" => (2, false),
            b"K" => (2, true),
            b"cs" | b"CS" => {
                let side = usize::from(op.operator == b"CS");
                let family = match op.operands.as_slice() {
                    [ContentOperand::Name(name)] => device_family(name).filter(|&f| named[f]),
                    _ => None,
                };
                let Some(family) = family else {
                    self.current[side] = Space::Other;
                    return None;
                };
                self.current[side] = Space::Named(family);
                let mut out = vec![b'/'];
                out.extend_from_slice(NAMES[family]);
                out.push(b' ');
                out.extend_from_slice(&op.operator);
                return Some(out);
            }
            b"BI" => return inline_image(content, op, named),
            _ => return None,
        };
        let side = usize::from(stroke);
        if !named[family] || !numbers(COMPONENTS[family]) {
            self.current[side] = match numbers(COMPONENTS[family]) {
                true => Space::Other,
                false => Space::Unknown,
            };
            return None;
        }
        let operands = content.get(op.span.start..op.span.end.checked_sub(op.operator.len())?)?;
        let (set, paint) = match stroke {
            true => (b" CS ", b"SC"),
            false => (b" cs ", b"sc"),
        };
        let mut out = Vec::new();
        if self.current[side] != Space::Named(family) {
            out.push(b'/');
            out.extend_from_slice(NAMES[family]);
            out.extend_from_slice(set);
            self.current[side] = Space::Named(family);
        }
        // As written: a comment may end the operands, closed by its line end.
        out.extend_from_slice(operands);
        if !operands.last().is_some_and(u8::is_ascii_whitespace) {
            out.push(b' ');
        }
        out.extend_from_slice(paint);
        Some(out)
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rewritten(content: &str, named: [bool; 3]) -> String {
        let out = rewrite(content.as_bytes(), named).unwrap_or_default();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn a_colour_operator_sets_the_named_space_only_when_it_is_not_set() {
        let rgb = [false, true, false];
        assert_eq!(
            rewritten("1 0 0 rg 0 1 0 rg 0 0 1 RG 1 1 0 RG", rgb),
            "/TPDFDefaultRGB cs 1 0 0 sc 0 1 0 sc /TPDFDefaultRGB CS 0 0 1 SC 1 1 0 SC"
        );
        // Another space in between, or a restore past what was saved.
        assert_eq!(
            rewritten("1 0 0 rg 0.5 g 0 1 0 rg /P cs 0 0 1 rg Q 1 1 1 rg", rgb),
            "/TPDFDefaultRGB cs 1 0 0 sc 0.5 g /TPDFDefaultRGB cs 0 1 0 sc /P cs \
             /TPDFDefaultRGB cs 0 0 1 sc Q /TPDFDefaultRGB cs 1 1 1 sc"
        );
        // Q restores what q saved.
        assert_eq!(
            rewritten("q 1 0 0 rg q 0.5 g Q 0 1 0 rg Q 0 0 1 rg", rgb),
            "q /TPDFDefaultRGB cs 1 0 0 sc q 0.5 g Q 0 1 0 sc Q \
             /TPDFDefaultRGB cs 0 0 1 sc"
        );
        // A device space set by name is the named one; malformed operands
        // leave the space unknown.
        assert_eq!(
            rewritten("/DeviceRGB cs 1 0 0 sc 0 1 0 rg 1 rg 0 0 1 rg", rgb),
            "/TPDFDefaultRGB cs 1 0 0 sc 0 1 0 sc 1 rg /TPDFDefaultRGB cs 0 0 1 sc"
        );
        assert_eq!(rewritten("0.5 g 0 0 0 1 k", rgb), "");
    }

    #[test]
    fn a_comment_before_the_operator_keeps_its_line_end() {
        let rgb = [false, true, false];
        assert_eq!(
            rewritten("1 0 0 % a comment\nrg 0 0 50 50 re f", rgb),
            "/TPDFDefaultRGB cs 1 0 0 % a comment\nsc 0 0 50 50 re f"
        );
        assert_eq!(
            rewritten("0 % c\r0 1 % d\r\nRG", rgb),
            "/TPDFDefaultRGB CS 0 % c\r0 1 % d\r\nSC"
        );
    }

    #[test]
    fn after_a_form_no_space_is_known() {
        // A form's surplus Q may restore its caller's states in hayro.
        let rgb = [false, true, false];
        assert_eq!(
            rewritten("q 1 0 0 rg q /Fm Do 0 1 0 rg Q 0 0 1 rg Q 1 1 1 rg", rgb),
            "q /TPDFDefaultRGB cs 1 0 0 sc q /Fm Do /TPDFDefaultRGB cs 0 1 0 sc Q \
             /TPDFDefaultRGB cs 0 0 1 sc Q /TPDFDefaultRGB cs 1 1 1 sc"
        );
        // States saved after it are known again.
        assert_eq!(
            rewritten("/Fm Do 1 0 0 rg q 0.5 g Q 0 1 0 rg", rgb),
            "/Fm Do /TPDFDefaultRGB cs 1 0 0 sc q 0.5 g Q 0 1 0 sc"
        );
    }

    #[test]
    fn saved_states_past_the_depth_restore_nothing_known() {
        let mut content = "1 0 0 rg ".to_string();
        content.push_str(&"q ".repeat(MAX_SAVED + 1));
        content.push_str("0 1 0 rg Q 0 0 1 rg");
        let out = rewritten(&content, [false, true, false]);
        assert!(
            out.ends_with("q 0 1 0 sc Q /TPDFDefaultRGB cs 0 0 1 sc"),
            "{}",
            &out[out.len() - 80..]
        );
    }

    #[test]
    fn the_memo_forgets_every_rewrite_of_a_stream() {
        let id = |n| ObjectId::new(n, 0).unwrap();
        let mut memo = Memo::default();
        let data = Some(Arc::new(vec![0; 10]));
        memo.keep((id(3), [false, true, false]), (10, 10, data.clone()));
        memo.keep((id(3), [true, true, true]), (10, 10, data.clone()));
        memo.keep((id(4), [false; 3]), (10, 10, data));
        assert_eq!(memo.bytes, 30);
        memo.forget([id(3), id(9)]);
        assert_eq!(memo.content.len(), 1);
        assert_eq!(memo.bytes, 10);
    }
}
