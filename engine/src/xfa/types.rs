// SPDX-License-Identifier: MIT
//! The XFA form model: what `template` reads from the template packet and
//! `bind` completes from the datasets packet, what `layout` places and
//! `emit` draws. Lengths are points; rectangles are y-down from the top-left
//! corner of their container, as XFA measures them.
//!
//! Every struct here derives `Clone` and `Debug`. Unknown elements and
//! attributes are ignored by the reader, so the model holds what the
//! renderer uses and nothing else.

use std::fmt;

/// A length in points.
pub type Pt = f64;

/// Reads an XFA measurement (`12.7mm`, `8.5in`, `10pt`, `1cm`, `0.5em` is
/// not a length here, a bare number is points) into points. `None` for
/// anything else.
pub fn length(text: &str) -> Option<Pt> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == '+'))
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(split);
    let value: f64 = number.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    let factor = match unit.trim() {
        "" | "pt" => 1.0,
        "mm" => 72.0 / 25.4,
        "cm" => 72.0 / 2.54,
        "in" => 72.0,
        "px" => 0.75,
        "mp" => 0.001,
        _ => return None,
    };
    Some(value * factor)
}

/// An RGB colour, each channel 0 to 255 (XFA `value="r,g,b"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const BLACK: Rgb = Rgb(0, 0, 0);
    pub const WHITE: Rgb = Rgb(255, 255, 255);

    /// `"r,g,b"`; `None` for anything else.
    pub fn parse(text: &str) -> Option<Rgb> {
        let mut parts = text.split(',').map(|p| p.trim().parse::<u8>().ok());
        let r = parts.next()??;
        let g = parts.next()??;
        let b = parts.next()??;
        Some(Rgb(r, g, b))
    }
}

/// A rectangle in points, y-down.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: Pt,
    pub y: Pt,
    pub w: Pt,
    pub h: Pt,
}

impl Rect {
    pub fn new(x: Pt, y: Pt, w: Pt, h: Pt) -> Self {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> Pt {
        self.x + self.w
    }
    pub fn bottom(&self) -> Pt {
        self.y + self.h
    }
    /// This rectangle shrunk by `m` on each side (never below zero size).
    pub fn inset(&self, m: &Margin) -> Rect {
        Rect {
            x: self.x + m.left,
            y: self.y + m.top,
            w: (self.w - m.left - m.right).max(0.0),
            h: (self.h - m.top - m.bottom).max(0.0),
        }
    }
    /// This rectangle moved by (`dx`, `dy`).
    pub fn offset(&self, dx: Pt, dy: Pt) -> Rect {
        Rect {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }
}

/// XFA `presence`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Presence {
    #[default]
    Visible,
    /// Takes no space and is not drawn.
    Hidden,
    /// Takes its space but is not drawn.
    Invisible,
    /// Hidden, and never interactive.
    Inactive,
}

impl Presence {
    pub fn parse(text: &str) -> Presence {
        match text {
            "hidden" => Presence::Hidden,
            "invisible" => Presence::Invisible,
            "inactive" => Presence::Inactive,
            _ => Presence::Visible,
        }
    }
    /// Takes space in the layout.
    pub fn occupies(self) -> bool {
        matches!(self, Presence::Visible | Presence::Invisible)
    }
    /// Is drawn.
    pub fn drawn(self) -> bool {
        self == Presence::Visible
    }
}

/// Subform `layout`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Layout {
    /// Children placed at their own x/y.
    #[default]
    Position,
    /// Top to bottom, each child below the previous.
    Tb,
    /// Left to right, wrapping to a new line when the row is full.
    Lr,
    /// Right to left (laid out as `Lr` for now).
    Rl,
    /// A table: every child is a row (or a `row` subform), columns from
    /// `column_widths`.
    Table,
    /// One table row: children side by side in the table's columns.
    Row,
}

impl Layout {
    pub fn parse(text: &str) -> Layout {
        match text {
            "tb" => Layout::Tb,
            "lr-tb" | "lr" => Layout::Lr,
            "rl-tb" | "rl" => Layout::Rl,
            "table" => Layout::Table,
            "row" => Layout::Row,
            _ => Layout::Position,
        }
    }
}

/// Insets, in points.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Margin {
    pub left: Pt,
    pub right: Pt,
    pub top: Pt,
    pub bottom: Pt,
}

