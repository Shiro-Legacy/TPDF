// SPDX-License-Identifier: MIT
//! The layout: the bound form flowed into pages. Subforms place their
//! children by `Layout` (positioned, top-to-bottom, left-to-right, table
//! and row), fields and draws take their fixed size or grow to their
//! content (`min_h`, `max_h`), captions take their reserve, hidden nodes
//! take no space, and content that passes the bottom of the current
//! content area continues in the next one, or on a new page from the
//! page set (`keep` and `break` honoured). Every page gets its master's
//! static content.
//!
//! The result is paint-ordered items in page coordinates (points,
//! y-down): borders and fills, text blocks, shapes, and a `Widget` for
//! each fillable field (which the emitter turns into an annotation; the
//! view draws its value).
//!
//! It runs in two passes.
//!
//! **Measure.** Every visible node becomes a `Laid` box of known size.
//! A box's margins are insets inside it; its border is drawn on it. A
//! node without `w` takes the parent's content width in a `tb` flow (and
//! a table column's width in a row); elsewhere it takes `min_w` or its
//! natural width, capped by `max_w`. A node without `h` takes its
//! content's height clamped to `min_h..=max_h` (`min_h` wins a conflict).
//! A field with no value still gets one line of its font; a check button
//! its `size`. Captions carve `reserve` (or, left and right, the
//! caption's natural width + 2pt, at most half the content width; top and
//! bottom, its laid-out height) off their side; `inline` is `left` with
//! the natural width; a button's caption without a reserve takes the
//! whole content rect. Positioned children sit at their `x`/`y` with the
//! `anchor` point there; `tb` stacks; `lr-tb` (and `rl-tb`, approximated
//! as `lr-tb`) fills lines and wraps; a `table` sizes its columns (`-1`
//! and missing entries share what is left equally) and every row
//! subform puts its cells in successive columns (`colSpan`; 0 is 1, past
//! the last column is the rest of the row), every cell stretched to the
//! row's height; a non-row child of a table spans its width. A `tb`,
//! `lr-tb` or `table` box without a fixed height stays a *flow*: a list
//! of children (lines for `lr-tb`, rows for a table) the paginator may
//! split between. Everything else (fields, draws, positioned subforms,
//! rows, fixed-height or `max_h`-cut flows) is a *leaf* of items.
//! `invisible` nodes keep their size and draw nothing; `hidden` and
//! `inactive` ones are skipped.
//!
//! **Paginate.** The root flows into the first content area of the first
//! page master at the area's left. A leaf that does not fit the rest of
//! the area moves to the next area (the master's next content area, else
//! a new page: the same master until its `occur.max` is used, then the
//! next master; masters without a usable content area, one of positive
//! height, are skipped), unless the area is still empty or the leaf is
//! taller than the whole area, in which case it is placed where it is and
//! overflows. A flow splits between its children; its border is drawn
//! once per fragment, and a fragment left with nothing in it is dropped
//! (its top margin is applied again in the next area). A `keep intact`
//! flow that does not fit moves whole to the next area first, and splits
//! only if it is taller than an area. Children chained by `keep next` /
//! `keep previous` move together when they fit in one area.
//! `breakBefore` / `breakAfter` start a new content area or page, never
//! before the first item placed nor when the area (page) is still empty;
//! a break after the last item adds nothing. Each page starts with its
//! master's statics (laid out as positioned children of the page).
//!
//! Limits: `limits::MAX_PAGES`, `limits::MAX_ITEMS`, 256 levels of
//! nesting (`XfaError::Limit`). Non-finite lengths read as zero, negative
//! sizes as zero. Without any usable content area the whole form goes on
//! one page.
//!
//! Approximations: `rotate` is ignored; `hand` is left to the border's
//! `inside`; `rl-tb` lays out as `lr-tb`; anchors only matter for
//! positioned children; `occur.min` of page masters is not padded; a
//! flow's continuation fragment does not repeat its top margin and its
//! last fragment gets the bottom margin but not a `min_h` remainder; flows
//! are measured once, at the first usable area's width; exclusion groups
//! lay out as subforms; check buttons are placed by the field's `para`
//! alignment as read (the model's default is top, not middle).

use super::XfaError;
use super::limits;
use super::text::{self, Block};
use super::types::{
    Anchor, Border, Break, Caption, Common, Edge, Field, Fill, Form, HAlign, Keep, Layout, Margin,
    Node, Para, Placement, Pt, Rect, Subform, Ui, VAlign, Value,
};

/// What is painted at an item's rectangle.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    /// A node's border and fill over its box.
    Border(Border),
    /// A text block laid out within the rectangle (the rectangle already
    /// excludes margins), aligned vertically.
    Text { block: Block, v_align: VAlign },
    /// A `draw` line.
    Line { slope_down: bool, edge: Edge },
    /// A `draw` rectangle.
    Rectangle(Border),
    /// A `draw` arc.
    Arc {
        circular: bool,
        edge: Edge,
        fill: Option<Fill>,
    },
    /// A check button's box (the border it draws when unfilled, and the
    /// mark when `checked`); for a fillable one a `Widget` follows at the
    /// same rectangle.
    Check {
        border: Border,
        checked: bool,
        round: bool,
    },
    /// A fillable field's value area, or a clickable push button's whole
    /// box (`Field::clickable`): the emitter writes a widget annotation
    /// here. The field carries its SOM name, data path, kind and current
    /// value.
    Widget(Box<Field>),
}

/// One painted item.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub rect: Rect,
    pub kind: ItemKind,
}

impl Item {
    fn offset(&self, dx: Pt, dy: Pt) -> Item {
        Item {
            rect: self.rect.offset(dx, dy),
            kind: self.kind.clone(),
        }
    }
}

/// A laid-out page.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Page {
    pub width: Pt,
    pub height: Pt,
    /// In paint order.
    pub items: Vec<Item>,
}

/// Lays `form` out into pages. Bounded by `limits::MAX_PAGES` and
/// `MAX_ITEMS`. A form with no visible content is one empty page.
pub fn lay_out(form: &Form) -> Result<Vec<Page>, XfaError> {
    lay_out_bounded(form, limits::MAX_PAGES, limits::MAX_ITEMS)
}

/// Nesting the layout descends before it gives up.
const MAX_DEPTH: usize = 256;
/// Tolerance for fitting comparisons, in points.
const EPS: Pt = 1e-3;
/// US Letter, the page of a form without a page set.
const LETTER: (Pt, Pt) = (612.0, 792.0);

fn lay_out_bounded(form: &Form, max_pages: usize, max_items: usize) -> Result<Vec<Page>, XfaError> {
    let mut measurer = Measurer {
        form,
        items: 0,
        max_items,
    };
    let mut masters = Vec::new();
    for area in &form.page_set.areas {
        let areas: Vec<Rect> = area
            .content_areas
            .iter()
            .map(clean_rect)
            .filter(|r| r.h > 0.0)
            .collect();
        if areas.is_empty() {
            continue;
        }
        masters.push(Master {
            width: len(area.width),
            height: len(area.height),
            areas,
            statics: measurer.statics(&area.statics)?,
            max: area.occur.max.map(|m| m.max(1)),
        });
    }
    let mut single = false;
    if masters.is_empty() {
        // No page set: Letter with a full-page area, as many as needed.
        // A page set without a usable area: everything on one page.
        let (width, height, statics) = match form.page_set.areas.first() {
            Some(first) => {
                single = true;
                (
                    len(first.width),
                    len(first.height),
                    measurer.statics(&first.statics)?,
                )
            }
            None => (LETTER.0, LETTER.1, Vec::new()),
        };
        masters.push(Master {
            width,
            height,
            areas: vec![Rect::new(0.0, 0.0, width, height)],
            statics,
            max: None,
        });
    }
    let root = &form.root;
    let laid = if root.common.presence.occupies() {
        let width = masters[0].areas[0].w;
        Some(measurer.subform(root, Width::Fill(width), None, 0)?)
    } else {
        None
    };
    let mut pager = Pager {
        used: vec![0; masters.len()],
        masters,
        single,
        m: 0,
        a: 0,
        pages: Vec::new(),
        cursor: 0.0,
        area_has: false,
        page_has: false,
        started: false,
        frags: Vec::new(),
        pending: None,
        items: 0,
        max_items,
        max_pages,
    };
    pager.new_page(0)?;
    if let Some(laid) = &laid {
        pager.place(laid, 0.0)?;
    }
    Ok(pager
        .pages
        .into_iter()
        .map(|p| Page {
            width: p.width,
            height: p.height,
            items: p.items.into_iter().flatten().collect(),
        })
        .collect())
}

// ---------------------------------------------------------------------
// Numbers

/// A size: non-finite and negative read as zero.
fn len(v: Pt) -> Pt {
    if v.is_finite() && v > 0.0 { v } else { 0.0 }
}

/// A position: non-finite reads as zero.
fn pos(v: Pt) -> Pt {
    if v.is_finite() { v } else { 0.0 }
}

fn clean_margin(m: &Margin) -> Margin {
    Margin {
        left: len(m.left),
        right: len(m.right),
        top: len(m.top),
        bottom: len(m.bottom),
    }
}

fn clean_rect(r: &Rect) -> Rect {
    Rect::new(pos(r.x), pos(r.y), len(r.w), len(r.h))
}

/// `v` within `min..=max` (each optional; `min` wins a conflict).
fn clamp(v: Pt, min: Option<Pt>, max: Option<Pt>) -> Pt {
    let mut v = len(v);
    if let Some(max) = max.filter(|m| m.is_finite()) {
        v = v.min(len(max));
    }
    if let Some(min) = min {
        v = v.max(len(min));
    }
    v
}

