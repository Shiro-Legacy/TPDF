// SPDX-License-Identifier: MIT
//! The template packet read into the `types` model: `subform`, `field`,
//! `draw`, `exclGroup`, `area`, `pageSet`/`pageArea`/`contentArea`, with
//! `proto` definitions resolved through `use="#id"` / `usehref` (same
//! packet only), `event`/`script` and `variables` kept as text for
//! `script` to run, unknown elements ignored. Occurrences are not expanded
//! here (`bind` does).
//!
//! Rules it follows:
//! - Every length goes through `types::length`; one that does not read is
//!   absent. `maxW`/`maxH` of 0 (XFA's "no maximum") are absent too.
//! - A prototype supplies defaults: the referencing element's attributes
//!   win, and each of its property children is merged over the
//!   prototype's same-named child (attribute by attribute, recursively);
//!   container children (`field`, `draw`, `subform`, ...) are appended
//!   after the prototype's. References are followed to a depth of 16 and
//!   a cycle stops at the element that closes it; the elements copied by
//!   expansion are bounded by `limits::MAX_XML_ELEMENTS`.
//! - `subformSet` is flattened into its parent. `pageSet`s anywhere in the
//!   tree contribute their `pageArea`s in document order.
//! - `access` on a subform or exclGroup makes everything inside read-only
//!   too (XFA cascades it).
//! - A caption without a `font` takes its field's font (what Designer
//!   means by leaving it out).
//! - Containers nest at most `MAX_NESTING` deep and the form holds at
//!   most `limits::MAX_FORM_NODES` nodes (`XfaError::Limit`).

use std::borrow::Cow;
use std::collections::HashMap;

use super::XfaError;
use super::limits::{MAX_FORM_NODES, MAX_XML_ELEMENTS};
use super::types::{
    Anchor, Bind, BindMatch, Border, Break, Caption, CheckMark, CheckShape, ChoiceOpen, Common,
    Draw, DrawUi, Edge, Event, ExclGroup, Field, Fill, Font, Form, HAlign, Item, Keep, Layout,
    Margin, Node, Occur, PageArea, PageSet, Para, Picture, Placement, Presence, Pt, Rect, Rgb,
    Stroke, Subform, Ui, VAlign, Value, ValueKind, Variable, length,
};
use super::xml::{self, Element, XmlNode};

/// Containers (subforms, areas, subform sets, groups) nested inside each
/// other, at most. Real forms stay under 30.
pub const MAX_NESTING: usize = 256;
/// `use` references followed in a chain, at most.
const MAX_PROTO_DEPTH: usize = 16;
/// Events kept per node, and variables per subform, at most.
const MAX_EVENTS: usize = 64;
const MAX_VARIABLES: usize = 256;
/// A script (an event's or a script object's) longer than this is not
/// kept: `script` would not run it.
pub const MAX_SCRIPT_BYTES: usize = 256 << 10;

/// Element names that are containers (appended, not merged, by prototype
/// expansion) rather than properties.
const CONTAINERS: &[&str] = &[
    "subform",
    "subformSet",
    "field",
    "draw",
    "exclGroup",
    "area",
    "pageSet",
    "pageArea",
    "contentArea",
    "proto",
    "event",
    "script",
    "variables",
];

/// Reads a template packet. The result's `root` holds one instance of
/// every subform as written; `bind::bind` expands it.
pub fn read(template_xml: &str) -> Result<Form, XfaError> {
    let doc = xml::parse(template_xml).map_err(|e| XfaError::Xml(format!("template: {e}")))?;
    let template = if doc.local() == "template" {
        &doc
    } else {
        doc.child("template")
            .ok_or_else(|| XfaError::Unsupported("no template element".into()))?
    };
    let mut reader = Reader::new(template);
    let root_element = template
        .child("subform")
        .ok_or_else(|| XfaError::Unsupported("no root subform".into()))?;
    let root_element = reader.resolve(root_element);
    let root = reader.subform(&root_element, false, false, 0)?;
    if reader.over_budget {
        return Err(XfaError::Limit("template prototype expansion"));
    }
    let locale = template
        .attr("locale")
        .or_else(|| root_element.attr("locale"))
        .filter(|l| !l.is_empty())
        .unwrap_or("en_US")
        .to_string();
    let mut page_set = PageSet {
        areas: reader.areas,
    };
    if page_set.areas.is_empty() {
        page_set.areas.push(PageArea::default());
    }
    Ok(Form {
        page_set,
        root,
        locale,
        ids: Default::default(),
    })
}

struct Reader<'x> {
    /// Every element with an `id`, the first of each.
    ids: HashMap<&'x str, &'x Element>,
    /// Form nodes read.
    nodes: usize,
    /// Elements copied by prototype expansion.
    copied: usize,
    over_budget: bool,
    /// Page masters, from every `pageSet` in document order.
    areas: Vec<PageArea>,
}

impl<'x> Reader<'x> {
    fn new(template: &'x Element) -> Self {
        let mut ids = HashMap::new();
        let mut stack = vec![template];
        while let Some(e) = stack.pop() {
            if let Some(id) = e.attr("id").filter(|id| !id.is_empty()) {
                ids.entry(id).or_insert(e);
            }
            // Reverse, so the first in document order is seen first.
            let start = stack.len();
            stack.extend(e.elements());
            stack[start..].reverse();
        }
        Reader {
            ids,
            nodes: 0,
            copied: 0,
            over_budget: false,
            areas: Vec::new(),
        }
    }

    fn count_node(&mut self) -> Result<(), XfaError> {
        self.nodes += 1;
        if self.nodes > MAX_FORM_NODES {
            return Err(XfaError::Limit("template form nodes"));
        }
        Ok(())
    }