impl Margin {
    pub fn uniform(v: Pt) -> Self {
        Margin {
            left: v,
            right: v,
            top: v,
            bottom: v,
        }
    }
    pub fn horizontal(&self) -> Pt {
        self.left + self.right
    }
    pub fn vertical(&self) -> Pt {
        self.top + self.bottom
    }
}

/// Horizontal alignment of text (`para hAlign`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HAlign {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

/// Vertical alignment of text (`para vAlign`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

/// Paragraph settings (`para`). Margins are in points; `line_height` is
/// `None` for the font's own.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Para {
    pub h_align: HAlign,
    pub v_align: VAlign,
    pub line_height: Option<Pt>,
    pub space_above: Pt,
    pub space_below: Pt,
    pub margin_left: Pt,
    pub margin_right: Pt,
    pub text_indent: Pt,
}

/// A font request (`font`). `family` is the typeface as written (`Arial`,
/// `Myriad Pro`); `text::face_for` maps it to a base 14 face.
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub family: String,
    pub size: Pt,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub line_through: bool,
    pub color: Rgb,
    /// Extra advance after each glyph, in points (`letterSpacing`).
    pub letter_spacing: Pt,
}

impl Default for Font {
    fn default() -> Self {
        Font {
            family: "Arial".to_string(),
            size: 10.0,
            bold: false,
            italic: false,
            underline: false,
            line_through: false,
            color: Rgb::BLACK,
            letter_spacing: 0.0,
        }
    }
}

/// How an edge is stroked (`edge stroke`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Stroke {
    #[default]
    Solid,
    Dashed,
    Dotted,
    DashDot,
    DashDotDot,
    /// Not drawn.
    None,
    /// Drawn as `Solid` (lowered, raised, etched, embossed).
    Other,
}

impl Stroke {
    pub fn parse(text: &str) -> Stroke {
        match text {
            "solid" => Stroke::Solid,
            "dashed" => Stroke::Dashed,
            "dotted" => Stroke::Dotted,
            "dashDot" => Stroke::DashDot,
            "dashDotDot" => Stroke::DashDotDot,
            "none" => Stroke::None,
            _ => Stroke::Other,
        }
    }
}

/// One edge of a border (`edge`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edge {
    pub presence: Presence,
    pub thickness: Pt,
    pub stroke: Stroke,
    pub color: Rgb,
}

impl Default for Edge {
    fn default() -> Self {
        Edge {
            presence: Presence::Visible,
            thickness: 0.5,
            stroke: Stroke::Solid,
            color: Rgb::BLACK,
        }
    }
}

impl Edge {
    /// Draws a visible line.
    pub fn drawn(&self) -> bool {
        self.presence.drawn() && self.stroke != Stroke::None && self.thickness > 0.0
    }
}

/// A fill (`fill`): a solid colour for now (patterns and gradients draw
/// their first colour).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fill {
    pub presence: Presence,
    pub color: Rgb,
}

impl Default for Fill {
    fn default() -> Self {
        Fill {
            presence: Presence::Visible,
            color: Rgb::WHITE,
        }
    }
}

/// A border (`border`): edges in XFA order (top, right, bottom, left), an
/// optional fill, and a corner radius. A `border` element with no `edge`
/// children has four default edges; one with fewer than four repeats the
/// last. A missing `border` element is `Border::none()`.
#[derive(Clone, Debug, PartialEq)]
pub struct Border {
    pub presence: Presence,
    pub edges: [Edge; 4],
    pub fill: Option<Fill>,
    pub corner_radius: Pt,
    /// `hand`: `even` (centred on the boundary), `left` (inside) or
    /// `right` (outside). Drawn inside for now.
    pub inside: bool,
}

impl Border {
    /// No border element: nothing drawn.
    pub fn none() -> Self {
        Border {
            presence: Presence::Hidden,
            edges: [Edge::default(); 4],
            fill: None,
            corner_radius: 0.0,
            inside: true,
        }
    }
    pub fn drawn(&self) -> bool {
        self.presence.drawn()
            && (self.edges.iter().any(Edge::drawn) || self.fill.is_some_and(|f| f.presence.drawn()))
    }
    pub fn top(&self) -> &Edge {
        &self.edges[0]
    }
    pub fn right(&self) -> &Edge {
        &self.edges[1]
    }
    pub fn bottom(&self) -> &Edge {
        &self.edges[2]
    }
    pub fn left(&self) -> &Edge {
        &self.edges[3]
    }
}