fn text_w(block: &Block) -> Pt {
    if block.is_empty() {
        0.0
    } else {
        len(text::lay_out(block, None).width)
    }
}

fn text_h(block: &Block, width: Pt) -> Pt {
    if block.is_empty() {
        0.0
    } else {
        len(text::lay_out(block, Some(len(width))).height)
    }
}

/// The offset from a node's top-left to its anchor point.
fn anchor_offset(anchor: Anchor, w: Pt, h: Pt) -> (Pt, Pt) {
    let x = match anchor {
        Anchor::TopLeft | Anchor::MiddleLeft | Anchor::BottomLeft => 0.0,
        Anchor::TopCenter | Anchor::MiddleCenter | Anchor::BottomCenter => w / 2.0,
        Anchor::TopRight | Anchor::MiddleRight | Anchor::BottomRight => w,
    };
    let y = match anchor {
        Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight => 0.0,
        Anchor::MiddleLeft | Anchor::MiddleCenter | Anchor::MiddleRight => h / 2.0,
        Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => h,
    };
    (x, y)
}

// ---------------------------------------------------------------------
// Measure

/// How a node's width is decided when it has no `w` of its own.
#[derive(Clone, Copy, Debug)]
enum Width<'w> {
    /// In a `tb` flow: the parent's content width.
    Fill(Pt),
    /// Positioned or in a line: `min_w` or the natural width.
    Natural,
    /// A table cell or a table's non-row child: this width, even over
    /// its own `w`.
    Forced(Pt),
    /// A row subform in a table: these column widths.
    Columns(&'w [Pt]),
}

/// A measured node: its size, what pagination needs to know about it,
/// and its content relative to its top-left.
#[derive(Clone, Debug)]
struct Laid {
    w: Pt,
    h: Pt,
    keep: Keep,
    break_before: Option<Break>,
    break_after: Option<Break>,
    body: Body,
}

#[derive(Clone, Debug)]
enum Body {
    /// Never split: these items.
    Leaf(Vec<Item>),
    /// Splits between its children.
    Flow(Flow),
}

#[derive(Clone, Debug)]
struct Flow {
    border: Option<Border>,
    margin: Margin,
    intact: bool,
    /// Stacked top to bottom from the top margin; each with its x
    /// offset from the flow's left.
    children: Vec<(Pt, Laid)>,
}

impl Laid {
    fn leaf(w: Pt, h: Pt, items: Vec<Item>) -> Laid {
        Laid {
            w,
            h,
            keep: Keep::default(),
            break_before: None,
            break_after: None,
            body: Body::Leaf(items),
        }
    }
}

/// Appends `laid`'s items at (`ox`, `oy`) to `out`.
fn flatten(laid: Laid, ox: Pt, oy: Pt, out: &mut Vec<Item>) {
    match laid.body {
        Body::Leaf(items) => out.extend(items.into_iter().map(|mut i| {
            i.rect = i.rect.offset(ox, oy);
            i
        })),
        Body::Flow(flow) => {
            if let Some(border) = flow.border {
                out.push(Item {
                    rect: Rect::new(ox, oy, laid.w, laid.h),
                    kind: ItemKind::Border(border),
                });
            }
            let mut y = oy + flow.margin.top;
            for (dx, child) in flow.children {
                let h = child.h;
                flatten(child, ox + dx, y, out);
                y += h;
            }
        }
    }
}

/// A caption's resolved side and extent along its axis.
struct CapPlan<'c> {
    cap: &'c Caption,
    placement: Placement,
    extent: Pt,
    block: Block,
}

/// A field's or draw's box split into caption and value.
struct Frame {
    w: Pt,
    h: Pt,
    items: Vec<Item>,
    value: Rect,
}

struct Measurer<'f> {
    form: &'f Form,
    items: usize,
    max_items: usize,
}

impl<'f> Measurer<'f> {
    fn block(&self, value: &Value, font: &super::types::Font, para: &Para) -> Block {
        let ids = &self.form.ids;
        text::block(value, font, para, &|id: &str| ids.get(id).cloned())
    }

    fn count(&mut self, n: usize) -> Result<(), XfaError> {
        self.items = self.items.saturating_add(n);
        if self.items > self.max_items {
            Err(XfaError::Limit("layout items"))
        } else {
            Ok(())
        }
    }

    /// A page master's statics, as items on the page.
    fn statics(&mut self, nodes: &[Node]) -> Result<Vec<Item>, XfaError> {
        let mut out = Vec::new();
        for node in nodes.iter().filter(|n| n.common().presence.occupies()) {
            let laid = self.node(node, Width::Natural, None, 1)?;
            let c = node.common();
            let (ax, ay) = anchor_offset(c.anchor, laid.w, laid.h);
            flatten(laid, pos(c.x) - ax, pos(c.y) - ay, &mut out);
        }
        Ok(out)
    }

    fn node(
        &mut self,
        node: &Node,
        width: Width<'_>,
        force_h: Option<Pt>,
        depth: usize,
    ) -> Result<Laid, XfaError> {
        if depth > MAX_DEPTH {
            return Err(XfaError::Limit("layout depth"));
        }
        let laid = match node {
            Node::Field(f) => self.field(f, width, force_h)?,
            Node::Draw(d) => self.draw(d, width, force_h)?,
            Node::Subform(s) => return self.subform(s, width, force_h, depth),
            Node::ExclGroup(g) => {
                let children: Vec<Node> = g.fields.iter().cloned().map(Node::Field).collect();
                let k = Container {
                    common: &g.common,
                    layout: g.layout,
                    columns: &[],
                    children: &children,
                };
                self.container(&k, width, force_h, depth)?
            }
        };
        Ok(finish(laid, node.common()))
    }

    fn subform(
        &mut self,
        s: &Subform,
        width: Width<'_>,
        force_h: Option<Pt>,
        depth: usize,
    ) -> Result<Laid, XfaError> {
        if depth > MAX_DEPTH {
            return Err(XfaError::Limit("layout depth"));
        }
        let k = Container {
            common: &s.common,
            layout: s.layout,
            columns: &s.column_widths,
            children: &s.children,
        };
        let laid = self.container(&k, width, force_h, depth)?;
        Ok(finish(laid, &s.common))
    }

    /// The box, the caption's item and the value rect of a field or draw.
    /// `value_w` is the value's natural width; `value_h` its height at a
    /// given width.
    #[allow(clippy::too_many_arguments)]
    fn frame(
        &self,
        c: &Common,
        caption: Option<&Caption>,
        whole: bool,
        value_w: Pt,
        value_h: &dyn Fn(Pt) -> Pt,
        width: Width<'_>,
        force_h: Option<Pt>,
    ) -> Frame {
        let margin = clean_margin(&c.margin);
        let caption = caption.filter(|cap| cap.presence.occupies());
        let w = resolve_w(c, width, || {
            margin.horizontal() + self.caption_natural_w(caption, whole, value_w)
        });
        let cw = (w - margin.horizontal()).max(0.0);
        let plan = caption.map(|cap| self.caption_plan(cap, cw, whole));
        let content_h = match &plan {
            None => value_h(cw),
            Some(p) => match p.placement {
                Placement::Top | Placement::Bottom => p.extent + value_h(cw),
                _ => {
                    let cm = clean_margin(&p.cap.margin);
                    let cap_h = text_h(&p.block, p.extent - cm.horizontal()) + cm.vertical();
                    cap_h.max(value_h((cw - p.extent).max(0.0)))
                }
            },
        };
        let h = resolve_h(c, force_h, content_h + margin.vertical());
        let bx = Rect::new(0.0, 0.0, w, h);
        let content = bx.inset(&margin);
        let mut items = Vec::new();
        if c.border.drawn() {
            items.push(Item {
                rect: bx,
                kind: ItemKind::Border(c.border.clone()),
            });
        }
        let value = match plan {
            None => content,
            Some(p) => {
                let (cap_rect, value) = split(content, p.placement, p.extent);
                if p.cap.presence.drawn() && !p.block.is_empty() {
                    items.push(Item {
                        rect: cap_rect.inset(&clean_margin(&p.cap.margin)),
                        kind: ItemKind::Text {
                            block: p.block,
                            v_align: p.cap.para.v_align,
                        },
                    });
                }
                value
            }
        };
        Frame { w, h, items, value }
    }

    fn caption_natural_w(&self, caption: Option<&Caption>, whole: bool, value_w: Pt) -> Pt {
        let Some(cap) = caption else { return value_w };
        let block = self.block(&cap.value, &cap.font, &cap.para);
        let natural = text_w(&block) + clean_margin(&cap.margin).horizontal();
        if whole {
            return natural.max(value_w);
        }
        match cap.placement {
            Placement::Top | Placement::Bottom => natural.max(value_w),
            _ => cap.reserve.map(len).unwrap_or(natural + 2.0) + value_w,
        }
    }