    /// `e` with its prototype (`use`/`usehref`) applied; `e` itself when it
    /// has none.
    fn resolve<'a>(&mut self, e: &'a Element) -> Cow<'a, Element> {
        if reference(e).is_none() {
            return Cow::Borrowed(e);
        }
        let mut chain = Vec::new();
        Cow::Owned(self.expand(e, &mut chain))
    }

    fn expand(&mut self, e: &Element, chain: &mut Vec<String>) -> Element {
        let base = match reference(e) {
            Some(id) if chain.len() < MAX_PROTO_DEPTH && !chain.iter().any(|c| c == id) => {
                match self.ids.get(id).copied() {
                    Some(target) => {
                        chain.push(id.to_string());
                        let base = self.expand(target, chain);
                        chain.pop();
                        Some(base)
                    }
                    None => None,
                }
            }
            _ => None,
        };
        self.copied = self.copied.saturating_add(element_count(e));
        if self.copied > MAX_XML_ELEMENTS {
            self.over_budget = true;
            return Element {
                name: e.name.clone(),
                ..Default::default()
            };
        }
        let mut own = e.clone();
        own.attrs
            .retain(|(k, _)| !matches!(local_of(k), "use" | "usehref"));
        match base {
            Some(mut base) => {
                base.attrs.retain(|(k, _)| local_of(k) != "id");
                merge(base, own)
            }
            None => own,
        }
    }

    /// The first child named `local`, its prototype applied.
    fn prop<'a>(&mut self, e: &'a Element, local: &str) -> Option<Cow<'a, Element>> {
        let child = e.child(local)?;
        Some(self.resolve(child))
    }

    fn margin(&mut self, e: &Element) -> Margin {
        self.prop(e, "margin")
            .map(|m| read_margin(&m))
            .unwrap_or_default()
    }

    fn border(&mut self, e: &Element) -> Border {
        match self.prop(e, "border") {
            Some(b) => self.border_of(&b),
            None => Border::none(),
        }
    }

    /// A `border` (or `rectangle`): edges, corners, fill, hand.
    fn border_of(&mut self, b: &Element) -> Border {
        let mut edges: Vec<Edge> = Vec::new();
        for edge in b.children_named("edge") {
            let edge = self.resolve(edge);
            edges.push(read_edge(&edge));
        }
        let mut four = [Edge::default(); 4];
        if let Some(&last) = edges.last() {
            for (i, slot) in four.iter_mut().enumerate() {
                *slot = edges.get(i).copied().unwrap_or(last);
            }
        }
        let corner_radius = self
            .prop(b, "corner")
            .and_then(|c| c.attr("radius").and_then(length))
            .filter(|r| *r > 0.0)
            .unwrap_or(0.0);
        let fill = self.prop(b, "fill").and_then(|f| read_fill(&f));
        Border {
            presence: presence(b),
            edges: four,
            fill,
            corner_radius,
            inside: b.attr("hand") != Some("right"),
        }
    }

    fn font(&mut self, e: &Element, default: &Font) -> Font {
        match self.prop(e, "font") {
            Some(f) => read_font(&f, default),
            None => default.clone(),
        }
    }

    fn para(&mut self, e: &Element) -> Para {
        self.prop(e, "para")
            .map(|p| read_para(&p))
            .unwrap_or_default()
    }

    fn bind_of(&mut self, e: &Element) -> Bind {
        let Some(b) = self.prop(e, "bind") else {
            return Bind::default();
        };
        let matching = match b.attr("match") {
            Some("global") => BindMatch::Global,
            Some("none") => BindMatch::None,
            Some("dataRef") => BindMatch::DataRef,
            _ => BindMatch::Once,
        };
        Bind {
            matching,
            reference: b.attr("ref").unwrap_or("").trim().to_string(),
        }
    }

    fn common(&mut self, e: &Element, read_only: bool) -> Common {
        let len = |name: &str| e.attr(name).and_then(length);
        let positive_max = |name: &str| len(name).filter(|v| *v > 0.0);
        let mut common = Common {
            name: e.attr("name").unwrap_or("").to_string(),
            id: e.attr("id").unwrap_or("").to_string(),
            x: len("x").unwrap_or(0.0),
            y: len("y").unwrap_or(0.0),
            anchor: Anchor::parse(e.attr("anchorType").unwrap_or("")),
            w: len("w"),
            h: len("h"),
            min_w: len("minW"),
            min_h: len("minH"),
            max_w: positive_max("maxW"),
            max_h: positive_max("maxH"),
            presence: presence(e),
            screen_only: e
                .attr("relevant")
                .is_some_and(|r| r.split_whitespace().any(|t| t == "-print")),
            col_span: match e.attr("colSpan").and_then(|v| v.trim().parse::<i64>().ok()) {
                Some(n) if n < 0 => 0,
                Some(n) => u32::try_from(n).unwrap_or(u32::MAX).max(1),
                None => 1,
            },
            rotate: e
                .attr("rotate")
                .and_then(|v| v.trim().parse::<i64>().ok())
                .map_or(0, |r| i32::try_from(r.rem_euclid(360)).unwrap_or(0)),
            read_only: read_only || is_read_only(e),
            ..Common::default()
        };
        common.margin = self.margin(e);
        common.border = self.border(e);
        if let Some(keep) = self.prop(e, "keep") {
            let set = |name: &str| matches!(keep.attr(name), Some("contentArea" | "pageArea"));
            common.keep = Keep {
                intact: set("intact"),
                previous: set("previous"),
                next: set("next"),
            };
        }
        if let Some(b) = self.prop(e, "breakBefore") {
            common.break_before = break_target(b.attr("targetType"));
        }
        if let Some(b) = self.prop(e, "breakAfter") {
            common.break_after = break_target(b.attr("targetType"));
        }
        if let Some(b) = self.prop(e, "break") {
            common.break_before = common.break_before.or(break_target(b.attr("before")));
            common.break_after = common.break_after.or(break_target(b.attr("after")));
        }
        if let Some(assist) = self.prop(e, "assist") {
            common.tooltip = assist
                .child("toolTip")
                .map(|t| t.text().trim().to_string())
                .unwrap_or_default();
            common.speak = assist
                .child("speak")
                .map(|t| t.text().trim().to_string())
                .unwrap_or_default();
        }
        common
    }

    fn subform(
        &mut self,
        e: &Element,
        read_only: bool,
        is_area: bool,
        depth: usize,
    ) -> Result<Subform, XfaError> {
        if depth > MAX_NESTING {
            return Err(XfaError::Limit("template nesting"));
        }
        self.count_node()?;
        let common = self.common(e, read_only);
        let read_only = common.read_only;
        let layout = if is_area {
            Layout::Position
        } else {
            Layout::parse(e.attr("layout").unwrap_or(""))
        };
        let column_widths = e
            .attr("columnWidths")
            .map(|w| {
                w.split_whitespace()
                    .map(|v| length(v).unwrap_or(-1.0))
                    .collect()
            })
            .unwrap_or_default();
        let occur = match self.prop(e, "occur") {
            Some(o) => read_occur(&o, Occur::default()),
            None => Occur::default(),
        };
        let bind = self.bind_of(e);
        let events = self.events(e);
        let variables = self.variables(e);
        let mut children = Vec::new();
        self.children(e, read_only, depth, &mut children)?;
        Ok(Subform {
            common,
            layout,
            column_widths,
            children,
            bind,
            occur,
            is_area,
            events,
            variables,
            ..Subform::default()
        })
    }

    /// The form nodes inside a subform, area or subform set (flattened).
    fn children(
        &mut self,
        e: &Element,
        read_only: bool,
        depth: usize,
        out: &mut Vec<Node>,
    ) -> Result<(), XfaError> {
        for child in e.elements() {
            if !matches!(
                child.local(),
                "subform" | "area" | "subformSet" | "field" | "draw" | "exclGroup" | "pageSet"
            ) {
                continue;
            }
            let child = self.resolve(child);
            match child.local() {
                "subform" => out.push(Node::Subform(self.subform(
                    &child,
                    read_only,
                    false,
                    depth + 1,
                )?)),
                "area" => out.push(Node::Subform(self.subform(
                    &child,
                    read_only,
                    true,
                    depth + 1,
                )?)),
                "subformSet" => {
                    if depth + 1 > MAX_NESTING {
                        return Err(XfaError::Limit("template nesting"));
                    }
                    let read_only = read_only || is_read_only(&child);
                    self.children(&child, read_only, depth + 1, out)?;
                }
                "field" => out.push(Node::Field(self.field(&child, read_only)?)),
                "draw" => out.push(Node::Draw(self.draw(&child, read_only)?)),
                "exclGroup" => out.push(Node::ExclGroup(self.group(&child, read_only)?)),
                _ => self.page_set(&child, depth + 1)?,
            }
        }
        Ok(())
    }

    fn page_set(&mut self, e: &Element, depth: usize) -> Result<(), XfaError> {
        if depth > MAX_NESTING {
            return Err(XfaError::Limit("template nesting"));
        }
        for child in e.elements() {
            match child.local() {
                "pageArea" => {
                    let child = self.resolve(child);
                    let area = self.page_area(&child, depth)?;
                    self.areas.push(area);
                }
                "pageSet" => {
                    let child = self.resolve(child);
                    self.page_set(&child, depth + 1)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn page_area(&mut self, e: &Element, depth: usize) -> Result<PageArea, XfaError> {
        let (mut width, mut height) = (612.0, 792.0);
        if let Some(medium) = self.prop(e, "medium") {
            let (stock_short, stock_long) = stock_size(medium.attr("stock").unwrap_or(""));
            let short = medium.attr("short").and_then(length).unwrap_or(stock_short);
            let long = medium.attr("long").and_then(length).unwrap_or(stock_long);
            (width, height) = (short, long);
            if medium.attr("orientation") == Some("landscape") {
                (width, height) = (long, short);
            }
        }
        let mut content_areas = Vec::new();
        for ca in e.children_named("contentArea") {
            let ca = self.resolve(ca);
            let len = |n: &str| ca.attr(n).and_then(length).unwrap_or(0.0);
            content_areas.push(Rect::new(len("x"), len("y"), len("w"), len("h")));
        }
        if content_areas.is_empty() {
            content_areas.push(Rect::new(0.0, 0.0, width, height));
        }
        let page_default = Occur {
            min: 0,
            max: None,
            initial: 1,
        };
        let occur = match self.prop(e, "occur") {
            Some(o) => read_occur(&o, page_default),
            None => page_default,
        };
        let mut statics = Vec::new();
        for child in e.elements() {
            if !matches!(
                child.local(),
                "subform" | "area" | "field" | "draw" | "exclGroup"
            ) {
                continue;
            }
            let child = self.resolve(child);
            let node = match child.local() {
                "subform" => Node::Subform(self.subform(&child, false, false, depth + 1)?),
                "area" => Node::Subform(self.subform(&child, false, true, depth + 1)?),
                "field" => Node::Field(self.field(&child, false)?),
                "draw" => Node::Draw(self.draw(&child, false)?),
                _ => Node::ExclGroup(self.group(&child, false)?),
            };
            statics.push(node);
        }
        Ok(PageArea {
            name: e.attr("name").unwrap_or("").to_string(),
            width,
            height,
            content_areas,
            statics,
            occur,
        })
    }

    fn group(&mut self, e: &Element, read_only: bool) -> Result<ExclGroup, XfaError> {
        self.count_node()?;
        let common = self.common(e, read_only);
        let read_only = common.read_only;
        let mut fields = Vec::new();
        for child in e.children_named("field") {
            let child = self.resolve(child);
            fields.push(self.field(&child, read_only)?);
        }
        Ok(ExclGroup {
            layout: Layout::parse(e.attr("layout").unwrap_or("")),
            bind: self.bind_of(e),
            events: self.events(e),
            common,
            fields,
            ..ExclGroup::default()
        })
    }

    fn caption(&mut self, e: &Element, field_font: &Font) -> Option<Caption> {
        let c = self.prop(e, "caption")?;
        let value = match self.prop(&c, "value") {
            Some(v) => read_value(&v).value,
            None => Value::Empty,
        };
        Some(Caption {
            presence: presence(&c),
            placement: Placement::parse(c.attr("placement").unwrap_or("")),
            reserve: c.attr("reserve").and_then(length).filter(|r| *r >= 0.0),
            value,
            font: self.font(&c, field_font),
            para: self.para(&c),
            margin: self.margin(&c),
        })
    }

    fn field(&mut self, e: &Element, read_only: bool) -> Result<Field, XfaError> {
        self.count_node()?;
        let common = self.common(e, read_only);
        let font = self.font(e, &Font::default());
        let para = self.para(e);
        let caption = self.caption(e, &font);
        let mut field = Field {
            common,
            font,
            para,
            caption,
            bind: self.bind_of(e),
            events: self.events(e),
            ..Field::default()
        };
        field.ui = Ui::Unknown;
        let mut ui_picture = None;
        if let Some(ui) = self.prop(e, "ui") {
            ui_picture = ui.child("picture").map(|p| p.text());
            if let Some(widget) = ui
                .elements()
                .find(|w| !matches!(w.local(), "picture" | "extras" | "desc"))
            {
                let widget = self.resolve(widget);
                ui_picture = ui_picture.or_else(|| widget.child("picture").map(|p| p.text()));
                field.ui = self.ui(&widget);
                if !matches!(field.ui, Ui::CheckButton { .. }) {
                    field.ui_border = self.border(&widget);
                    field.ui_margin = self.prop(&widget, "margin").map(|m| read_margin(&m));
                }
            }
        }
        if let Some(v) = self.prop(e, "value") {
            let read = read_value(&v);
            field.value = read.value;
            field.value_kind = read.kind;
            field.max_chars = read.max_chars;
        }
        let format_picture = self
            .prop(e, "format")
            .and_then(|f| f.child("picture").map(|p| p.text()));
        field.picture = format_picture
            .or(ui_picture)
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .map(Picture);
        let lists: Vec<Cow<'_, Element>> =
            e.children_named("items").map(|i| self.resolve(i)).collect();
        field.items = read_items(&lists, field.is_check());
        Ok(field)
    }

    /// The node's `event`s that carry a script, in document order
    /// (`MAX_EVENTS` at most; a script past `MAX_SCRIPT_BYTES` is dropped).
    fn events(&mut self, e: &Element) -> Vec<Event> {
        let mut out = Vec::new();
        for event in e.children_named("event") {
            if out.len() >= MAX_EVENTS {
                break;
            }
            let event = self.resolve(event);
            let Some(script) = event.child("script") else {
                continue;
            };
            let text = script.text();
            if text.trim().is_empty() || text.len() > MAX_SCRIPT_BYTES {
                continue;
            }
            out.push(Event {
                activity: event.attr("activity").unwrap_or("click").trim().to_string(),
                reference: event.attr("ref").unwrap_or("").trim().to_string(),
                javascript: is_javascript(script),
                script: text,
            });
        }
        out
    }

    /// A subform's `variables`: named `script` objects (JavaScript only;
    /// a FormCalc script object is left out, as its functions cannot run)
    /// and named text variables (`text`, `integer`, ...).
    fn variables(&mut self, e: &Element) -> Vec<Variable> {
        let mut out = Vec::new();
        for variables in e.children_named("variables") {
            for v in variables.elements() {
                if out.len() >= MAX_VARIABLES {
                    return out;
                }
                let is_script = match v.local() {
                    "script" => true,
                    "text" | "integer" | "decimal" | "float" | "boolean" | "date" | "time"
                    | "dateTime" => false,
                    _ => continue,
                };
                let name = v.attr("name").unwrap_or("").trim().to_string();
                if name.is_empty() || (is_script && !is_javascript(v)) {
                    continue;
                }
                let text = v.text();
                if text.len() > MAX_SCRIPT_BYTES {
                    continue;
                }
                out.push(Variable {
                    name,
                    text,
                    is_script,
                });
            }
        }
        out
    }

    fn ui(&mut self, widget: &Element) -> Ui {
        match widget.local() {
            "textEdit" => Ui::TextEdit {
                multi_line: flag(widget, "multiLine"),
                comb: widget
                    .child("comb")
                    .and_then(|c| c.attr("numberOfCells"))
                    .and_then(|n| n.trim().parse::<u32>().ok())
                    .filter(|n| *n > 0),
            },
            "numericEdit" => Ui::NumericEdit,
            "dateTimeEdit" => Ui::DateTimeEdit,
            "passwordEdit" => Ui::PasswordEdit,
            "checkButton" => Ui::CheckButton {
                shape: match widget.attr("shape") {
                    Some("round") => CheckShape::Round,
                    _ => CheckShape::Square,
                },
                mark: match widget.attr("mark") {
                    Some("check") => CheckMark::Check,
                    Some("circle") => CheckMark::Circle,
                    Some("cross") => CheckMark::Cross,
                    Some("diamond") => CheckMark::Diamond,
                    Some("square") => CheckMark::Square,
                    Some("star") => CheckMark::Star,
                    _ => CheckMark::Default,
                },
                size: widget
                    .attr("size")
                    .and_then(length)
                    .filter(|s| *s > 0.0)
                    .unwrap_or(10.0),
                border: self.border(widget),
            },
            "choiceList" => Ui::ChoiceList {
                open: match widget.attr("open") {
                    Some("always") => ChoiceOpen::Always,
                    Some("onEntry") => ChoiceOpen::OnEntry,
                    Some("multiSelect") => ChoiceOpen::MultiSelect,
                    _ => ChoiceOpen::UserControl,
                },
                text_entry: flag(widget, "textEntry"),
            },
            "button" => Ui::Button {
                highlight: widget.attr("highlight").unwrap_or("inverted").to_string(),
            },
            "imageEdit" => Ui::ImageEdit,
            "signature" => Ui::Signature,
            "barcode" => Ui::Barcode,
            _ => Ui::Unknown,
        }
    }

    fn draw(&mut self, e: &Element, read_only: bool) -> Result<Draw, XfaError> {
        self.count_node()?;
        let common = self.common(e, read_only);
        let font = self.font(e, &Font::default());
        let para = self.para(e);
        let caption = self.caption(e, &font);
        let mut draw = Draw {
            common,
            font,
            para,
            caption,
            ..Draw::default()
        };
        if let Some(ui) = self.prop(e, "ui")
            && let Some(widget) = ui
                .elements()
                .find(|w| !matches!(w.local(), "picture" | "extras" | "desc"))
        {
            let widget = self.resolve(widget);
            draw.ui = match widget.local() {
                "imageEdit" => DrawUi::Image,
                _ => self.shape(&widget).unwrap_or(DrawUi::Text),
            };
        }
        if let Some(v) = self.prop(e, "value") {
            let read = read_value(&v);
            draw.value = read.value;
            if read.kind == ValueKind::Image {
                draw.ui = DrawUi::Image;
            }
            if let Some(content) = v
                .elements()
                .find(|c| matches!(c.local(), "line" | "rectangle" | "arc"))
            {
                let content = self.resolve(content);
                if let Some(shape) = self.shape(&content) {
                    draw.ui = shape;
                }
            }
        }
        Ok(draw)
    }

    /// A `line`, `rectangle` or `arc` as what the draw shows.
    fn shape(&mut self, e: &Element) -> Option<DrawUi> {
        let edge = |me: &mut Self| match me.prop(e, "edge") {
            Some(edge) => read_edge(&edge),
            None => Edge::default(),
        };
        match e.local() {
            "line" => Some(DrawUi::Line {
                slope_down: e.attr("slope") != Some("/"),
                edge: edge(self),
            }),
            "rectangle" => Some(DrawUi::Rectangle {
                border: self.border_of(e),
            }),
            "arc" => Some(DrawUi::Arc {
                circular: flag(e, "circular"),
                edge: edge(self),
                fill: self.prop(e, "fill").and_then(|f| read_fill(&f)),
            }),
            _ => None,
        }
    }
}

/// The id a `use="#id"` or `usehref="#id"` names. Hrefs to other
/// documents and SOM expressions are not followed.
fn reference(e: &Element) -> Option<&str> {
    let value = e
        .attrs
        .iter()
        .find(|(k, v)| matches!(local_of(k), "use" | "usehref") && v.starts_with('#'))
        .map(|(_, v)| v.as_str())?;
    let id = value.strip_prefix('#')?.trim();
    (!id.is_empty() && !id.starts_with("som(")).then_some(id)
}

fn local_of(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Elements in `e`'s tree, `e` included.
fn element_count(e: &Element) -> usize {
    let mut count = 0usize;
    let mut stack = vec![e];
    while let Some(e) = stack.pop() {
        count += 1;
        stack.extend(e.elements());
    }
    count
}

/// `over` applied on top of the prototype `base`.
fn merge(base: Element, over: Element) -> Element {
    let mut attrs = base.attrs;
    for (k, v) in over.attrs {
        match attrs.iter_mut().find(|(bk, _)| *bk == k) {
            Some(slot) => slot.1 = v,
            None => attrs.push((k, v)),
        }
    }
    let over_text = over
        .children
        .iter()
        .any(|c| matches!(c, XmlNode::Text(t) if !t.trim().is_empty()));
    let mut over_children: Vec<Option<XmlNode>> = over.children.into_iter().map(Some).collect();
    let mut children = Vec::new();
    for child in base.children {
        match child {
            XmlNode::Text(t) => {
                if !over_text {
                    children.push(XmlNode::Text(t));
                }
            }
            XmlNode::Element(b) => {
                let paired = if CONTAINERS.contains(&b.local()) {
                    None
                } else {
                    over_children
                        .iter_mut()
                        .find(|slot| matches!(slot, Some(XmlNode::Element(o)) if o.name == b.name))
                };
                match paired.and_then(Option::take) {
                    Some(XmlNode::Element(o)) => children.push(XmlNode::Element(merge(b, o))),
                    _ => children.push(XmlNode::Element(b)),
                }
            }
        }
    }
    for child in over_children.into_iter().flatten() {
        match child {
            XmlNode::Text(_) if !over_text => {}
            other => children.push(other),
        }
    }
    Element {
        name: over.name,
        attrs,
        children,
    }
}

fn presence(e: &Element) -> Presence {
    Presence::parse(e.attr("presence").unwrap_or(""))
}

/// A `script` element whose `contentType` names JavaScript
/// (`application/x-javascript`); FormCalc and an unset type are not.
fn is_javascript(script: &Element) -> bool {
    script
        .attr("contentType")
        .is_some_and(|c| c.to_ascii_lowercase().contains("javascript"))
}

fn flag(e: &Element, name: &str) -> bool {
    matches!(e.attr(name).map(str::trim), Some("1" | "true"))
}

fn is_read_only(e: &Element) -> bool {
    matches!(
        e.attr("access"),
        Some("readOnly" | "protected" | "nonInteractive")
    )
}

fn break_target(value: Option<&str>) -> Option<Break> {
    match value? {
        "contentArea" => Some(Break::ContentArea),
        "pageArea" | "pageEven" | "pageOdd" => Some(Break::PageArea),
        _ => None,
    }
}

/// Paper sizes of common `stock` names, short side first.
fn stock_size(stock: &str) -> (Pt, Pt) {
    match stock.to_ascii_lowercase().as_str() {
        "legal" => (612.0, 1008.0),
        "a4" => (595.276, 841.89),
        "a3" => (841.89, 1190.551),
        "a5" => (419.528, 595.276),
        "ledger" | "tabloid" => (792.0, 1224.0),
        "executive" => (522.0, 756.0),
        _ => (612.0, 792.0),
    }
}

fn read_occur(o: &Element, default: Occur) -> Occur {
    let int = |name: &str| o.attr(name).and_then(|v| v.trim().parse::<i64>().ok());
    let to_usize = |v: i64| usize::try_from(v.max(0)).unwrap_or(usize::MAX);
    let min = int("min").map_or(default.min, to_usize);
    let mut max = match int("max") {
        Some(v) if v < 0 => None,
        Some(v) => Some(to_usize(v)),
        None => default.max,
    };
    if let Some(m) = max
        && m < min
    {
        max = Some(min);
    }
    let mut initial = int("initial").map_or(default.initial, to_usize).max(min);
    if let Some(m) = max {
        initial = initial.min(m);
    }
    Occur { min, max, initial }
}

fn read_margin(m: &Element) -> Margin {
    let len = |name: &str| m.attr(name).and_then(length).unwrap_or(0.0);
    Margin {
        left: len("leftInset"),
        right: len("rightInset"),
        top: len("topInset"),
        bottom: len("bottomInset"),
    }
}

fn color_of(e: &Element) -> Option<Rgb> {
    e.child("color")
        .and_then(|c| c.attr("value"))
        .and_then(Rgb::parse)
}

fn read_edge(e: &Element) -> Edge {
    Edge {
        presence: presence(e),
        thickness: e
            .attr("thickness")
            .and_then(length)
            .filter(|t| *t >= 0.0)
            .unwrap_or(0.5),
        stroke: Stroke::parse(e.attr("stroke").unwrap_or("solid")),
        color: color_of(e).unwrap_or(Rgb::BLACK),
    }
}

/// A `fill`: its colour, else its pattern's first colour, else white;
/// `None` when hidden.
fn read_fill(f: &Element) -> Option<Fill> {
    let presence = presence(f);
    if presence == Presence::Hidden {
        return None;
    }
    let color = color_of(f)
        .or_else(|| f.elements().find_map(color_of))
        .unwrap_or(Rgb::WHITE);
    Some(Fill { presence, color })
}

fn read_font(f: &Element, default: &Font) -> Font {
    let mut font = default.clone();
    if let Some(face) = f.attr("typeface").filter(|t| !t.trim().is_empty()) {
        font.family = face.trim().to_string();
    }
    if let Some(size) = f.attr("size").and_then(length).filter(|s| *s > 0.0) {
        font.size = size;
    }
    if let Some(w) = f.attr("weight") {
        font.bold = w == "bold";
    }
    if let Some(p) = f.attr("posture") {
        font.italic = p == "italic";
    }
    if let Some(u) = f.attr("underline") {
        font.underline = matches!(u.trim(), "1" | "2");
    }
    if let Some(l) = f.attr("lineThrough") {
        font.line_through = matches!(l.trim(), "1" | "2");
    }
    if let Some(s) = f.attr("letterSpacing") {
        font.letter_spacing = length(s).unwrap_or(0.0);
    }
    if let Some(fill) = f.child("fill")
        && let Some(color) = color_of(fill).or_else(|| fill.elements().find_map(color_of))
    {
        font.color = color;
    }
    font
}

fn read_para(p: &Element) -> Para {
    let len = |name: &str| p.attr(name).and_then(length).unwrap_or(0.0);
    Para {
        h_align: match p.attr("hAlign") {
            Some("center") => HAlign::Center,
            Some("right" | "radix") => HAlign::Right,
            Some("justify" | "justifyAll") => HAlign::Justify,
            _ => HAlign::Left,
        },
        v_align: match p.attr("vAlign") {
            Some("middle") => VAlign::Middle,
            Some("bottom") => VAlign::Bottom,
            _ => VAlign::Top,
        },
        line_height: p.attr("lineHeight").and_then(length).filter(|h| *h > 0.0),
        space_above: len("spaceAbove"),
        space_below: len("spaceBelow"),
        margin_left: len("marginLeft"),
        margin_right: len("marginRight"),
        text_indent: len("textIndent"),
    }
}

struct ReadValue {
    value: Value,
    kind: ValueKind,
    max_chars: u32,
}

/// A `value` element's content.
fn read_value(v: &Element) -> ReadValue {
    let mut out = ReadValue {
        value: Value::Empty,
        kind: ValueKind::Text,
        max_chars: 0,
    };
    for c in v.elements() {
        let kind = match c.local() {
            "text" => ValueKind::Text,
            "integer" => ValueKind::Integer,
            "decimal" => ValueKind::Decimal,
            "float" => ValueKind::Float,
            "date" => ValueKind::Date,
            "time" => ValueKind::Time,
            "dateTime" => ValueKind::DateTime,
            "boolean" => ValueKind::Boolean,
            "image" => ValueKind::Image,
            "exData" => ValueKind::Html,
            _ => continue,
        };
        out.kind = kind;
        out.max_chars = c
            .attr("maxChars")
            .and_then(|m| m.trim().parse::<u32>().ok())
            .unwrap_or(0);
        out.value = match kind {
            ValueKind::Text => Value::Text(c.text()),
            ValueKind::Image => Value::Image,
            ValueKind::Html => {
                let content = c.attr("contentType").unwrap_or("");
                if content.contains("html") || (content.is_empty() && c.has_elements()) {
                    Value::Html(c.inner_xml())
                } else {
                    out.kind = ValueKind::Text;
                    Value::Text(c.text())
                }
            }
            _ => Value::Text(c.text().trim().to_string()),
        };
        break;
    }
    out
}

/// A field's `items` lists: for a check button the first list's on, off
/// and neutral values; for anything else the choices, paired by index
/// between the display list and the `save="1"` list when there are two.
fn read_items(lists: &[Cow<'_, Element>], check: bool) -> Vec<Item> {
    let values = |list: &Element| -> Vec<String> {
        list.elements()
            .map(|e| {
                if e.local() == "text" {
                    e.text()
                } else {
                    e.text().trim().to_string()
                }
            })
            .collect()
    };
    let Some(first) = lists.first() else {
        return Vec::new();
    };
    if check || lists.len() == 1 {
        return values(first)
            .into_iter()
            .map(|v| Item {
                display: v.clone(),
                save: v,
            })
            .collect();
    }
    let save_at = lists.iter().position(|l| flag(l, "save")).unwrap_or(1);
    let display_at = if save_at == 0 { 1 } else { 0 };
    let display = values(&lists[display_at]);
    let save = values(&lists[save_at]);
    display
        .into_iter()
        .enumerate()
        .map(|(i, d)| Item {
            save: save.get(i).cloned().unwrap_or_else(|| d.clone()),
            display: d,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xfa::types::Anchor;

    fn wrap(body: &str) -> String {
        format!(
            "<template xmlns=\"http://www.xfa.org/schema/xfa-template/3.6/\"><subform name=\"form1\" layout=\"tb\" locale=\"fr_CA\">{body}</subform></template>"
        )
    }

    fn approx(a: Pt, b: Pt) -> bool {
        (a - b).abs() < 1e-6
    }

    fn mm(v: f64) -> Pt {
        v * 72.0 / 25.4
    }

    fn only_field(form: &Form) -> &Field {
        match &form.root.children[0] {
            Node::Field(f) => f,
            other => panic!("not a field: {other:?}"),
        }
    }

    #[test]
    fn reads_subform_attributes() {
        let form = read(&wrap(
            "<subform name=\"S\" layout=\"table\" columnWidths=\"10mm 1in -1\" w=\"100pt\" minH=\"5mm\" maxH=\"0\" \
             x=\"1in\" y=\"2pt\" anchorType=\"middleCenter\" presence=\"invisible\" relevant=\"-print\" colSpan=\"-1\" \
             rotate=\"-90\" access=\"readOnly\">\
             <margin leftInset=\"1pt\" topInset=\"2pt\"/><keep intact=\"contentArea\" next=\"pageArea\"/>\
             <break before=\"pageArea\"/><breakAfter targetType=\"contentArea\"/><occur min=\"0\" max=\"-1\" initial=\"2\"/>\
             <bind match=\"global\"/><assist><toolTip>tip</toolTip><speak>say</speak></assist>\
             <field name=\"F\"/></subform>",
        ))
        .unwrap();
        assert_eq!(form.locale, "fr_CA");
        assert_eq!(form.root.common.name, "form1");
        assert_eq!(form.root.layout, Layout::Tb);
        let Node::Subform(s) = &form.root.children[0] else {
            panic!()
        };
        assert_eq!(s.layout, Layout::Table);
        assert!(
            approx(s.column_widths[0], mm(10.0))
                && s.column_widths[1] == 72.0
                && s.column_widths[2] == -1.0
        );
        let c = &s.common;
        assert_eq!(
            (c.w, c.h, c.max_h, c.x, c.y),
            (Some(100.0), None, None, 72.0, 2.0)
        );
        assert!(approx(c.min_h.unwrap(), mm(5.0)));
        assert_eq!(c.anchor, Anchor::MiddleCenter);
        assert_eq!(c.presence, Presence::Invisible);
        assert!(c.screen_only && c.read_only);
        assert_eq!((c.col_span, c.rotate), (0, 270));
        assert_eq!(
            (c.margin.left, c.margin.top, c.margin.right),
            (1.0, 2.0, 0.0)
        );
        assert_eq!(
            c.keep,
            Keep {
                intact: true,
                previous: false,
                next: true
            }
        );
        assert_eq!(
            (c.break_before, c.break_after),
            (Some(Break::PageArea), Some(Break::ContentArea))
        );
        assert_eq!(
            s.occur,
            Occur {
                min: 0,
                max: None,
                initial: 2
            }
        );
        assert_eq!(s.bind.matching, BindMatch::Global);
        assert_eq!((c.tooltip.as_str(), c.speak.as_str()), ("tip", "say"));
        // access cascades.
        let Node::Field(f) = &s.children[0] else {
            panic!()
        };
        assert!(f.common.read_only);
        assert_eq!(f.common.col_span, 1);
    }

    #[test]
    fn reads_a_text_field_with_caption_font_para_and_borders() {
        let form = read(&wrap(
            "<field name=\"CCName\" id=\"f1\" w=\"190.5mm\" minH=\"12.226mm\" bad=\"x\" h=\"12zz\">\
             <ui><textEdit multiLine=\"1\"><border><edge stroke=\"lowered\"/></border><margin topInset=\"1pt\"/>\
             <comb numberOfCells=\"8\"/></textEdit></ui>\
             <font typeface=\"Arial\" size=\"12pt\" weight=\"bold\" posture=\"italic\" underline=\"1\" lineThrough=\"0\" \
             letterSpacing=\"0.5pt\"><fill><color value=\"0,0,255\"/></fill></font>\
             <para hAlign=\"justifyAll\" vAlign=\"middle\" lineHeight=\"14pt\" spaceAbove=\"1pt\" spaceBelow=\"2pt\" \
             marginLeft=\"3pt\" marginRight=\"4pt\" textIndent=\"-5pt\"/>\
             <border presence=\"visible\" hand=\"right\"><edge thickness=\"2pt\"><color value=\"255,0,0\"/></edge>\
             <edge stroke=\"dashed\"/><corner radius=\"3pt\"/><fill><color value=\"1,2,3\"/></fill></border>\
             <caption reserve=\"5.08mm\" placement=\"top\"><value><text>Name of Corporation</text></value>\
             <para vAlign=\"middle\"/></caption>\
             <value><text maxChars=\"40\"> keep  </text></value>\
             <format><picture>text{X}</picture></format><assist><toolTip>Tip</toolTip></assist></field>",
        ))
        .unwrap();
        let f = only_field(&form);
        assert_eq!(f.common.id, "f1");
        assert_eq!(f.common.h, None);
        assert_eq!(
            f.ui,
            Ui::TextEdit {
                multi_line: true,
                comb: Some(8)
            }
        );
        assert_eq!(f.ui_border.edges[0].stroke, Stroke::Other);
        assert_eq!(
            f.ui_margin,
            Some(Margin {
                top: 1.0,
                ..Margin::default()
            })
        );
        assert_eq!(f.font.family, "Arial");
        assert_eq!(f.font.size, 12.0);
        assert!(f.font.bold && f.font.italic && f.font.underline && !f.font.line_through);
        assert_eq!((f.font.color, f.font.letter_spacing), (Rgb(0, 0, 255), 0.5));
        assert_eq!(f.para.h_align, HAlign::Justify);
        assert_eq!(f.para.v_align, VAlign::Middle);
        assert_eq!(f.para.line_height, Some(14.0));
        assert_eq!(
            (
                f.para.space_above,
                f.para.space_below,
                f.para.margin_left,
                f.para.margin_right,
                f.para.text_indent
            ),
            (1.0, 2.0, 3.0, 4.0, -5.0)
        );
        let b = &f.common.border;
        assert!(b.drawn() && !b.inside);
        assert_eq!(
            (b.edges[0].thickness, b.edges[0].color),
            (2.0, Rgb(255, 0, 0))
        );
        // Fewer than four edges: the last repeats.
        assert_eq!(
            [b.edges[1].stroke, b.edges[2].stroke, b.edges[3].stroke],
            [Stroke::Dashed; 3]
        );
        assert_eq!(b.corner_radius, 3.0);
        assert_eq!(
            b.fill,
            Some(Fill {
                presence: Presence::Visible,
                color: Rgb(1, 2, 3)
            })
        );
        let cap = f.caption.as_ref().unwrap();
        assert_eq!(cap.placement, Placement::Top);
        assert!(approx(cap.reserve.unwrap(), mm(5.08)));
        assert_eq!(cap.value, Value::Text("Name of Corporation".into()));
        // No caption font: the field's.
        assert_eq!(cap.font, f.font);
        assert_eq!(cap.para.v_align, VAlign::Middle);
        assert_eq!(f.value, Value::Text(" keep  ".into()));
        assert_eq!((f.value_kind, f.max_chars), (ValueKind::Text, 40));
        assert_eq!(f.picture, Some(Picture("text{X}".into())));
        assert_eq!(f.common.tooltip, "Tip");
    }

    #[test]
    fn reads_every_ui_kind_and_items() {
        let form = read(&wrap(
            "<field name=\"a\"><ui><numericEdit/><picture>num{z9}</picture></ui><value><decimal> 1.50 </decimal></value></field>\
             <field name=\"b\"><ui><dateTimeEdit><picture>date{YYYY}</picture></dateTimeEdit></ui><value><date>2020-01-01</date></value></field>\
             <field name=\"c\"><ui><passwordEdit/></ui></field>\
             <field name=\"d\"><ui><checkButton shape=\"round\" mark=\"cross\" size=\"8pt\"><border><edge/></border></checkButton></ui>\
             <items><integer>1</integer><integer>0</integer><integer>2</integer></items><value><integer>0</integer></value></field>\
             <field name=\"e\"><ui><choiceList open=\"always\" textEntry=\"1\"/></ui>\
             <items><text>One</text><text>Two</text></items><items save=\"1\"><text>1</text><text>2</text></items></field>\
             <field name=\"f\"><ui><choiceList/></ui><items save=\"1\"><text>will</text><text>will not</text></items></field>\
             <field name=\"g\"><ui><button highlight=\"push\"/></ui></field>\
             <field name=\"h\"><ui><imageEdit/></ui><value><image/></value></field>\
             <field name=\"i\"><ui><signature/></ui></field>\
             <field name=\"j\"><ui><barcode/></ui></field>\
             <field name=\"k\"><ui><exObject/></ui></field>\
             <field name=\"l\"/>\
             <field name=\"m\" access=\"protected\"><value><boolean>1</boolean></value></field>",
        ))
        .unwrap();
        let fields: Vec<&Field> = form
            .root
            .children
            .iter()
            .map(|n| match n {
                Node::Field(f) => f,
                _ => panic!(),
            })
            .collect();
        assert_eq!(fields[0].ui, Ui::NumericEdit);
        assert_eq!(fields[0].picture, Some(Picture("num{z9}".into())));
        assert_eq!(
            (fields[0].value_kind, &fields[0].value),
            (ValueKind::Decimal, &Value::Text("1.50".into()))
        );
        assert_eq!(fields[1].ui, Ui::DateTimeEdit);
        assert_eq!(fields[1].picture, Some(Picture("date{YYYY}".into())));
        assert_eq!(fields[1].value_kind, ValueKind::Date);
        assert_eq!(fields[2].ui, Ui::PasswordEdit);
        let Ui::CheckButton {
            shape,
            mark,
            size,
            border,
        } = &fields[3].ui
        else {
            panic!()
        };
        assert_eq!(
            (*shape, *mark, *size),
            (CheckShape::Round, CheckMark::Cross, 8.0)
        );
        assert!(border.drawn());
        assert_eq!(fields[3].check_values(), ("1".to_string(), "0".to_string()));
        assert_eq!(fields[3].items.len(), 3);
        assert_eq!(
            fields[4].ui,
            Ui::ChoiceList {
                open: ChoiceOpen::Always,
                text_entry: true
            }
        );
        assert_eq!(
            fields[4].items,
            vec![
                Item {
                    display: "One".into(),
                    save: "1".into()
                },
                Item {
                    display: "Two".into(),
                    save: "2".into()
                }
            ]
        );
        assert_eq!(
            fields[5].items[1],
            Item {
                display: "will not".into(),
                save: "will not".into()
            }
        );
        assert_eq!(
            fields[6].ui,
            Ui::Button {
                highlight: "push".into()
            }
        );
        assert_eq!(
            (&fields[7].ui, &fields[7].value),
            (&Ui::ImageEdit, &Value::Image)
        );
        assert_eq!(fields[8].ui, Ui::Signature);
        assert_eq!(fields[9].ui, Ui::Barcode);
        assert_eq!(fields[10].ui, Ui::Unknown);
        assert_eq!(fields[11].ui, Ui::Unknown);
        assert!(fields[12].common.read_only);
        assert_eq!(fields[12].value_kind, ValueKind::Boolean);
    }

    #[test]
    fn reads_draws_of_each_kind() {
        let form = read(&wrap(
            "<draw name=\"t\"><ui><textEdit/></ui><value><exData contentType=\"text/html\">\
             <body xmlns=\"http://www.w3.org/1999/xhtml\"><p>A <span style=\"font-weight:bold\">b</span></p></body></exData></value></draw>\
             <draw name=\"l\"><value><line slope=\"/\"><edge thickness=\"1pt\"/></line></value></draw>\
             <draw name=\"r\"><value><rectangle><edge/><corner radius=\"2pt\"/><fill><color value=\"9,9,9\"/></fill></rectangle></value></draw>\
             <draw name=\"a\"><value><arc circular=\"1\"><edge/><fill/></arc></value></draw>\
             <draw name=\"i\"><ui><imageEdit/></ui><value><image/></value></draw>\
             <draw name=\"n\"><value><text>plain</text></value></draw>\
             <draw name=\"u\"><ui><line/></ui></draw>",
        ))
        .unwrap();
        let draws: Vec<&Draw> = form
            .root
            .children
            .iter()
            .map(|n| match n {
                Node::Draw(d) => d,
                _ => panic!(),
            })
            .collect();
        assert_eq!(draws[0].ui, DrawUi::Text);
        assert_eq!(
            draws[0].value,
            Value::Html(
                "<body xmlns=\"http://www.w3.org/1999/xhtml\"><p>A <span style=\"font-weight:bold\">b</span></p></body>".into()
            )
        );
        let DrawUi::Line { slope_down, edge } = draws[1].ui else {
            panic!()
        };
        assert!(!slope_down);
        assert_eq!(edge.thickness, 1.0);
        let DrawUi::Rectangle { border } = &draws[2].ui else {
            panic!()
        };
        assert_eq!(
            (border.corner_radius, border.fill.map(|f| f.color)),
            (2.0, Some(Rgb(9, 9, 9)))
        );
        assert!(border.edges.iter().all(Edge::drawn));
        let DrawUi::Arc { circular, fill, .. } = draws[3].ui else {
            panic!()
        };
        assert!(circular);
        assert_eq!(fill.map(|f| f.color), Some(Rgb::WHITE));
        assert_eq!(
            (&draws[4].ui, &draws[4].value),
            (&DrawUi::Image, &Value::Image)
        );
        assert_eq!(
            (&draws[5].ui, &draws[5].value),
            (&DrawUi::Text, &Value::Text("plain".into()))
        );
        assert!(matches!(
            draws[6].ui,
            DrawUi::Line {
                slope_down: true,
                ..
            }
        ));
    }

    #[test]
    fn border_rules() {
        let form = read(&wrap(
            "<draw name=\"a\"><border/></draw><draw name=\"b\"><border><edge presence=\"hidden\"/><fill presence=\"hidden\"/></border></draw>\
             <draw name=\"c\"><border presence=\"hidden\"><edge/></border></draw><draw name=\"d\"/>\
             <draw name=\"e\"><border><fill><linear><color value=\"4,5,6\"/></linear></fill></border></draw>",
        ))
        .unwrap();
        let border = |i: usize| form.root.children[i].common().border.clone();
        assert!(
            border(0).drawn()
                && border(0).edges == [Edge::default(); 4]
                && border(0).fill.is_none()
        );
        assert!(!border(1).drawn() && border(1).fill.is_none());
        assert!(!border(2).drawn());
        assert_eq!(border(3), Border::none());
        assert_eq!(border(4).fill.map(|f| f.color), Some(Rgb(4, 5, 6)));
    }

    #[test]
    fn reads_exclusion_groups_and_areas() {
        let form = read(&wrap(
            "<exclGroup name=\"CondoType\" layout=\"tb\" access=\"readOnly\"><bind match=\"global\"/>\
             <field name=\"Common\"><ui><checkButton shape=\"round\"/></ui><items><text>1</text></items></field>\
             <field name=\"Standard\"><ui><checkButton/></ui><items><text>2</text></items><value><text>2</text></value></field>\
             </exclGroup><area x=\"1pt\" y=\"2pt\" layout=\"tb\"><draw name=\"d\"/></area>\
             <subformSet><subform name=\"in\"/><field name=\"x\"/></subformSet>",
        ))
        .unwrap();
        let Node::ExclGroup(g) = &form.root.children[0] else {
            panic!()
        };
        assert_eq!(
            (g.layout, g.bind.matching, g.fields.len()),
            (Layout::Tb, BindMatch::Global, 2)
        );
        assert!(g.fields.iter().all(|f| f.common.read_only));
        assert_eq!(g.fields[1].value, Value::Text("2".into()));
        let Node::Subform(area) = &form.root.children[1] else {
            panic!()
        };
        assert!(area.is_area);
        assert_eq!(
            (area.layout, area.common.x, area.common.y),
            (Layout::Position, 1.0, 2.0)
        );
        // subformSet children join the parent.
        assert_eq!(form.root.children.len(), 4);
        assert_eq!(form.root.children[2].common().name, "in");
        assert_eq!(form.root.children[3].common().name, "x");
    }

    #[test]
    fn reads_page_sets() {
        let form = read(&wrap(
            "<pageSet><pageArea name=\"P1\" id=\"Page1\"><contentArea x=\"12.7mm\" y=\"12.7mm\" w=\"190.5mm\" h=\"100mm\"/>\
             <contentArea x=\"0\" y=\"400pt\" w=\"612pt\" h=\"300pt\"/><medium short=\"8.5in\" long=\"11in\"/>\
             <draw name=\"footer\" x=\"10pt\" y=\"760pt\"/><field name=\"pageNo\"/><proto><field id=\"p\"/></proto></pageArea>\
             <pageSet><pageArea name=\"L\"><medium stock=\"a4\" orientation=\"landscape\"/><occur max=\"2\"/></pageArea></pageSet>\
             </pageSet><subform name=\"body\"/>",
        ))
        .unwrap();
        assert_eq!(form.page_set.areas.len(), 2);
        let p1 = &form.page_set.areas[0];
        assert_eq!(
            (p1.name.as_str(), p1.width, p1.height),
            ("P1", 612.0, 792.0)
        );
        assert_eq!(p1.content_areas.len(), 2);
        let ca = p1.content_areas[0];
        assert!(approx(ca.x, mm(12.7)) && approx(ca.w, mm(190.5)) && approx(ca.h, mm(100.0)));
        assert_eq!(p1.content_areas[1], Rect::new(0.0, 400.0, 612.0, 300.0));
        assert_eq!(p1.statics.len(), 2);
        assert_eq!(
            (p1.statics[0].common().x, p1.statics[0].common().y),
            (10.0, 760.0)
        );
        assert_eq!(
            p1.occur,
            Occur {
                min: 0,
                max: None,
                initial: 1
            }
        );
        let l = &form.page_set.areas[1];
        assert!(approx(l.width, 841.89) && approx(l.height, 595.276));
        assert_eq!(
            l.content_areas,
            vec![Rect::new(0.0, 0.0, l.width, l.height)]
        );
        assert_eq!(
            l.occur,
            Occur {
                min: 0,
                max: Some(2),
                initial: 1
            }
        );
        // The body is the only form child; the page set is not a node.
        assert_eq!(form.root.children.len(), 1);
        // No page set: one letter page.
        let bare = read(&wrap("")).unwrap();
        assert_eq!(bare.page_set.areas, vec![PageArea::default()]);
    }

    #[test]
    fn prototypes_supply_defaults() {
        let form = read(
            "<template><subform name=\"form1\"><proto>\
             <font id=\"big\" typeface=\"Courier\" size=\"20pt\" weight=\"bold\"/>\
             <field id=\"base\" w=\"50pt\" h=\"10pt\"><ui><checkButton/></ui><font typeface=\"Times\" size=\"8pt\"/>\
             <items><integer>1</integer><integer>0</integer></items></field>\
             <field id=\"loop1\" use=\"#loop2\" w=\"1pt\"/><field id=\"loop2\" usehref=\"#loop1\" h=\"2pt\"/>\
             </proto>\
             <field name=\"a\" use=\"#base\" h=\"12pt\"><font size=\"9pt\"/></field>\
             <field name=\"b\"><font use=\"#big\" size=\"11pt\"/></field>\
             <field name=\"c\" use=\"#loop1\"/>\
             <field name=\"d\" use=\"#missing\" w=\"3pt\"/>\
             <field name=\"e\" usehref=\"other.xdp#base\"/>\
             </subform></template>",
        )
        .unwrap();
        let field = |i: usize| match &form.root.children[i] {
            Node::Field(f) => f.clone(),
            _ => panic!(),
        };
        let a = field(0);
        assert_eq!((a.common.w, a.common.h), (Some(50.0), Some(12.0)));
        assert!(a.is_check());
        assert_eq!((a.font.family.as_str(), a.font.size), ("Times", 9.0));
        assert_eq!(a.items.len(), 2);
        assert_eq!(a.common.id, "");
        let b = field(1);
        assert_eq!(
            (b.font.family.as_str(), b.font.size, b.font.bold),
            ("Courier", 11.0, true)
        );
        let c = field(2);
        assert_eq!((c.common.w, c.common.h), (Some(1.0), Some(2.0)));
        assert_eq!(field(3).common.w, Some(3.0));
        assert_eq!(field(4).common.w, None);
        assert_eq!(form.root.children.len(), 5);
    }

    #[test]
    fn keeps_scripts_and_skips_unknown_elements() {
        let form = read(&wrap(
            "<event activity=\"click\"><script>x</script></event><variables><script name=\"s\"/>\
             <script name=\"js\" contentType=\"application/x-javascript\">function f(){}</script>\
             <text name=\"t\">unit</text><script name=\"fc\" contentType=\"application/x-formcalc\">1</script></variables>\
             <calculate/><validate/><desc/><extras/><connect/><traversal/><whatever/>\
             <?templateDesigner expand 1?><field name=\"x\"><event activity=\"initialize\" ref=\"$form\">\
             <script contentType=\"application/x-javascript\">y</script></event><event><script/></event></field>",
        ))
        .unwrap();
        assert_eq!(form.root.children.len(), 1);
        assert_eq!(form.root.events.len(), 1);
        assert_eq!(form.root.events[0].activity, "click");
        assert_eq!(form.root.events[0].script, "x");
        assert!(
            !form.root.events[0].javascript,
            "no contentType is FormCalc"
        );
        let names: Vec<(&str, bool)> = form
            .root
            .variables
            .iter()
            .map(|v| (v.name.as_str(), v.is_script))
            .collect();
        assert_eq!(names, [("js", true), ("t", false)]);
        assert_eq!(form.root.variables[1].text, "unit");
        let field = only_field(&form);
        assert_eq!(
            field.events.len(),
            1,
            "an event without a script is dropped"
        );
        assert_eq!(
            (
                field.events[0].activity.as_str(),
                field.events[0].reference.as_str(),
                field.events[0].javascript
            ),
            ("initialize", "$form", true)
        );
    }

    #[test]
    fn hostile_templates_fail_cleanly() {
        assert!(matches!(read("<template>"), Err(XfaError::Xml(_))));
        assert!(matches!(read("<template/>"), Err(XfaError::Unsupported(_))));
        assert!(matches!(
            read("<other><subform/></other>"),
            Err(XfaError::Unsupported(_))
        ));
        // A self-referencing prototype.
        assert!(read(&wrap("<field id=\"s\" use=\"#s\" name=\"x\"/>")).is_ok());
        // Nesting past the container limit.
        let deep = "<subform>".repeat(MAX_NESTING + 5) + &"</subform>".repeat(MAX_NESTING + 5);
        assert_eq!(
            read(&wrap(&deep)).unwrap_err(),
            XfaError::Limit("template nesting")
        );
        // Past the XML depth: an XML error.
        let deeper = "<subform>".repeat(600) + &"</subform>".repeat(600);
        assert!(matches!(read(&wrap(&deeper)), Err(XfaError::Xml(_))));
        // Within limits, deep nesting reads.
        let ok = "<subform>".repeat(200) + &"</subform>".repeat(200);
        assert!(read(&wrap(&ok)).is_ok());
        // Prototype expansion that multiplies.
        let mut chain =
            String::from("<proto><subform id=\"p0\"><field/><field/><field/><field/></subform>");
        for i in 1..12 {
            let prev = i - 1;
            chain.push_str(&format!(
                "<subform id=\"p{i}\"><subform use=\"#p{prev}\"/><subform use=\"#p{prev}\"/><subform use=\"#p{prev}\"/><subform use=\"#p{prev}\"/></subform>"
            ));
        }
        chain.push_str("</proto><subform use=\"#p11\"/>");
        assert!(matches!(read(&wrap(&chain)), Err(XfaError::Limit(_))));
        // Truncations of a real-looking template never panic.
        let text = wrap(
            "<field name=\"a\" w=\"1mm\"><ui><checkButton/></ui><items><integer>1</integer></items><caption><value><exData contentType=\"text/html\"><body><p>x</p></body></exData></value></caption></field>",
        );
        for cut in 0..text.len() {
            let _ = read(&text[..cut]);
        }
    }

    /// The owner's bench form, when it is on this machine (never in CI).
    #[test]
    fn bench_template() {
        let Ok(text) =
            std::fs::read_to_string("/Users/jiaqihou/Documents/tpdf-wt/scratch/xfa/template.xml")
        else {
            return;
        };
        let form = read(&text).unwrap();
        assert_eq!(form.root.common.name, "form1");
        assert_eq!(form.locale, "en_US");
        let (mut fields, mut draws, mut subforms, mut groups) = (0, 0, 0, 0);
        let mut stack: Vec<&Node> = form.root.children.iter().collect();
        while let Some(node) = stack.pop() {
            match node {
                Node::Subform(s) => {
                    subforms += 1;
                    stack.extend(s.children.iter());
                }
                Node::Field(_) => fields += 1,
                Node::Draw(_) => draws += 1,
                Node::ExclGroup(g) => {
                    groups += 1;
                    fields += g.fields.len();
                }
            }
        }
        println!(
            "bench template: {fields} fields, {draws} draws, {subforms} subforms, {groups} groups"
        );
        assert_eq!((fields, draws, groups), (209, 265, 2));
        assert_eq!(form.page_set.areas.len(), 1);
        let page = &form.page_set.areas[0];
        assert_eq!((page.width, page.height), (612.0, 792.0));
        assert_eq!(page.content_areas.len(), 1);
        let ca = page.content_areas[0];
        assert!(approx(ca.x, mm(12.7)) && approx(ca.y, mm(12.7)));
        assert!(approx(ca.w, mm(190.5)) && approx(ca.h, mm(263.525)));
    }
}