impl Default for Border {
    fn default() -> Self {
        Border::none()
    }
}

/// `keep`: what stays together across pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Keep {
    /// `intact="contentArea"` or `"pageArea"`: the node is not split.
    pub intact: bool,
    /// `previous="contentArea"`: stays with the node before it.
    pub previous: bool,
    /// `next="contentArea"`: stays with the node after it.
    pub next: bool,
}

/// `breakBefore` / `breakAfter` targeting a new content area or page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Break {
    ContentArea,
    PageArea,
}

/// `anchorType`: which point of the node `x`/`y` place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Anchor {
    #[default]
    TopLeft,
    TopCenter,
    TopRight,
    MiddleLeft,
    MiddleCenter,
    MiddleRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl Anchor {
    pub fn parse(text: &str) -> Anchor {
        match text {
            "topCenter" => Anchor::TopCenter,
            "topRight" => Anchor::TopRight,
            "middleLeft" => Anchor::MiddleLeft,
            "middleCenter" => Anchor::MiddleCenter,
            "middleRight" => Anchor::MiddleRight,
            "bottomLeft" => Anchor::BottomLeft,
            "bottomCenter" => Anchor::BottomCenter,
            "bottomRight" => Anchor::BottomRight,
            _ => Anchor::TopLeft,
        }
    }
}

/// What every container, field and draw carries.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Common {
    /// `name`; empty when unnamed.
    pub name: String,
    /// `id`; empty when none. Floating fields are referenced by it from
    /// rich text (`xfa:embed`).
    pub id: String,
    /// Position inside the parent, for `Layout::Position` parents.
    pub x: Pt,
    pub y: Pt,
    pub anchor: Anchor,
    /// `w`/`h`: a fixed size. `None` grows to the content (`min_w`,
    /// `min_h`, `max_w`, `max_h` bound it) or, for width in a flow, fills
    /// the parent.
    pub w: Option<Pt>,
    pub h: Option<Pt>,
    pub min_w: Option<Pt>,
    pub min_h: Option<Pt>,
    pub max_w: Option<Pt>,
    pub max_h: Option<Pt>,
    pub presence: Presence,
    /// `relevant="-print"`: shown on screen, left off print. Drawn here.
    pub screen_only: bool,
    pub margin: Margin,
    pub border: Border,
    pub keep: Keep,
    pub break_before: Option<Break>,
    pub break_after: Option<Break>,
    /// `colSpan` inside a `Layout::Row`.
    pub col_span: u32,
    /// `rotate`, degrees counter-clockwise (0, 90, 180, 270). Only 0 is
    /// laid out exactly; others are drawn unrotated.
    pub rotate: i32,
    /// `assist/toolTip`.
    pub tooltip: String,
    /// `assist/speak`, read when there is no tooltip.
    pub speak: String,
    /// `access`: `readOnly`, `protected` or `nonInteractive` make the field
    /// non-fillable.
    pub read_only: bool,
}

/// A field's or draw's value (`value`).
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Empty,
    /// `text`, `integer`, `decimal`, `float`, `date`, `time`, `dateTime`
    /// and `boolean` as their string content, trimmed of surrounding
    /// whitespace except for `text` (whose `maxChars` the field keeps).
    Text(String),
    /// `exData contentType="text/html"`: the XHTML body as written,
    /// `richtext::parse` reads it.
    Html(String),
    /// An image (`image`); not drawn.
    Image,
}

impl Value {
    pub fn is_empty(&self) -> bool {
        match self {
            Value::Empty | Value::Image => true,
            Value::Text(t) => t.is_empty(),
            Value::Html(h) => h.is_empty(),
        }
    }
    /// The value's plain text, rich text with its tags removed.
    pub fn plain(&self) -> String {
        match self {
            Value::Text(t) => t.clone(),
            Value::Html(h) => crate::xfa::richtext::plain_text(h),
            _ => String::new(),
        }
    }
}