    fn caption_plan<'c>(&self, cap: &'c Caption, cw: Pt, whole: bool) -> CapPlan<'c> {
        let block = self.block(&cap.value, &cap.font, &cap.para);
        let cm = clean_margin(&cap.margin);
        let natural = || {
            if block.is_empty() {
                0.0
            } else {
                text_w(&block) + cm.horizontal() + 2.0
            }
        };
        let (placement, extent) = if whole {
            (Placement::Left, cw)
        } else {
            match cap.placement {
                Placement::Left | Placement::Right => (
                    cap.placement,
                    cap.reserve
                        .map(len)
                        .unwrap_or_else(|| natural().min(cw / 2.0)),
                ),
                Placement::Inline => (
                    Placement::Left,
                    cap.reserve.map(len).unwrap_or_else(|| natural().min(cw)),
                ),
                Placement::Top | Placement::Bottom => (
                    cap.placement,
                    cap.reserve
                        .map(len)
                        .unwrap_or_else(|| text_h(&block, cw - cm.horizontal()) + cm.vertical()),
                ),
            }
        };
        CapPlan {
            cap,
            placement,
            extent: len(extent),
            block,
        }
    }

    /// The value a non-fillable field shows: a choice's display text for
    /// its save value, else the value.
    fn shown_value(f: &Field) -> Value {
        if let (Ui::ChoiceList { .. }, Value::Text(t)) = (&f.ui, &f.value)
            && let Some(item) = f.items.iter().find(|i| &i.save == t)
        {
            return Value::Text(item.display.clone());
        }
        f.value.clone()
    }

    fn field(
        &mut self,
        f: &Field,
        width: Width<'_>,
        force_h: Option<Pt>,
    ) -> Result<Laid, XfaError> {
        let is_button = matches!(f.ui, Ui::Button { .. });
        let check = match &f.ui {
            Ui::CheckButton {
                shape,
                size,
                border,
                ..
            } => Some((
                len(*size),
                *shape == super::types::CheckShape::Round,
                border,
            )),
            _ => None,
        };
        let block = if is_button || check.is_some() {
            Block::default()
        } else {
            self.block(&Self::shown_value(f), &f.font, &f.para)
        };
        let line = len(text::line_height(&f.font, &f.para));
        let value_w = match check {
            Some((s, ..)) => s,
            None => text_w(&block),
        };
        let value_h = |vw: Pt| match check {
            Some((s, ..)) => s,
            None if is_button => 0.0,
            None => line.max(text_h(&block, vw)),
        };
        let whole = is_button && f.caption.as_ref().is_some_and(|c| c.reserve.is_none());
        let frame = self.frame(
            &f.common,
            f.caption.as_ref(),
            whole,
            value_w,
            &value_h,
            width,
            force_h,
        );
        let Frame {
            w,
            h,
            mut items,
            value,
        } = frame;
        match check {
            Some((s, round, border)) => {
                let x = match f.para.h_align {
                    HAlign::Center => value.x + (value.w - s) / 2.0,
                    HAlign::Right => value.right() - s,
                    HAlign::Left | HAlign::Justify => value.x,
                };
                let y = match f.para.v_align {
                    VAlign::Top => value.y,
                    VAlign::Middle => value.y + (value.h - s) / 2.0,
                    VAlign::Bottom => value.bottom() - s,
                };
                let square = Rect::new(x, y, s, s);
                items.push(Item {
                    rect: square,
                    kind: ItemKind::Check {
                        border: border.clone(),
                        checked: f.checked,
                        round,
                    },
                });
                if f.fillable() {
                    items.push(Item {
                        rect: square,
                        kind: ItemKind::Widget(Box::new(f.clone())),
                    });
                }
            }
            None => {
                if f.fillable() {
                    items.push(Item {
                        rect: value,
                        kind: ItemKind::Widget(Box::new(f.clone())),
                    });
                } else if !is_button && !block.is_empty() {
                    items.push(Item {
                        rect: value,
                        kind: ItemKind::Text {
                            block,
                            v_align: f.para.v_align,
                        },
                    });
                }
            }
        }
        if is_button && f.clickable() {
            items.push(Item {
                rect: Rect::new(0.0, 0.0, w, h),
                kind: ItemKind::Widget(Box::new(f.clone())),
            });
        }
        self.count(items.len())?;
        Ok(Laid::leaf(w, h, items))
    }

    fn draw(
        &mut self,
        d: &super::types::Draw,
        width: Width<'_>,
        force_h: Option<Pt>,
    ) -> Result<Laid, XfaError> {
        use super::types::DrawUi;
        let block = if d.ui == DrawUi::Text {
            self.block(&d.value, &d.font, &d.para)
        } else {
            Block::default()
        };
        let value_h = |vw: Pt| text_h(&block, vw);
        let frame = self.frame(
            &d.common,
            d.caption.as_ref(),
            false,
            text_w(&block),
            &value_h,
            width,
            force_h,
        );
        let Frame {
            w,
            h,
            mut items,
            value,
        } = frame;
        let kind = match &d.ui {
            DrawUi::Text => (!block.is_empty()).then_some(ItemKind::Text {
                block,
                v_align: d.para.v_align,
            }),
            DrawUi::Line { slope_down, edge } => Some(ItemKind::Line {
                slope_down: *slope_down,
                edge: *edge,
            }),
            DrawUi::Rectangle { border } => Some(ItemKind::Rectangle(border.clone())),
            DrawUi::Arc {
                circular,
                edge,
                fill,
            } => Some(ItemKind::Arc {
                circular: *circular,
                edge: *edge,
                fill: *fill,
            }),
            DrawUi::Image => None,
        };
        if let Some(kind) = kind {
            items.push(Item { rect: value, kind });
        }
        self.count(items.len())?;
        Ok(Laid::leaf(w, h, items))
    }

    fn container(
        &mut self,
        k: &Container<'_>,
        width: Width<'_>,
        force_h: Option<Pt>,
        depth: usize,
    ) -> Result<Laid, XfaError> {
        let c = k.common;
        let margin = clean_margin(&c.margin);
        let visible: Vec<&Node> = k
            .children
            .iter()
            .filter(|n| n.common().presence.occupies())
            .collect();
        let known = known_w(c, width);
        if c.border.drawn() {
            self.count(1)?;
        }
        match (k.layout, width) {
            (Layout::Position, _) => {
                let mut placed = Vec::with_capacity(visible.len());
                let (mut right, mut bottom): (Pt, Pt) = (0.0, 0.0);
                for child in visible {
                    let laid = self.node(child, Width::Natural, None, depth + 1)?;
                    let cc = child.common();
                    let (ax, ay) = anchor_offset(cc.anchor, laid.w, laid.h);
                    let (x, y) = (pos(cc.x) - ax, pos(cc.y) - ay);
                    right = right.max(x + laid.w);
                    bottom = bottom.max(y + laid.h);
                    placed.push((x, y, laid));
                }
                let w =
                    known.unwrap_or_else(|| clamp(right + margin.horizontal(), c.min_w, c.max_w));
                let h = resolve_h(c, force_h, bottom + margin.vertical());
                let mut items = Vec::new();
                if c.border.drawn() {
                    items.push(Item {
                        rect: Rect::new(0.0, 0.0, w, h),
                        kind: ItemKind::Border(c.border.clone()),
                    });
                }
                for (x, y, laid) in placed {
                    flatten(laid, margin.left + x, margin.top + y, &mut items);
                }
                Ok(Laid::leaf(w, h, items))
            }
            (Layout::Row, Width::Columns(columns)) => {
                self.row(c, &visible, columns, force_h, depth)
            }
            (Layout::Tb, _) => {
                let rule = match known {
                    Some(w) => Width::Fill((w - margin.horizontal()).max(0.0)),
                    None => Width::Natural,
                };
                let mut children = Vec::with_capacity(visible.len());
                for child in visible {
                    children.push(self.node(child, rule, None, depth + 1)?);
                }
                let w = known.unwrap_or_else(|| {
                    let widest = children.iter().map(|l| l.w).fold(0.0, Pt::max);
                    clamp(widest + margin.horizontal(), c.min_w, c.max_w)
                });
                Ok(finish_flow(c, margin, w, force_h, children))
            }
            (Layout::Lr | Layout::Rl | Layout::Row, _) => {
                let limit = known
                    .map(|w| (w - margin.horizontal()).max(0.0))
                    .unwrap_or(Pt::INFINITY);
                let mut lines: Vec<Laid> = Vec::new();
                let mut line: Vec<Item> = Vec::new();
                let (mut x, mut line_h, mut widest): (Pt, Pt, Pt) = (0.0, 0.0, 0.0);
                for child in visible {
                    let laid = self.node(child, Width::Natural, None, depth + 1)?;
                    if x > 0.0 && x + laid.w > limit + EPS {
                        lines.push(Laid::leaf(x, line_h, std::mem::take(&mut line)));
                        widest = widest.max(x);
                        x = 0.0;
                        line_h = 0.0;
                    }
                    let (lw, lh) = (laid.w, laid.h);
                    flatten(laid, x, 0.0, &mut line);
                    x += lw;
                    line_h = line_h.max(lh);
                }
                if x > 0.0 || line_h > 0.0 || !line.is_empty() {
                    lines.push(Laid::leaf(x, line_h, line));
                    widest = widest.max(x);
                }
                let w =
                    known.unwrap_or_else(|| clamp(widest + margin.horizontal(), c.min_w, c.max_w));
                Ok(finish_flow(c, margin, w, force_h, lines))
            }
            (Layout::Table, _) => {
                let spans = visible
                    .iter()
                    .filter_map(|n| match n {
                        Node::Subform(s) if s.layout == Layout::Row => Some(
                            s.children
                                .iter()
                                .filter(|c| c.common().presence.occupies())
                                .map(|c| (c.common().col_span.max(1) as usize).min(MAX_COLUMNS))
                                .fold(0usize, |a, b| a.saturating_add(b)),
                        ),
                        _ => None,
                    })
                    .max()
                    .unwrap_or(0);
                let n = k.columns.len().max(spans).min(MAX_COLUMNS);
                let fixed: Vec<Option<Pt>> = (0..n)
                    .map(|i| {
                        k.columns
                            .get(i)
                            .copied()
                            .filter(|v| v.is_finite() && *v >= 0.0)
                    })
                    .collect();
                let fixed_sum: Pt = fixed.iter().flatten().sum();
                let autos = fixed.iter().filter(|v| v.is_none()).count();
                // A table without a width of its own and with every column
                // fixed is as wide as its columns, even inside a flow that
                // would stretch it (Designer sizes it so).
                let own_columns = c.w.is_none() && autos == 0 && fixed_sum > 0.0;
                let cw = if own_columns {
                    fixed_sum
                } else {
                    known
                        .map(|w| (w - margin.horizontal()).max(0.0))
                        .unwrap_or(fixed_sum)
                };
                let share = if autos > 0 {
                    (cw - fixed_sum).max(0.0) / autos as Pt
                } else {
                    0.0
                };
                let columns: Vec<Pt> = fixed.iter().map(|v| v.unwrap_or(share)).collect();
                let mut rows = Vec::with_capacity(visible.len());
                for child in visible {
                    let rule = match child {
                        Node::Subform(s) if s.layout == Layout::Row => Width::Columns(&columns),
                        _ => Width::Forced(cw),
                    };
                    rows.push(self.node(child, rule, None, depth + 1)?);
                }
                let w = match known {
                    Some(w) if !own_columns => w,
                    _ => clamp(cw + margin.horizontal(), c.min_w, c.max_w),
                };
                Ok(finish_flow(c, margin, w, force_h, rows))
            }
        }
    }

    /// A table row: cells in successive columns, all as tall as the
    /// tallest.
    fn row(
        &mut self,
        c: &Common,
        cells: &[&Node],
        columns: &[Pt],
        force_h: Option<Pt>,
        depth: usize,
    ) -> Result<Laid, XfaError> {
        let margin = clean_margin(&c.margin);
        let n = columns.len();
        let mut slots = Vec::with_capacity(cells.len());
        let mut col = 0usize;
        for cell in cells {
            let span = (cell.common().col_span.max(1) as usize).min(n.saturating_sub(col).max(1));
            let x: Pt = columns.iter().take(col).sum();
            let rule = if col < n {
                Width::Forced(columns[col..(col + span).min(n)].iter().sum())
            } else {
                Width::Natural
            };
            slots.push((x, rule));
            col = col.saturating_add(span);
        }
        // Cells past the last column follow the previous cell.
        let mut laid = Vec::with_capacity(cells.len());
        let mut tallest: Pt = 0.0;
        for (cell, (_, rule)) in cells.iter().zip(&slots) {
            let l = self.node(cell, *rule, None, depth + 1)?;
            tallest = tallest.max(l.h);
            laid.push(l);
        }
        let h = resolve_h(c, force_h, tallest + margin.vertical());
        let inner = (h - margin.vertical()).max(0.0);
        let mut items = Vec::new();
        let w = columns.iter().sum::<Pt>() + margin.horizontal();
        if c.border.drawn() {
            items.push(Item {
                rect: Rect::new(0.0, 0.0, w, h),
                kind: ItemKind::Border(c.border.clone()),
            });
        }
        let mut next_x: Pt = 0.0;
        for ((cell, (x, rule)), l) in cells.iter().zip(&slots).zip(laid) {
            let l = if l.h + EPS < inner {
                self.node(cell, *rule, Some(inner), depth + 1)?
            } else {
                l
            };
            let x = if matches!(rule, Width::Natural) {
                next_x
            } else {
                *x
            };
            next_x = x + l.w;
            flatten(l, margin.left + x, margin.top, &mut items);
        }
        Ok(Laid::leaf(w, h, items))
    }
}

/// The most table columns laid out.
const MAX_COLUMNS: usize = 1000;

struct Container<'n> {
    common: &'n Common,
    layout: Layout,
    columns: &'n [Pt],
    children: &'n [Node],
}

/// The node's `keep`, breaks and presence applied to its measured box.
fn finish(mut laid: Laid, c: &Common) -> Laid {
    if !c.presence.drawn() {
        laid.body = Body::Leaf(Vec::new());
    }
    laid.keep = c.keep;
    laid.break_before = c.break_before;
    laid.break_after = c.break_after;
    laid
}

/// A stacked container: a flow unless its height is fixed or cut by
/// `max_h`, then a leaf.
fn finish_flow(
    c: &Common,
    margin: Margin,
    w: Pt,
    force_h: Option<Pt>,
    children: Vec<Laid>,
) -> Laid {
    let content: Pt = children.iter().map(|l| l.h).sum();
    let natural = content + margin.vertical();
    let h = resolve_h(c, force_h, natural);
    let fixed = force_h.is_some() || c.h.is_some() || h + EPS < natural;
    let flow = Flow {
        border: c.border.drawn().then(|| c.border.clone()),
        margin,
        intact: c.keep.intact,
        children: children.into_iter().map(|l| (margin.left, l)).collect(),
    };
    let laid = Laid {
        w,
        h,
        keep: Keep::default(),
        break_before: None,
        break_after: None,
        body: Body::Flow(flow),
    };
    if fixed {
        let mut items = Vec::new();
        flatten(laid, 0.0, 0.0, &mut items);
        Laid::leaf(w, h, items)
    } else {
        laid
    }
}

/// A width known without measuring the content.
fn known_w(c: &Common, width: Width<'_>) -> Option<Pt> {
    match width {
        Width::Forced(w) => Some(len(w)),
        _ if c.w.is_some() => c.w.map(len),
        Width::Fill(w) => Some(len(w)),
        Width::Columns(cols) => Some(len(cols.iter().sum())),
        Width::Natural => None,
    }
}

fn resolve_w(c: &Common, width: Width<'_>, natural: impl FnOnce() -> Pt) -> Pt {
    known_w(c, width).unwrap_or_else(|| clamp(natural(), c.min_w, c.max_w))
}

fn resolve_h(c: &Common, force_h: Option<Pt>, content: Pt) -> Pt {
    match force_h.or(c.h) {
        Some(h) => len(h),
        None => clamp(content, c.min_h, c.max_h),
    }
}

/// `content` split into the caption's rect and the value's.
fn split(content: Rect, placement: Placement, extent: Pt) -> (Rect, Rect) {
    let Rect { x, y, w, h } = content;
    match placement {
        Placement::Left | Placement::Inline => {
            let e = extent.min(w);
            (Rect::new(x, y, e, h), Rect::new(x + e, y, w - e, h))
        }
        Placement::Right => {
            let e = extent.min(w);
            (Rect::new(x + w - e, y, e, h), Rect::new(x, y, w - e, h))
        }
        Placement::Top => {
            let e = extent.min(h);
            (Rect::new(x, y, w, e), Rect::new(x, y + e, w, h - e))
        }
        Placement::Bottom => {
            let e = extent.min(h);
            (Rect::new(x, y + h - e, w, e), Rect::new(x, y, w, h - e))
        }
    }
}

// ---------------------------------------------------------------------
// Paginate

struct Master {
    width: Pt,
    height: Pt,
    /// Usable content areas (positive height).
    areas: Vec<Rect>,
    statics: Vec<Item>,
    max: Option<usize>,
}

struct Build {
    width: Pt,
    height: Pt,
    /// `None`: a fragment border dropped because nothing followed it.
    items: Vec<Option<Item>>,
}

/// An open flow fragment in the current area.
struct Frag {
    border: Option<Border>,
    dx: Pt,
    w: Pt,
    start_y: Pt,
    margin_top: Pt,
    /// This fragment's border item on the current page.
    index: Option<usize>,
    /// The first item after the border: anything from here on is content.
    first: usize,
    /// Content of this flow was left in an earlier area.
    split: bool,
}

struct Pager {
    masters: Vec<Master>,
    single: bool,
    m: usize,
    a: usize,
    used: Vec<usize>,
    pages: Vec<Build>,
    cursor: Pt,
    area_has: bool,
    page_has: bool,
    started: bool,
    frags: Vec<Frag>,
    pending: Option<Break>,
    items: usize,
    max_items: usize,
    max_pages: usize,
}

impl Pager {
    fn area(&self) -> Rect {
        self.masters[self.m].areas[self.a]
    }

    fn page_items(&mut self) -> &mut Vec<Option<Item>> {
        // A page exists from the start (`lay_out_bounded`).
        let last = self.pages.len().saturating_sub(1);
        &mut self.pages[last].items
    }

    fn count(&mut self, n: usize) -> Result<(), XfaError> {
        self.items = self.items.saturating_add(n);
        if self.items > self.max_items {
            Err(XfaError::Limit("layout items"))
        } else {
            Ok(())
        }
    }

    fn new_page(&mut self, m: usize) -> Result<(), XfaError> {
        if self.pages.len() >= self.max_pages {
            return Err(XfaError::Limit("layout pages"));
        }
        let master = &self.masters[m];
        let items: Vec<Option<Item>> = master.statics.iter().cloned().map(Some).collect();
        let n = items.len();
        self.pages.push(Build {
            width: master.width,
            height: master.height,
            items,
        });
        self.count(n)?;
        self.used[m] += 1;
        self.m = m;
        self.a = 0;
        self.cursor = self.area().y;
        self.area_has = false;
        self.page_has = false;
        Ok(())
    }

    /// The master the next page uses.
    fn next_master(&self) -> usize {
        let m = self.m;
        match self.masters[m].max {
            Some(max) if self.used[m] >= max && m + 1 < self.masters.len() => m + 1,
            _ => m,
        }
    }