/// A caption's placement (`caption placement`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Placement {
    #[default]
    Left,
    Right,
    Top,
    Bottom,
    /// Inline: before the value in the same box.
    Inline,
}

impl Placement {
    pub fn parse(text: &str) -> Placement {
        match text {
            "right" => Placement::Right,
            "top" => Placement::Top,
            "bottom" => Placement::Bottom,
            "inline" => Placement::Inline,
            _ => Placement::Left,
        }
    }
}

/// A field's caption (`caption`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Caption {
    pub presence: Presence,
    pub placement: Placement,
    /// `reserve`: the space kept for the caption along its placement
    /// axis. `None` measures the caption's text.
    pub reserve: Option<Pt>,
    pub value: Value,
    pub font: Font,
    pub para: Para,
    pub margin: Margin,
}

/// A choice (`items`): what is shown and what is saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub display: String,
    pub save: String,
}

/// A check button's shape and mark (`checkButton`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CheckShape {
    #[default]
    Square,
    Round,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CheckMark {
    #[default]
    Default,
    Check,
    Circle,
    Cross,
    Diamond,
    Square,
    Star,
}

/// How a choice list opens (`choiceList open`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ChoiceOpen {
    /// A list box.
    Always,
    /// A combo box.
    #[default]
    UserControl,
    OnEntry,
    /// A multi-select list box.
    MultiSelect,
}

/// A field's user interface (`ui`).
#[derive(Clone, Debug, PartialEq)]
pub enum Ui {
    TextEdit {
        multi_line: bool,
        /// `comb numberOfCells`.
        comb: Option<u32>,
    },
    NumericEdit,
    DateTimeEdit,
    PasswordEdit,
    CheckButton {
        shape: CheckShape,
        mark: CheckMark,
        /// The box's side, in points (`size`; default 10pt).
        size: Pt,
        /// Draws the value as its own border (`border` inside `checkButton`).
        border: Border,
    },
    ChoiceList {
        open: ChoiceOpen,
        /// `textEntry`: the user may type a value that is not an item.
        text_entry: bool,
    },
    Button {
        /// `highlight`.
        highlight: String,
    },
    ImageEdit,
    Signature,
    Barcode,
    /// A `ui` with no known child, or none: drawn as a text edit.
    Unknown,
}

impl Default for Ui {
    fn default() -> Self {
        Ui::TextEdit {
            multi_line: false,
            comb: None,
        }
    }
}

/// The kind of value a field holds (its `value` child's element).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ValueKind {
    #[default]
    Text,
    Integer,
    Decimal,
    Float,
    Date,
    Time,
    DateTime,
    Boolean,
    Image,
    Html,
}

/// `bind match`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BindMatch {
    /// By name under the parent's data node.
    #[default]
    Once,
    /// By name anywhere in the data (`global`).
    Global,
    /// Not bound.
    None,
    /// `ref`: an explicit SOM expression (kept as written in `Bind::reference`).
    DataRef,
}

/// `bind`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Bind {
    pub matching: BindMatch,
    pub reference: String,
}

/// The path of a data node under `xfa:data`: element names with their
/// occurrence index among same-named siblings, root first (`form1[0]`,
/// `StatusCertificate[0]`, ...).
#[derive(Clone, Debug, PartialEq, Eq, Default, Hash)]
pub struct DataPath(pub Vec<(String, usize)>);

impl fmt::Display for DataPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, (name, index)) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            write!(f, "{name}[{index}]")?;
        }
        Ok(())
    }
}

impl DataPath {
    /// Reads what `Display` writes. `None` for anything else.
    pub fn parse(text: &str) -> Option<DataPath> {
        if text.is_empty() {
            return Some(DataPath::default());
        }
        let mut parts = Vec::new();
        for part in text.split('/') {
            let open = part.rfind('[')?;
            if !part.ends_with(']') {
                return None;
            }
            let index: usize = part[open + 1..part.len() - 1].parse().ok()?;
            let name = &part[..open];
            if name.is_empty() {
                return None;
            }
            parts.push((name.to_string(), index));
        }
        Some(DataPath(parts))
    }
}

/// A picture clause (`format/picture`, `ui/.../picture`) kept as written;
/// used to format dates and numbers for display.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Picture(pub String);