    /// Moves to the next content area (or, with `page`, the next page),
    /// closing the open fragments here and opening them there.
    fn advance(&mut self, page: bool) -> Result<(), XfaError> {
        if self.single {
            return Ok(());
        }
        let bottom = self.area().bottom();
        let mut had = vec![false; self.frags.len()];
        let mut frags = std::mem::take(&mut self.frags);
        for (i, f) in frags.iter_mut().enumerate().rev() {
            let items = self.page_items();
            let content = items
                .get(f.first..)
                .is_some_and(|rest| rest.iter().any(Option::is_some));
            had[i] = content;
            if let Some(slot) = f.index.and_then(|ix| items.get_mut(ix)) {
                if content {
                    if let Some(item) = slot {
                        item.rect.h = (bottom - f.start_y).max(0.0);
                    }
                } else {
                    *slot = None;
                }
            }
            f.split |= content;
        }
        if !page && self.a + 1 < self.masters[self.m].areas.len() {
            self.a += 1;
            self.area_has = false;
        } else {
            let m = self.next_master();
            self.new_page(m)?;
        }
        let area = self.area();
        let mut y = area.y;
        for (f, had) in frags.iter_mut().zip(had) {
            f.start_y = y;
            f.index = None;
            if let Some(border) = &f.border {
                let item = Item {
                    rect: Rect::new(area.x + f.dx, y, f.w, 0.0),
                    kind: ItemKind::Border(border.clone()),
                };
                let items = self.page_items();
                items.push(Some(item));
                f.index = Some(items.len() - 1);
            }
            f.first = self.page_items().len();
            if !had {
                y += f.margin_top;
            }
        }
        self.frags = frags;
        self.cursor = y;
        Ok(())
    }

    /// Moves on when `h` does not fit the rest of the area, unless the
    /// area is empty or `h` is taller than the whole area.
    fn fit(&mut self, h: Pt) -> Result<(), XfaError> {
        if !self.area_has {
            return Ok(());
        }
        let area = self.area();
        if self.cursor + h <= area.bottom() + EPS || h > area.h + EPS {
            return Ok(());
        }
        self.advance(false)
    }

    fn request(&mut self, b: Option<Break>) {
        self.pending = match (self.pending, b) {
            (Some(Break::PageArea), _) | (_, Some(Break::PageArea)) => Some(Break::PageArea),
            (p, None) => p,
            (_, b) => b,
        };
    }

    fn apply_pending(&mut self) -> Result<(), XfaError> {
        let Some(b) = self.pending.take() else {
            return Ok(());
        };
        if !self.started {
            return Ok(());
        }
        match b {
            Break::ContentArea if self.area_has => self.advance(false),
            Break::PageArea if self.page_has => self.advance(true),
            _ => Ok(()),
        }
    }

    fn place(&mut self, laid: &Laid, dx: Pt) -> Result<(), XfaError> {
        self.request(laid.break_before);
        self.apply_pending()?;
        match &laid.body {
            Body::Leaf(items) => {
                self.fit(laid.h)?;
                let (ox, oy) = (self.area().x + dx, self.cursor);
                self.count(items.len())?;
                let page = self.page_items();
                page.extend(items.iter().map(|i| Some(i.offset(ox, oy))));
                self.cursor += laid.h;
                self.area_has = true;
                self.page_has = true;
                self.started = true;
            }
            Body::Flow(flow) => {
                if flow.intact {
                    self.fit(laid.h)?;
                }
                self.open(flow, dx, laid.w)?;
                self.cursor += flow.margin.top;
                let children = &flow.children;
                let mut i = 0;
                while i < children.len() {
                    let mut end = i + 1;
                    while end < children.len()
                        && (children[end - 1].1.keep.next || children[end].1.keep.previous)
                    {
                        end += 1;
                    }
                    if end - i > 1 {
                        let together: Pt = children[i..end].iter().map(|(_, l)| l.h).sum();
                        self.fit(together)?;
                    }
                    for (cdx, child) in &children[i..end] {
                        self.place(child, dx + cdx)?;
                    }
                    i = end;
                }
                self.close(laid.h, flow.margin.bottom);
            }
        }
        self.request(laid.break_after);
        Ok(())
    }

    fn open(&mut self, flow: &Flow, dx: Pt, w: Pt) -> Result<(), XfaError> {
        let (x, y) = (self.area().x + dx, self.cursor);
        let mut index = None;
        if let Some(border) = &flow.border {
            self.count(1)?;
            let items = self.page_items();
            items.push(Some(Item {
                rect: Rect::new(x, y, w, 0.0),
                kind: ItemKind::Border(border.clone()),
            }));
            index = Some(items.len() - 1);
        }
        let first = self.page_items().len();
        self.frags.push(Frag {
            border: flow.border.clone(),
            dx,
            w,
            start_y: y,
            margin_top: flow.margin.top,
            index,
            first,
            split: false,
        });
        Ok(())
    }

    fn close(&mut self, h: Pt, margin_bottom: Pt) {
        let Some(f) = self.frags.pop() else { return };
        let end = if f.split {
            self.cursor + margin_bottom
        } else {
            f.start_y + h
        };
        if let Some(Some(item)) = f.index.and_then(|ix| self.page_items().get_mut(ix)) {
            item.rect.h = (end - f.start_y).max(0.0);
        }
        self.cursor = end;
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{
        CheckShape, Draw, DrawUi, ExclGroup, Font, Occur, PageArea, PageSet, Presence,
    };
    use super::*;

    fn common(w: Option<Pt>, h: Option<Pt>) -> Common {
        Common {
            w,
            h,
            col_span: 1,
            ..Common::default()
        }
    }

    fn field(name: &str, w: Option<Pt>, h: Option<Pt>) -> Field {
        Field {
            common: Common {
                name: name.into(),
                ..common(w, h)
            },
            som: name.into(),
            ..Field::default()
        }
    }

    fn at(mut n: Field, x: Pt, y: Pt) -> Field {
        n.common.x = x;
        n.common.y = y;
        n
    }

    fn sub(layout: Layout, w: Option<Pt>, h: Option<Pt>, children: Vec<Node>) -> Subform {
        Subform {
            common: common(w, h),
            layout,
            children,
            ..Subform::default()
        }
    }

    fn text_draw(text: &str, w: Option<Pt>, h: Option<Pt>) -> Draw {
        Draw {
            common: common(w, h),
            value: Value::Text(text.into()),
            ..Draw::default()
        }
    }

    fn drawn_border() -> Border {
        Border {
            presence: Presence::Visible,
            ..Border::none()
        }
    }

    fn form(root: Subform, areas: Vec<PageArea>) -> Form {
        Form {
            root,
            page_set: PageSet { areas },
            ..Form::default()
        }
    }

    fn master(width: Pt, height: Pt, areas: Vec<Rect>) -> PageArea {
        PageArea {
            width,
            height,
            content_areas: areas,
            occur: Occur {
                min: 1,
                max: None,
                initial: 1,
            },
            ..PageArea::default()
        }
    }

    fn widgets(page: &Page) -> Vec<(String, Rect)> {
        page.items
            .iter()
            .filter_map(|i| match &i.kind {
                ItemKind::Widget(f) => Some((f.som.clone(), i.rect)),
                _ => None,
            })
            .collect()
    }

    fn widget(pages: &[Page], name: &str) -> (usize, Rect) {
        for (p, page) in pages.iter().enumerate() {
            if let Some((_, r)) = widgets(page).into_iter().find(|(n, _)| n == name) {
                return (p, r);
            }
        }
        panic!("no widget {name}");
    }

    fn texts(page: &Page) -> Vec<(String, Rect)> {
        page.items
            .iter()
            .filter_map(|i| match &i.kind {
                ItemKind::Text { block, .. } => Some((block.plain(), i.rect)),
                _ => None,
            })
            .collect()
    }

    fn close(a: Rect, b: Rect) -> bool {
        (a.x - b.x).abs() < 1e-6
            && (a.y - b.y).abs() < 1e-6
            && (a.w - b.w).abs() < 1e-6
            && (a.h - b.h).abs() < 1e-6
    }

    /// The items of one node placed at (0, 0) in a positioned root on a
    /// Letter page.
    fn alone(node: Node) -> Vec<Item> {
        let pages = lay_out(&form(sub(Layout::Position, None, None, vec![node]), vec![])).unwrap();
        assert_eq!(pages.len(), 1);
        pages.into_iter().next().unwrap().items
    }

    fn plain_block(text: &str, font: &Font, para: &Para) -> Block {
        text::block(&Value::Text(text.into()), font, para, &|_| None)
    }

    #[test]
    fn positioned_children_honour_anchors() {
        let mut a = at(field("a", Some(40.0), Some(20.0)), 100.0, 100.0);
        a.common.anchor = Anchor::TopLeft;
        let mut b = at(field("b", Some(40.0), Some(20.0)), 100.0, 100.0);
        b.common.anchor = Anchor::MiddleCenter;
        let mut c = at(field("c", Some(40.0), Some(20.0)), 100.0, 100.0);
        c.common.anchor = Anchor::BottomRight;
        let mut root = sub(
            Layout::Position,
            None,
            None,
            vec![Node::Field(a), Node::Field(b), Node::Field(c)],
        );
        root.common.margin = Margin::uniform(5.0);
        let pages = lay_out(&form(root, vec![])).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!((pages[0].width, pages[0].height), LETTER);
        assert!(close(
            widget(&pages, "a").1,
            Rect::new(105.0, 105.0, 40.0, 20.0)
        ));
        assert!(close(
            widget(&pages, "b").1,
            Rect::new(85.0, 95.0, 40.0, 20.0)
        ));
        assert!(close(
            widget(&pages, "c").1,
            Rect::new(65.0, 85.0, 40.0, 20.0)
        ));
    }

    #[test]
    fn tb_stacks_with_margins_and_borders() {
        let mut first = field("first", None, Some(20.0));
        first.common.margin = Margin::uniform(2.0);
        let second = field("second", Some(100.0), Some(20.0));
        let mut root = sub(
            Layout::Tb,
            None,
            None,
            vec![Node::Field(first), Node::Field(second)],
        );
        root.common.margin = Margin::uniform(10.0);
        root.common.border = drawn_border();
        let pages = lay_out(&form(root, vec![])).unwrap();
        let page = &pages[0];
        assert!(matches!(page.items[0].kind, ItemKind::Border(_)));
        assert!(close(page.items[0].rect, Rect::new(0.0, 0.0, 612.0, 60.0)));
        // The first takes the content width; its margins inset the value.
        assert!(close(
            widget(&pages, "first").1,
            Rect::new(12.0, 12.0, 588.0, 16.0)
        ));
        assert!(close(
            widget(&pages, "second").1,
            Rect::new(10.0, 30.0, 100.0, 20.0)
        ));
    }

    #[test]
    fn auto_height_follows_text() {
        let mut d = text_draw(
            "Some words that the measurer lays out\nand a second paragraph",
            Some(200.0),
            None,
        );
        d.common.margin = Margin::uniform(2.0);
        let expected = text::lay_out(
            &plain_block(&d.value.plain(), &d.font, &d.para),
            Some(196.0),
        )
        .height
            + 4.0;
        let after = field("after", Some(50.0), Some(10.0));
        let root = sub(
            Layout::Tb,
            None,
            None,
            vec![Node::Draw(d), Node::Field(after)],
        );
        let pages = lay_out(&form(root, vec![])).unwrap();
        let (_, r) = widget(&pages, "after");
        assert!((r.y - expected).abs() < 1e-6, "{} vs {expected}", r.y);
        let t = &texts(&pages[0])[0];
        assert!(close(t.1, Rect::new(2.0, 2.0, 196.0, expected - 4.0)));
    }

    #[test]
    fn min_and_max_height_bound_content() {
        let mut tall = field("tall", Some(50.0), None);
        tall.common.min_h = Some(30.0);
        let mut short = field("short", Some(50.0), None);
        short.common.max_h = Some(5.0);
        let plain = field("plain", Some(50.0), None);
        let line = text::line_height(&plain.font, &plain.para);
        let root = sub(
            Layout::Tb,
            None,
            None,
            vec![Node::Field(tall), Node::Field(short), Node::Field(plain)],
        );
        let pages = lay_out(&form(root, vec![])).unwrap();
        assert!(close(
            widget(&pages, "tall").1,
            Rect::new(0.0, 0.0, 50.0, 30.0)
        ));
        assert!(close(
            widget(&pages, "short").1,
            Rect::new(0.0, 30.0, 50.0, 5.0)
        ));
        assert!(close(
            widget(&pages, "plain").1,
            Rect::new(0.0, 35.0, 50.0, line)
        ));
    }

    fn captioned(placement: Placement, reserve: Option<Pt>) -> Field {
        let mut f = field("f", Some(200.0), Some(40.0));
        f.caption = Some(Caption {
            placement,
            reserve,
            value: Value::Text("Name".into()),
            ..Caption::default()
        });
        f
    }

    fn caption_and_widget(f: Field) -> (Option<Rect>, Rect) {
        let items = alone(Node::Field(f));
        let cap = items
            .iter()
            .find(|i| matches!(i.kind, ItemKind::Text { .. }))
            .map(|i| i.rect);
        let w = items
            .iter()
            .find(|i| matches!(i.kind, ItemKind::Widget(_)))
            .map(|i| i.rect)
            .unwrap();
        (cap, w)
    }

    #[test]
    fn captions_take_their_side() {
        let (c, w) = caption_and_widget(captioned(Placement::Left, Some(50.0)));
        assert!(close(c.unwrap(), Rect::new(0.0, 0.0, 50.0, 40.0)));
        assert!(close(w, Rect::new(50.0, 0.0, 150.0, 40.0)));
        let (c, w) = caption_and_widget(captioned(Placement::Right, Some(50.0)));
        assert!(close(c.unwrap(), Rect::new(150.0, 0.0, 50.0, 40.0)));
        assert!(close(w, Rect::new(0.0, 0.0, 150.0, 40.0)));
        let (c, w) = caption_and_widget(captioned(Placement::Top, Some(15.0)));
        assert!(close(c.unwrap(), Rect::new(0.0, 0.0, 200.0, 15.0)));
        assert!(close(w, Rect::new(0.0, 15.0, 200.0, 25.0)));
        let (c, w) = caption_and_widget(captioned(Placement::Bottom, Some(15.0)));
        assert!(close(c.unwrap(), Rect::new(0.0, 25.0, 200.0, 15.0)));
        assert!(close(w, Rect::new(0.0, 0.0, 200.0, 25.0)));
    }

    #[test]
    fn captions_without_reserve_measure_their_text() {
        let f = captioned(Placement::Left, None);
        let cap = f.caption.clone().unwrap();
        let natural = text::lay_out(&plain_block("Name", &cap.font, &cap.para), None).width + 2.0;
        let expected = natural.min(100.0);
        let (c, w) = caption_and_widget(f);
        assert!(close(c.unwrap(), Rect::new(0.0, 0.0, expected, 40.0)));
        assert!(close(w, Rect::new(expected, 0.0, 200.0 - expected, 40.0)));
        // Inline: the natural width, not capped at half.
        let (c, _) = caption_and_widget(captioned(Placement::Inline, None));
        assert!(close(
            c.unwrap(),
            Rect::new(0.0, 0.0, natural.min(200.0), 40.0)
        ));
        // Top: the caption's laid-out height.
        let top_h = text::lay_out(&plain_block("Name", &cap.font, &cap.para), Some(200.0)).height;
        let (c, w) = caption_and_widget(captioned(Placement::Top, None));
        assert!(close(c.unwrap(), Rect::new(0.0, 0.0, 200.0, top_h)));
        assert!(close(w, Rect::new(0.0, top_h, 200.0, 40.0 - top_h)));
        // A hidden caption takes nothing; an invisible one its space only.
        let mut hidden = captioned(Placement::Left, Some(50.0));
        hidden.caption.as_mut().unwrap().presence = Presence::Hidden;
        let (c, w) = caption_and_widget(hidden);
        assert!(c.is_none());
        assert!(close(w, Rect::new(0.0, 0.0, 200.0, 40.0)));
        let mut invisible = captioned(Placement::Left, Some(50.0));
        invisible.caption.as_mut().unwrap().presence = Presence::Invisible;
        let (c, w) = caption_and_widget(invisible);
        assert!(c.is_none());
        assert!(close(w, Rect::new(50.0, 0.0, 150.0, 40.0)));
    }

    #[test]
    fn caption_margins_inset_the_caption_text() {
        let mut f = captioned(Placement::Left, Some(50.0));
        f.caption.as_mut().unwrap().margin = Margin::uniform(3.0);
        let (c, _) = caption_and_widget(f);
        assert!(close(c.unwrap(), Rect::new(3.0, 3.0, 44.0, 34.0)));
    }

    fn check(name: &str) -> Field {
        let mut f = field(name, Some(100.0), Some(20.0));
        f.ui = Ui::CheckButton {
            shape: CheckShape::Square,
            mark: Default::default(),
            size: 10.0,
            border: drawn_border(),
        };
        f
    }

    #[test]
    fn check_button_square_and_widget_share_a_rect() {
        let mut f = check("box");
        f.para.h_align = HAlign::Center;
        f.para.v_align = VAlign::Middle;
        f.checked = true;
        let items = alone(Node::Field(f));
        assert_eq!(items.len(), 2);
        assert!(matches!(
            items[0].kind,
            ItemKind::Check {
                checked: true,
                round: false,
                ..
            }
        ));
        assert!(matches!(items[1].kind, ItemKind::Widget(_)));
        assert!(close(items[0].rect, Rect::new(45.0, 5.0, 10.0, 10.0)));
        assert_eq!(items[0].rect, items[1].rect);
        // Read-only: the box, no widget.
        let mut ro = check("ro");
        ro.common.read_only = true;
        let items = alone(Node::Field(ro));
        assert_eq!(items.len(), 1);
        assert!(matches!(items[0].kind, ItemKind::Check { .. }));
    }

    #[test]
    fn non_fillable_fields_draw_text_and_no_widget() {
        let mut f = field("ro", Some(100.0), Some(20.0));
        f.common.read_only = true;
        f.value = Value::Text("shown".into());
        f.common.border = drawn_border();
        let items = alone(Node::Field(f));
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0].kind, ItemKind::Border(_)));
        match &items[1].kind {
            ItemKind::Text { block, .. } => assert_eq!(block.plain(), "shown"),
            other => panic!("{other:?}"),
        }
        // A button draws its caption over its whole content, nothing else.
        let mut b = field("btn", Some(80.0), Some(20.0));
        b.ui = Ui::Button {
            highlight: String::new(),
        };
        b.caption = Some(Caption {
            value: Value::Text("Add".into()),
            ..Caption::default()
        });
        let items = alone(Node::Field(b.clone()));
        assert_eq!(items.len(), 1);
        assert!(close(items[0].rect, Rect::new(0.0, 0.0, 80.0, 20.0)));
        // One with a click event (an add-row button) gets a widget over it all.
        b.events.push(super::super::types::Event {
            activity: "click".into(),
            ..Default::default()
        });
        let items = alone(Node::Field(b.clone()));
        assert_eq!(items.len(), 2);
        assert!(matches!(items[1].kind, ItemKind::Widget(_)));
        assert!(close(items[1].rect, Rect::new(0.0, 0.0, 80.0, 20.0)));
        b.common.read_only = true;
        assert_eq!(alone(Node::Field(b)).len(), 1, "a read-only one does not");
        let mut bare = field("bare", Some(80.0), Some(20.0));
        bare.ui = Ui::Signature;
        assert!(alone(Node::Field(bare)).is_empty());
    }

    #[test]
    fn presence_hidden_invisible_inactive() {
        let mut hidden = field("hidden", Some(50.0), Some(10.0));
        hidden.common.presence = Presence::Hidden;
        let mut invisible = field("invisible", Some(50.0), Some(10.0));
        invisible.common.presence = Presence::Invisible;
        let mut inactive = field("inactive", Some(50.0), Some(10.0));
        inactive.common.presence = Presence::Inactive;
        let last = field("last", Some(50.0), Some(10.0));
        let root = sub(
            Layout::Tb,
            None,
            None,
            vec![
                Node::Field(hidden),
                Node::Field(invisible),
                Node::Field(inactive),
                Node::Field(last),
            ],
        );
        let pages = lay_out(&form(root, vec![])).unwrap();
        let names: Vec<String> = widgets(&pages[0]).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["last".to_string()]);
        assert!(close(
            widget(&pages, "last").1,
            Rect::new(0.0, 10.0, 50.0, 10.0)
        ));
    }

    #[test]
    fn lr_wraps_to_a_new_line() {
        let lr = sub(
            Layout::Lr,
            Some(100.0),
            None,
            vec![
                Node::Field(field("a", Some(40.0), Some(10.0))),
                Node::Field(field("b", Some(40.0), Some(20.0))),
                Node::Field(field("c", Some(40.0), Some(10.0))),
            ],
        );
        let after = field("after", Some(10.0), Some(10.0));
        let root = sub(
            Layout::Tb,
            None,
            None,
            vec![Node::Subform(lr), Node::Field(after)],
        );
        let pages = lay_out(&form(root, vec![])).unwrap();
        assert!(close(
            widget(&pages, "a").1,
            Rect::new(0.0, 0.0, 40.0, 10.0)
        ));
        assert!(close(
            widget(&pages, "b").1,
            Rect::new(40.0, 0.0, 40.0, 20.0)
        ));
        assert!(close(
            widget(&pages, "c").1,
            Rect::new(0.0, 20.0, 40.0, 10.0)
        ));
        assert!(close(
            widget(&pages, "after").1,
            Rect::new(0.0, 30.0, 10.0, 10.0)
        ));
    }

    fn row(cells: Vec<Node>) -> Node {
        Node::Subform(sub(Layout::Row, None, None, cells))
    }

    #[test]
    fn table_columns_spans_and_row_heights() {
        let r1 = row(vec![
            Node::Field(field("r1a", None, Some(10.0))),
            Node::Field(field("r1b", None, Some(20.0))),
            Node::Field(field("r1c", None, Some(10.0))),
        ]);
        let mut span = field("r2a", None, Some(10.0));
        span.common.col_span = 2;
        let r2 = row(vec![
            Node::Field(span),
            Node::Field(field("r2b", None, Some(10.0))),
        ]);
        let other = Node::Field(field("wide", Some(10.0), Some(10.0)));
        let mut table = sub(Layout::Table, Some(300.0), None, vec![r1, r2, other]);
        table.column_widths = vec![100.0, -1.0, 50.0];
        let root = sub(Layout::Tb, None, None, vec![Node::Subform(table)]);
        let pages = lay_out(&form(root, vec![])).unwrap();
        // Cells stretch to the row's tallest.
        assert!(close(
            widget(&pages, "r1a").1,
            Rect::new(0.0, 0.0, 100.0, 20.0)
        ));
        assert!(close(
            widget(&pages, "r1b").1,
            Rect::new(100.0, 0.0, 150.0, 20.0)
        ));
        assert!(close(
            widget(&pages, "r1c").1,
            Rect::new(250.0, 0.0, 50.0, 20.0)
        ));
        assert!(close(
            widget(&pages, "r2a").1,
            Rect::new(0.0, 20.0, 250.0, 10.0)
        ));
        assert!(close(
            widget(&pages, "r2b").1,
            Rect::new(250.0, 20.0, 50.0, 10.0)
        ));
        assert!(close(
            widget(&pages, "wide").1,
            Rect::new(0.0, 30.0, 300.0, 10.0)
        ));
    }

    #[test]
    fn table_missing_columns_share_the_rest() {
        let r = row(vec![
            Node::Field(field("a", None, Some(10.0))),
            Node::Field(field("b", None, Some(10.0))),
            Node::Field(field("c", None, Some(10.0))),
        ]);
        let mut table = sub(Layout::Table, Some(200.0), None, vec![r]);
        table.column_widths = vec![100.0];
        let pages = lay_out(&form(
            sub(Layout::Tb, None, None, vec![Node::Subform(table)]),
            vec![],
        ))
        .unwrap();
        assert!(close(
            widget(&pages, "b").1,
            Rect::new(100.0, 0.0, 50.0, 10.0)
        ));
        assert!(close(
            widget(&pages, "c").1,
            Rect::new(150.0, 0.0, 50.0, 10.0)
        ));
    }

    #[test]
    fn excl_group_lays_out_round_buttons() {
        let mut a = check("a");
        let mut b = check("b");
        for f in [&mut a, &mut b] {
            f.common.h = Some(10.0);
            f.ui = Ui::CheckButton {
                shape: CheckShape::Round,
                mark: Default::default(),
                size: 10.0,
                border: Border::none(),
            };
            f.group = Some("g".into());
        }
        let group = ExclGroup {
            common: common(None, None),
            layout: Layout::Tb,
            fields: vec![a, b],
            ..ExclGroup::default()
        };
        let root = sub(Layout::Tb, None, None, vec![Node::ExclGroup(group)]);
        let pages = lay_out(&form(root, vec![])).unwrap();
        let checks: Vec<(bool, Rect)> = pages[0]
            .items
            .iter()
            .filter_map(|i| match i.kind {
                ItemKind::Check { round, .. } => Some((round, i.rect)),
                _ => None,
            })
            .collect();
        assert_eq!(checks.len(), 2);
        assert!(checks.iter().all(|(round, _)| *round));
        assert!(close(
            widget(&pages, "a").1,
            Rect::new(0.0, 0.0, 10.0, 10.0)
        ));
        assert!(close(
            widget(&pages, "b").1,
            Rect::new(0.0, 10.0, 10.0, 10.0)
        ));
    }

    fn small_page() -> Vec<PageArea> {
        vec![master(
            200.0,
            200.0,
            vec![Rect::new(10.0, 10.0, 100.0, 100.0)],
        )]
    }

    fn fields(prefix: &str, n: usize, h: Pt) -> Vec<Node> {
        (0..n)
            .map(|i| Node::Field(field(&format!("{prefix}{i}"), None, Some(h))))
            .collect()
    }

    #[test]
    fn overflowing_content_continues_on_a_second_page() {
        let root = sub(Layout::Tb, None, None, fields("f", 15, 10.0));
        let pages = lay_out(&form(root, small_page())).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(widgets(&pages[0]).len(), 10);
        assert_eq!(widgets(&pages[1]).len(), 5);
        assert!(close(
            widget(&pages, "f10").1,
            Rect::new(10.0, 10.0, 100.0, 10.0)
        ));
        assert!(close(
            widget(&pages, "f9").1,
            Rect::new(10.0, 100.0, 100.0, 10.0)
        ));
    }

    #[test]
    fn keep_intact_moves_a_subform_whole() {
        let mut intact = sub(Layout::Tb, None, None, fields("k", 2, 30.0));
        intact.common.keep.intact = true;
        let root = sub(
            Layout::Tb,
            None,
            None,
            vec![
                Node::Field(field("top", None, Some(60.0))),
                Node::Subform(intact),
            ],
        );
        let pages = lay_out(&form(root, small_page())).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(
            widget(&pages, "k0"),
            (1, Rect::new(10.0, 10.0, 100.0, 30.0))
        );
        assert_eq!(widget(&pages, "k1").0, 1);
        // Without intact it splits at the child boundary.
        let loose = sub(Layout::Tb, None, None, fields("k", 2, 30.0));
        let root = sub(
            Layout::Tb,
            None,
            None,
            vec![
                Node::Field(field("top", None, Some(60.0))),
                Node::Subform(loose),
            ],
        );
        let pages = lay_out(&form(root, small_page())).unwrap();
        assert_eq!(widget(&pages, "k0").0, 0);
        assert_eq!(
            widget(&pages, "k1"),
            (1, Rect::new(10.0, 10.0, 100.0, 30.0))
        );
    }

    #[test]
    fn keep_next_moves_both() {
        let mut second = field("second", None, Some(30.0));
        second.common.keep.next = true;
        let root = sub(
            Layout::Tb,
            None,
            None,
            vec![
                Node::Field(field("first", None, Some(50.0))),
                Node::Field(second),
                Node::Field(field("third", None, Some(30.0))),
            ],
        );
        let pages = lay_out(&form(root, small_page())).unwrap();
        assert_eq!(
            widget(&pages, "second"),
            (1, Rect::new(10.0, 10.0, 100.0, 30.0))
        );
        assert_eq!(widget(&pages, "third").0, 1);
    }

    #[test]
    fn a_table_splits_between_rows_with_a_border_per_fragment() {
        let rows: Vec<Node> = (0..5)
            .map(|i| row(vec![Node::Field(field(&format!("c{i}"), None, Some(30.0)))]))
            .collect();
        let mut table = sub(Layout::Table, None, None, rows);
        table.column_widths = vec![-1.0];
        table.common.border = drawn_border();
        let root = sub(Layout::Tb, None, None, vec![Node::Subform(table)]);
        let pages = lay_out(&form(root, small_page())).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(widget(&pages, "c2").0, 0);
        assert_eq!(
            widget(&pages, "c3"),
            (1, Rect::new(10.0, 10.0, 100.0, 30.0))
        );
        for (p, h) in [(0, 100.0), (1, 60.0)] {
            let borders: Vec<Rect> = pages[p]
                .items
                .iter()
                .filter(|i| matches!(i.kind, ItemKind::Border(_)))
                .map(|i| i.rect)
                .collect();
            assert_eq!(borders.len(), 1);
            assert!(
                close(borders[0], Rect::new(10.0, 10.0, 100.0, h)),
                "{borders:?}"
            );
        }
    }

    #[test]
    fn break_before_starts_a_page_but_not_the_first() {
        let mut first = field("first", None, Some(10.0));
        first.common.break_before = Some(Break::PageArea);
        let mut second = field("second", None, Some(10.0));
        second.common.break_before = Some(Break::PageArea);
        let mut third = field("third", None, Some(10.0));
        third.common.break_after = Some(Break::ContentArea);
        let root = sub(
            Layout::Tb,
            None,
            None,
            vec![Node::Field(first), Node::Field(second), Node::Field(third)],
        );
        let pages = lay_out(&form(root, small_page())).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(widget(&pages, "first").0, 0);
        assert_eq!(
            widget(&pages, "second"),
            (1, Rect::new(10.0, 10.0, 100.0, 10.0))
        );
        assert_eq!(widget(&pages, "third").0, 1);
    }

    #[test]
    fn two_content_areas_fill_in_order() {
        let areas = vec![master(
            300.0,
            100.0,
            vec![
                Rect::new(0.0, 0.0, 100.0, 50.0),
                Rect::new(150.0, 10.0, 100.0, 50.0),
            ],
        )];
        let root = sub(Layout::Tb, None, None, fields("f", 6, 20.0));
        let pages = lay_out(&form(root, areas)).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(widget(&pages, "f1"), (0, Rect::new(0.0, 20.0, 100.0, 20.0)));
        assert_eq!(
            widget(&pages, "f2"),
            (0, Rect::new(150.0, 10.0, 100.0, 20.0))
        );
        assert_eq!(
            widget(&pages, "f3"),
            (0, Rect::new(150.0, 30.0, 100.0, 20.0))
        );
        assert_eq!(widget(&pages, "f4"), (1, Rect::new(0.0, 0.0, 100.0, 20.0)));
    }

    #[test]
    fn a_master_used_up_hands_over_to_the_next() {
        let mut first = master(200.0, 200.0, vec![Rect::new(0.0, 0.0, 100.0, 50.0)]);
        first.occur.max = Some(1);
        let second = master(300.0, 300.0, vec![Rect::new(5.0, 5.0, 100.0, 50.0)]);
        let root = sub(Layout::Tb, None, None, fields("f", 6, 20.0));
        let pages = lay_out(&form(root, vec![first, second])).unwrap();
        assert_eq!(pages.len(), 3);
        assert_eq!(
            (pages[0].width, pages[1].width, pages[2].width),
            (200.0, 300.0, 300.0)
        );
        assert_eq!(widget(&pages, "f2"), (1, Rect::new(5.0, 5.0, 100.0, 20.0)));
    }

    #[test]
    fn statics_come_first_on_every_page() {
        let mut m = small_page().remove(0);
        let mut header = text_draw("Header", Some(80.0), Some(8.0));
        header.common.x = 5.0;
        header.common.y = 1.0;
        m.statics = vec![Node::Draw(header)];
        let root = sub(Layout::Tb, None, None, fields("f", 15, 10.0));
        let pages = lay_out(&form(root, vec![m])).unwrap();
        assert_eq!(pages.len(), 2);
        for page in &pages {
            match &page.items[0].kind {
                ItemKind::Text { block, .. } => assert_eq!(block.plain(), "Header"),
                other => panic!("{other:?}"),
            }
            assert!(close(page.items[0].rect, Rect::new(5.0, 1.0, 80.0, 8.0)));
        }
    }

    #[test]
    fn page_and_item_limits_stop_the_layout() {
        let root = sub(Layout::Tb, None, None, fields("f", 30, 10.0));
        let one_row = vec![master(100.0, 100.0, vec![Rect::new(0.0, 0.0, 100.0, 10.0)])];
        let f = form(root, one_row);
        assert_eq!(
            lay_out_bounded(&f, 10, 1000),
            Err(XfaError::Limit("layout pages"))
        );
        assert_eq!(lay_out_bounded(&f, 30, 1000).unwrap().len(), 30);
        assert_eq!(
            lay_out_bounded(&f, 1000, 20),
            Err(XfaError::Limit("layout items"))
        );
    }

    #[test]
    fn zero_height_areas_are_skipped() {
        let areas = vec![master(
            200.0,
            200.0,
            vec![
                Rect::new(0.0, 0.0, 100.0, 0.0),
                Rect::new(0.0, 0.0, 100.0, -5.0),
                Rect::new(0.0, 20.0, 100.0, 50.0),
            ],
        )];
        let root = sub(Layout::Tb, None, None, fields("f", 4, 20.0));
        let pages = lay_out(&form(root.clone(), areas)).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(widget(&pages, "f0"), (0, Rect::new(0.0, 20.0, 100.0, 20.0)));
        // No usable area: one page, everything on it.
        let none = vec![master(200.0, 200.0, vec![Rect::new(0.0, 0.0, 100.0, 0.0)])];
        let pages = lay_out(&form(root, none)).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(widgets(&pages[0]).len(), 4);
        assert_eq!(widget(&pages, "f3").1, Rect::new(0.0, 60.0, 200.0, 20.0));
    }

    #[test]
    fn nan_and_negative_sizes_do_not_panic() {
        let mut a = field("a", Some(Pt::NAN), Some(Pt::INFINITY));
        a.common.margin = Margin::uniform(Pt::NAN);
        a.common.x = Pt::NAN;
        a.common.min_h = Some(Pt::NAN);
        let mut b = field("b", Some(-10.0), Some(-5.0));
        b.font.size = Pt::NAN;
        b.value = Value::Text("x".into());
        let mut c = check("c");
        c.ui = Ui::CheckButton {
            shape: CheckShape::Square,
            mark: Default::default(),
            size: Pt::NAN,
            border: Border::none(),
        };
        let mut t = sub(
            Layout::Table,
            Some(Pt::NAN),
            None,
            vec![row(vec![Node::Field(field("d", None, None))])],
        );
        t.column_widths = vec![Pt::NAN, -1.0];
        let root = sub(
            Layout::Tb,
            None,
            None,
            vec![
                Node::Field(a),
                Node::Field(b),
                Node::Field(c),
                Node::Subform(t),
            ],
        );
        let areas = vec![master(
            Pt::NAN,
            100.0,
            vec![Rect::new(Pt::NAN, 0.0, 100.0, 50.0)],
        )];
        let pages = lay_out(&form(root, areas)).unwrap();
        for page in &pages {
            for item in &page.items {
                let r = item.rect;
                assert!(
                    r.x.is_finite() && r.y.is_finite() && r.w.is_finite() && r.h.is_finite(),
                    "{item:?}"
                );
                assert!(r.w >= 0.0 && r.h >= 0.0);
            }
        }
    }

    #[test]
    fn deep_nesting_is_refused() {
        let mut node = sub(
            Layout::Tb,
            None,
            None,
            vec![Node::Field(field("leaf", None, Some(1.0)))],
        );
        for _ in 0..300 {
            node = sub(Layout::Tb, None, None, vec![Node::Subform(node)]);
        }
        assert_eq!(
            lay_out(&form(node, vec![])),
            Err(XfaError::Limit("layout depth"))
        );
    }

    #[test]
    fn an_empty_form_is_one_empty_page() {
        let pages = lay_out(&Form::default()).unwrap();
        assert_eq!(pages.len(), 1);
        assert!(pages[0].items.is_empty());
        let mut hidden = sub(Layout::Tb, None, None, fields("f", 3, 10.0));
        hidden.common.presence = Presence::Hidden;
        assert!(lay_out(&form(hidden, vec![])).unwrap()[0].items.is_empty());
    }

    #[test]
    fn draws_emit_their_shapes() {
        let line = Draw {
            common: common(Some(100.0), Some(2.0)),
            ui: DrawUi::Line {
                slope_down: false,
                edge: Edge::default(),
            },
            ..Draw::default()
        };
        let rect = Draw {
            common: common(Some(50.0), Some(50.0)),
            ui: DrawUi::Rectangle {
                border: drawn_border(),
            },
            ..Draw::default()
        };
        let image = Draw {
            common: common(Some(10.0), Some(10.0)),
            ui: DrawUi::Image,
            ..Draw::default()
        };
        let root = sub(
            Layout::Tb,
            None,
            None,
            vec![Node::Draw(line), Node::Draw(rect), Node::Draw(image)],
        );
        let pages = lay_out(&form(root, vec![])).unwrap();
        let items = &pages[0].items;
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0].kind, ItemKind::Line { .. }));
        assert!(close(items[1].rect, Rect::new(0.0, 2.0, 50.0, 50.0)));
    }

    #[test]
    fn an_empty_fragment_border_is_dropped() {
        let mut inner = sub(Layout::Tb, None, None, fields("k", 1, 30.0));
        inner.common.border = drawn_border();
        let root = sub(
            Layout::Tb,
            None,
            None,
            vec![
                Node::Field(field("top", None, Some(90.0))),
                Node::Subform(inner),
            ],
        );
        let pages = lay_out(&form(root, small_page())).unwrap();
        assert_eq!(pages.len(), 2);
        assert!(
            !pages[0]
                .items
                .iter()
                .any(|i| matches!(i.kind, ItemKind::Border(_)))
        );
        let border = pages[1]
            .items
            .iter()
            .find(|i| matches!(i.kind, ItemKind::Border(_)))
            .unwrap();
        assert!(close(border.rect, Rect::new(10.0, 10.0, 100.0, 30.0)));
    }
}