/// A fillable or display field (`field`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Field {
    pub common: Common,
    pub ui: Ui,
    pub caption: Option<Caption>,
    pub value_kind: ValueKind,
    /// The field's value: the data node's content when bound and present,
    /// else the template's `value`.
    pub value: Value,
    pub font: Font,
    pub para: Para,
    /// `items`: for check buttons the on, off and (optional) neutral
    /// values; for choice lists the choices.
    pub items: Vec<Item>,
    pub bind: Bind,
    pub picture: Option<Picture>,
    /// `maxChars` of a text value; 0 for unbounded.
    pub max_chars: u32,
    /// The fully qualified SOM name with indexes: `form1[0].Sub[0].F[0]`.
    /// The materialized AcroForm field's name.
    pub som: String,
    /// The data node this field reads and writes, when bound.
    pub data: Option<DataPath>,
    /// Set by `bind` when a check button's value equals its on item.
    pub checked: bool,
    /// `exclGroup` membership: the group's SOM name, for radio buttons.
    pub group: Option<String>,
    /// The widget's own border (`ui/textEdit/border`, likewise for
    /// `numericEdit`, `dateTimeEdit`, `passwordEdit`, `choiceList`): drawn
    /// around the value area, inside the caption. `Border::none()` when
    /// absent. A check button's is in `Ui::CheckButton::border`.
    pub ui_border: Border,
    /// The widget's own margin (`ui/textEdit/margin` and the like): insets
    /// of the value inside the widget border. `None` when absent (XFA then
    /// insets by the border's thickness, single-line by a little more).
    pub ui_margin: Option<Margin>,
}

impl Field {
    /// A check button's on and off save values (`items`), defaulting to
    /// `1` and `0`.
    pub fn check_values(&self) -> (String, String) {
        let on = self
            .items
            .first()
            .map(|i| i.save.clone())
            .unwrap_or_else(|| "1".to_string());
        let off = self
            .items
            .get(1)
            .map(|i| i.save.clone())
            .unwrap_or_else(|| "0".to_string());
        (on, off)
    }
    pub fn is_check(&self) -> bool {
        matches!(self.ui, Ui::CheckButton { .. })
    }
    /// Fillable in the materialized form: an interactive ui, not read-only,
    /// not a button, image or signature.
    pub fn fillable(&self) -> bool {
        !self.common.read_only
            && self.common.presence != Presence::Inactive
            && matches!(
                self.ui,
                Ui::TextEdit { .. }
                    | Ui::NumericEdit
                    | Ui::DateTimeEdit
                    | Ui::PasswordEdit
                    | Ui::CheckButton { .. }
                    | Ui::ChoiceList { .. }
                    | Ui::Unknown
            )
    }
}

/// What a draw shows (`draw ui`).
#[derive(Clone, Debug, PartialEq, Default)]
pub enum DrawUi {
    /// `textEdit` or no ui: its value as text.
    #[default]
    Text,
    /// `line`: a line across the draw's box from `slope` (`\` or `/`),
    /// stroked with `edge`.
    Line { slope_down: bool, edge: Edge },
    /// `rectangle`: its box with these edges and fill.
    Rectangle { border: Border },
    /// `arc`: an ellipse (or circle when `circular`) in its box.
    Arc {
        circular: bool,
        edge: Edge,
        fill: Option<Fill>,
    },
    /// `imageEdit`: not drawn.
    Image,
}

/// Static content (`draw`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Draw {
    pub common: Common,
    pub ui: DrawUi,
    pub value: Value,
    pub font: Font,
    pub para: Para,
    /// `draw`s may carry a caption too (rare); laid out like a field's.
    pub caption: Option<Caption>,
}

/// A radio button group (`exclGroup`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ExclGroup {
    pub common: Common,
    pub layout: Layout,
    pub fields: Vec<Field>,
    pub som: String,
    pub bind: Bind,
    pub data: Option<DataPath>,
    /// The group's value: the on item of the checked button, or empty.
    pub value: String,
}

/// A container (`subform`, and `area` read as a positioned subform).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Subform {
    pub common: Common,
    pub layout: Layout,
    /// `columnWidths` for `Layout::Table`, in points; a `-1` entry (an
    /// auto column) is left as a negative number for the layout to size.
    pub column_widths: Vec<Pt>,
    pub children: Vec<Node>,
    pub som: String,
    pub bind: Bind,
    pub data: Option<DataPath>,
    /// `occur`: min, max (`None` for unbounded), initial.
    pub occur: Occur,
    /// Which instance of its template this is (0-based), set by `bind`.
    pub instance: usize,
    /// `overflow` / `break` targets are not read; `Keep` is in `common`.
    pub is_area: bool,
}

/// `occur`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occur {
    pub min: usize,
    pub max: Option<usize>,
    pub initial: usize,
}

impl Default for Occur {
    fn default() -> Self {
        Occur {
            min: 1,
            max: Some(1),
            initial: 1,
        }
    }
}

/// A node of the form tree.
// The variants are read-only clones held in a tree; boxing buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Subform(Subform),
    Field(Field),
    Draw(Draw),
    ExclGroup(ExclGroup),
}

impl Node {
    pub fn common(&self) -> &Common {
        match self {
            Node::Subform(s) => &s.common,
            Node::Field(f) => &f.common,
            Node::Draw(d) => &d.common,
            Node::ExclGroup(g) => &g.common,
        }
    }
    pub fn common_mut(&mut self) -> &mut Common {
        match self {
            Node::Subform(s) => &mut s.common,
            Node::Field(f) => &mut f.common,
            Node::Draw(d) => &mut d.common,
            Node::ExclGroup(g) => &mut g.common,
        }
    }
}

/// A page master (`pageArea`): its paper and the content areas flowed
/// content fills, plus its own static draws and fields (headers, footers,
/// page numbers).
#[derive(Clone, Debug, PartialEq)]
pub struct PageArea {
    pub name: String,
    /// Paper size in points, after `orientation`.
    pub width: Pt,
    pub height: Pt,
    /// `contentArea`s in document order, as rectangles on the page.
    pub content_areas: Vec<Rect>,
    /// Static content placed on every page using this master (positioned
    /// at its own x/y on the page).
    pub statics: Vec<Node>,
    pub occur: Occur,
}

impl Default for PageArea {
    fn default() -> Self {
        PageArea {
            name: String::new(),
            width: 612.0,
            height: 792.0,
            content_areas: vec![Rect::new(0.0, 0.0, 612.0, 792.0)],
            statics: Vec::new(),
            occur: Occur::default(),
        }
    }
}

/// `pageSet`: the page masters in order. The layout uses the first for
/// every page unless an `occur` runs out (then the next).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct PageSet {
    pub areas: Vec<PageArea>,
}

/// The bound form: the root subform with every instance expanded and
/// every field's value and data path set.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Form {
    pub page_set: PageSet,
    pub root: Subform,
    /// `template` default locale (`locale`), for pictures; `en_US` when
    /// unset.
    pub locale: String,
    /// The plain value of every field that has an `id`, after binding
    /// (floating fields, which rich text embeds by id).
    pub ids: std::collections::BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_convert_to_points() {
        assert_eq!(length("72pt"), Some(72.0));
        assert_eq!(length("1in"), Some(72.0));
        assert!((length("25.4mm").unwrap() - 72.0).abs() < 1e-9);
        assert!((length("2.54cm").unwrap() - 72.0).abs() < 1e-9);
        assert_eq!(length("12"), Some(12.0));
        assert_eq!(length("-3pt"), Some(-3.0));
        assert_eq!(length("abc"), None);
        assert_eq!(length("1em"), None);
        assert_eq!(length(""), None);
    }

    #[test]
    fn colours_and_paths_round_trip() {
        assert_eq!(Rgb::parse("255, 0,7"), Some(Rgb(255, 0, 7)));
        assert_eq!(Rgb::parse("1,2"), None);
        let path = DataPath(vec![("form1".into(), 0), ("Row".into(), 2)]);
        assert_eq!(path.to_string(), "form1[0]/Row[2]");
        assert_eq!(DataPath::parse("form1[0]/Row[2]"), Some(path));
        assert_eq!(DataPath::parse("form1"), None);
        assert_eq!(DataPath::parse(""), Some(DataPath::default()));
    }

    #[test]
    fn rect_inset_never_goes_negative() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0).inset(&Margin::uniform(8.0));
        assert_eq!((r.w, r.h), (0.0, 0.0));
        assert_eq!((r.x, r.y), (8.0, 8.0));
    }
}
