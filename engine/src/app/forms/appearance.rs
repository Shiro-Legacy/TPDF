// SPDX-License-Identifier: AGPL-3.0-or-later
//! Widget appearance synthesis (spec 07 section 3): the appearance streams
//! of text, choice and button fields from their values, /DA, /Q, /MK and
//! /BS (ISO 32000-2 §12.5.6.19, §12.7.4.3, §12.7.5). Variable text is laid
//! out by `scripts` (shared with FreeText); the forms are written by
//! `write`.
//!
//! Every form paints its background, then its border (inside the box),
//! then its content clipped to the box less the border. The look is this
//! module's own: a 2 pt text margin, a line pitch of 1.2 em, multiline
//! auto size from 12 pt down to 4 pt, check, cross, circle, diamond,
//! square and star marks drawn as paths, a teal list selection and a
//! darker pressed button.
use std::cell::Cell;
use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::content::{ContentLimits, ContentOperand, ContentParser};
use crate::document::Document;
use crate::lexer::LexLimits;
use crate::object::{Dictionary, Object};
use crate::serialize::compact_real;

use super::scripts::{self, Cjk, Eight, Fonts, Glyph, Slot};
use super::text::{self, Family, Metrics, font_dict, win_ansi_code};
use super::{
    COMB, MULTILINE, PASSWORD, WidgetType, field_flags, inherited, max_len, number, on_state,
    pdf_rect, text_prefix, to_int, value_prefix, widget_type,
};

/// Characters of a value drawn at most; a longer value is cut there.
pub const MAX_DRAWN_CHARS: usize = 65_536;
/// A /DA longer than this reads as empty.
const MAX_DA_BYTES: usize = 16 * 1024;
/// /DA font sizes are clamped to this.
const MAX_FONT_SIZE: f64 = 2_000.0;
/// The smallest explicit /DA size drawn.
const MIN_FONT_SIZE: f64 = 0.01;
/// The smallest single-line auto size.
const MIN_AUTO_SIZE: f64 = 1.0;
/// Multiline auto size: the cap and the minimum (owner decision W3), and
/// the step it shrinks by.
const MULTILINE_CAP: f64 = 12.0;
const MULTILINE_MIN: f64 = 4.0;
const MULTILINE_STEP: f64 = 0.25;
/// A list box's size when /DA asks for auto size.
const LIST_SIZE: f64 = 12.0;
/// Baseline to baseline, in em.
const LINE_PITCH: f64 = 1.2;
/// Horizontal and vertical text margins inside the border, points.
const MARGIN_X: f64 = 2.0;
const MARGIN_Y: f64 = 1.0;
/// Lines laid out at most.
const MAX_LINES: usize = 65_536;
/// Comb dividers drawn at most.
const MAX_DIVIDERS: usize = 4_096;
/// /Opt entries a combo box looks through, and /V array entries read.
const MAX_OPTIONS: usize = 65_536;
/// Characters one widget's option reads, and its /V selection reads, may
/// take each (`Budget`): past it a combo box shows /V itself and a list box
/// shows the options read so far.
const MAX_OPTION_CHARS: usize = 1 << 22;
/// Coordinates written into content are clamped to this magnitude.
const MAX_COORDINATE: f64 = 1e12;
/// The list box selection colour: a teal band, dark enough to read as
/// selected at a glance, light enough for black text on it.
const SELECTION: Color = Color::Rgb(0.5, 0.78, 0.8);

/// A font an appearance's /Resources names.
#[derive(Clone, Debug, PartialEq)]
pub enum FontSource {
    /// The /AcroForm /DR font, as /DR holds it (a reference or a dictionary).
    Existing(Object),
    /// A new simple standard font with this /BaseFont (WinAnsiEncoding
    /// unless Symbol or ZapfDingbats): `write::base14_font` makes it.
    New(&'static [u8]),
    /// An 8-bit fallback font of a standard family (`Eight::font`).
    Eight { family: Family, eight: Eight },
    /// A CJK CID font (`Cjk::font` etc.).
    Cjk(Cjk),
}

/// One appearance stream: a Form XObject of `width` × `height` (the
/// widget's size, swapped for /MK /R 90 or 270) turned by `rotate`.
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub width: f64,
    pub height: f64,
    /// 0, 90, 180 or 270.
    pub rotate: i64,
    pub content: Vec<u8>,
    /// Resource name and font, in the order the content first uses them;
    /// the DA font (when text is drawn) first.
    pub fonts: Vec<(Vec<u8>, FontSource)>,
    /// A character no font encodes was drawn as `?`.
    pub lossy: bool,
}

impl Form {
    /// /Type /XObject /Subtype /Form /BBox [0 0 width height] /Matrix
    /// (the rotation of ISO 32000-2 §12.5.5, no translation), and when
    /// `fonts` is not empty /Resources << /Font << name → font_refs[i] >> >>.
    pub fn dictionary(&self, font_refs: &[Object]) -> Dictionary {
        let mut d = Dictionary::new();
        d.insert(b"Type".to_vec(), Object::name("XObject"));
        d.insert(b"Subtype".to_vec(), Object::name("Form"));
        d.insert(
            b"BBox".to_vec(),
            Object::Array(vec![
                Object::Integer(0),
                Object::Integer(0),
                number(self.width),
                number(self.height),
            ]),
        );
        let matrix: [i64; 6] = match self.rotate {
            90 => [0, 1, -1, 0, 0, 0],
            180 => [-1, 0, 0, -1, 0, 0],
            270 => [0, -1, 1, 0, 0, 0],
            _ => [1, 0, 0, 1, 0, 0],
        };
        d.insert(
            b"Matrix".to_vec(),
            Object::Array(matrix.iter().map(|&v| Object::Integer(v)).collect()),
        );
        if !self.fonts.is_empty() {
            let mut fonts = Dictionary::new();
            for ((name, _), font) in self.fonts.iter().zip(font_refs) {
                fonts.insert(name.clone(), font.clone());
            }
            let mut resources = Dictionary::new();
            resources.insert(b"Font".to_vec(), Object::Dictionary(fonts));
            d.insert(b"Resources".to_vec(), Object::Dictionary(resources));
        }
        d
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Synth {
    /// A text or choice field: /AP /N.
    Single(Form),
    /// A check box or radio button: /AP /N << /Off off /<on_name> on >>.
    States {
        off: Form,
        on_name: Vec<u8>,
        on: Form,
    },
    /// A push button: /AP /N and /D.
    Push { normal: Form, down: Form },
}

impl Synth {
    /// The forms in the order above (Single: [n]; States: [off, on];
    /// Push: [normal, down]).
    pub fn forms(&self) -> Vec<&Form> {
        match self {
            Self::Single(form) => vec![form],
            Self::States { off, on, .. } => vec![off, on],
            Self::Push { normal, down } => vec![normal, down],
        }
    }

    /// Some form is lossy.
    pub fn lossy(&self) -> bool {
        self.forms().iter().any(|form| form.lossy)
    }
}

/// The document's form defaults (/AcroForm /DA, /Q, /DR /Font) and the
/// work done with them so far.
pub struct Context<'d, 'a> {
    doc: &'d Document<'a>,
    acro: Option<&'d Dictionary>,
    catalog: Option<&'d Dictionary>,
    work: Cell<usize>,
}

impl<'d, 'a> Context<'d, 'a> {
    pub fn of(doc: &'d Document<'a>) -> Self {
        let catalog = doc.resolve_id(doc.root()).as_dict();
        let acro = catalog.and_then(|c| doc.dict_get(c, b"AcroForm").as_dict());
        Self {
            doc,
            acro,
            catalog,
            work: Cell::new(0),
        }
    }

    /// Bytes and characters every synthesis through this context has read
    /// (/DA strings, values, options), saturating.
    pub fn work(&self) -> usize {
        self.work.get()
    }

    /// /AcroForm /NeedAppearances is the boolean true.
    pub fn need_appearances(&self) -> bool {
        self.acro
            .is_some_and(|acro| self.doc.dict_get(acro, b"NeedAppearances").as_bool() == Some(true))
    }

    /// `synthesize_with(widget, None)`.
    pub fn synthesize(&self, widget: &Dictionary) -> Option<Synth> {
        self.synthesize_with(widget, None)
    }

    /// The widget's appearance. None when /Subtype is not /Widget, when no
    /// inherited /FT is a name, or for a signature field. `on` names a
    /// button's on state when its /AP has none (3.4.1).
    pub fn synthesize_with(&self, widget: &Dictionary, on: Option<&[u8]>) -> Option<Synth> {
        let doc = self.doc;
        if doc.dict_get(widget, b"Subtype").as_name() != Some(b"Widget") {
            return None;
        }
        inherited(doc, widget, b"FT").as_name()?;
        let kind = widget_type(doc, widget);
        let frame = Frame::of(doc, widget);
        match kind {
            WidgetType::Signature => None,
            WidgetType::Text => Some(Synth::Single(self.text_field(widget, &frame))),
            WidgetType::Combo => Some(Synth::Single(self.combo(widget, &frame))),
            WidgetType::List => Some(Synth::Single(self.list(widget, &frame))),
            WidgetType::Checkbox | WidgetType::Radio => {
                let radio = kind == WidgetType::Radio;
                let on_name = self.on_name(widget, on);
                let off = self.button_state(widget, &frame, radio, false);
                let on = self.button_state(widget, &frame, radio, true);
                Some(Synth::States { off, on_name, on })
            }
            WidgetType::PushButton => Some(Synth::Push {
                normal: self.push(widget, &frame, false),
                down: self.push(widget, &frame, true),
            }),
        }
    }

    fn charge(&self, amount: usize) {
        self.work.set(self.work.get().saturating_add(amount));
    }

    fn dr_fonts(&self) -> Option<&'d Dictionary> {
        let acro = self.acro?;
        let dr = self.doc.dict_get(acro, b"DR").as_dict()?;
        self.doc.dict_get(dr, b"Font").as_dict()
    }

    /// The widget's /DA, parsed (3.1.1).
    fn da(&self, widget: &Dictionary) -> Da {
        let own = inherited(self.doc, widget, b"DA").as_string();
        let bytes = own
            .or_else(|| {
                self.acro
                    .and_then(|acro| self.doc.dict_get(acro, b"DA").as_string())
            })
            .unwrap_or(b"");
        self.charge(bytes.len());
        Da::parse(bytes)
    }

    /// The inherited /Q, else /AcroForm /Q: 0, 1 or 2.
    fn quadding(&self, widget: &Dictionary) -> i64 {
        let own = inherited(self.doc, widget, b"Q");
        let q = if own.is_null() {
            self.acro
                .map_or(0, |acro| to_int(self.doc.dict_get(acro, b"Q")))
        } else {
            to_int(own)
        };
        if q == 1 || q == 2 { q } else { 0 }
    }

    /// The Han font of the widget's /Lang, else the catalog's.
    fn han(&self, widget: &Dictionary) -> Cjk {
        fn lang(value: &Object) -> Option<&[u8]> {
            match value {
                Object::Null => None,
                value => Some(value.as_string().unwrap_or(b"")),
            }
        }
        let own = lang(inherited(self.doc, widget, b"Lang"));
        let lang = own.or_else(|| {
            self.catalog
                .and_then(|c| lang(self.doc.dict_get(c, b"Lang")))
        });
        scripts::han_font(lang)
    }

    /// The DA font (3.1.4): the /DR font when it is a simple WinAnsi font
    /// of a non-symbolic family, else a new standard font of its family.
    fn da_font(&self, da: &Da) -> DaFont {
        let entry = self
            .dr_fonts()
            .and_then(|fonts| fonts.get(da.font.as_slice()));
        let mut family = Family::Helvetica;
        if let Some(entry) = entry
            && let Some(dict) = font_dict(self.doc, entry)
        {
            let base = self
                .doc
                .dict_get(dict, b"BaseFont")
                .as_name()
                .unwrap_or(b"");
            let of = Family::of(base);
            let subtype = self.doc.dict_get(dict, b"Subtype").as_name();
            let simple = matches!(subtype, Some(b"Type1" | b"TrueType" | b"MMType1"));
            let win = self.doc.dict_get(dict, b"Encoding").as_name() == Some(b"WinAnsiEncoding");
            if !of.symbolic() {
                if simple && win {
                    return DaFont {
                        name: da.font.clone(),
                        source: FontSource::Existing(entry.clone()),
                        metrics: Metrics::font(self.doc, dict),
                        family: of,
                    };
                }
                family = of;
            }
        }
        DaFont {
            name: da.font.clone(),
            source: FontSource::New(family.base_font()),
            metrics: Metrics::family(family),
            family,
        }
    }

    /// The ZapfDingbats font of a button mark (3.4.2).
    fn dingbats(&self) -> (Vec<u8>, FontSource) {
        if let Some(fonts) = self.dr_fonts() {
            for (name, entry) in fonts.iter() {
                let Some(dict) = font_dict(self.doc, entry) else {
                    continue;
                };
                let base = self.doc.dict_get(dict, b"BaseFont").as_name();
                if base == Some(b"ZapfDingbats") && dict.get(b"Encoding".as_slice()).is_none() {
                    return (name.clone(), FontSource::Existing(entry.clone()));
                }
            }
        }
        (b"ZaDb".to_vec(), FontSource::New(b"ZapfDingbats"))
    }

    /// The writer of one variable-text form.
    fn writer<'f>(&self, widget: &Dictionary, frame: &'f Frame, da: &Da) -> Writer<'f> {
        let font = self.da_font(da);
        Writer {
            frame,
            fallback: Eight::embedded_family(font.family),
            han: self.han(widget),
            color: da.color,
            out: String::new(),
            fonts: Vec::new(),
            lossy: false,
            font,
        }
    }

    // -------------------------------------------------------------- text

    fn text_field(&self, widget: &Dictionary, frame: &Frame) -> Form {
        let doc = self.doc;
        let flags = field_flags(doc, widget);
        let value = value_prefix(doc, widget, MAX_DRAWN_CHARS);
        self.charge(value.len());
        let da = self.da(widget);
        let q = self.quadding(widget);
        let mut w = self.writer(widget, frame, &da);
        let password = flags & PASSWORD != 0;
        let shown = if password {
            "*".repeat(value.chars().count())
        } else {
            value
        };
        let cells = max_len(doc, widget);
        w.frame_paint(false);
        if flags & COMB != 0 && flags & (MULTILINE | PASSWORD) == 0 && cells > 0 {
            w.comb_dividers(cells);
            w.open_tx();
            w.comb(&shown, cells, da.size);
        } else if flags & MULTILINE != 0 && !password {
            w.open_tx();
            w.multiline(&shown, da.size, q);
        } else {
            w.open_tx();
            w.single_line(&shown, da.size, q);
        }
        w.close_tx();
        w.finish()
    }

    // -------------------------------------------------------------- choice

    fn combo(&self, widget: &Dictionary, frame: &Frame) -> Form {
        let doc = self.doc;
        let value = value_prefix(doc, widget, MAX_DRAWN_CHARS);
        self.charge(value.len());
        let shown = self
            .display_of(widget, &value, &Budget::new())
            .unwrap_or(value);
        let da = self.da(widget);
        let q = self.quadding(widget);
        let mut w = self.writer(widget, frame, &da);
        w.frame_paint(false);
        w.open_tx();
        w.single_line(&shown, da.size, q);
        w.close_tx();
        w.finish()
    }

    /// The display text of the first /Opt entry whose export value is
    /// `value` (3.3.1); None, so /V itself is shown, when no entry is or
    /// `budget` runs out first.
    fn display_of(&self, widget: &Dictionary, value: &str, budget: &Budget) -> Option<String> {
        let doc = self.doc;
        let options = inherited(doc, widget, b"Opt").as_array()?;
        let wanted = value.chars().count().saturating_add(1);
        for option in options.iter().take(MAX_OPTIONS) {
            let option = doc.resolve(option);
            let (export, display) = match option {
                Object::Array(pair) if pair.len() == 2 => (doc.resolve(&pair[0]), Some(&pair[1])),
                other => (other, None),
            };
            // An export read cut short by the budget could match falsely.
            if budget.left() < wanted {
                return None;
            }
            let export = budget.read(export, wanted)?;
            self.charge(export.len().max(1));
            if export == value {
                let display = match display {
                    Some(d) => budget.read(doc.resolve(d), MAX_DRAWN_CHARS)?,
                    None => export,
                };
                self.charge(display.len());
                return Some(display);
            }
        }
        None
    }

    fn list(&self, widget: &Dictionary, frame: &Frame) -> Form {
        let doc = self.doc;
        let da = self.da(widget);
        let q = self.quadding(widget);
        let size = da.size.unwrap_or(LIST_SIZE);
        let selected = self.selected(widget);
        let top = usize::try_from(to_int(inherited(doc, widget, b"TI"))).unwrap_or(0);
        let mut w = self.writer(widget, frame, &da);
        let inner = frame.inner();
        let pitch = LINE_PITCH * size;
        let rows = if inner.height() > 0.0 {
            ((inner.height() / pitch).ceil() as usize).clamp(1, MAX_LINES)
        } else {
            0
        };
        // Characters one option may need to fill the width (no glyph is
        // narrower than 0.05 em), and all options together.
        let per_option =
            ((inner.width() / (size * 0.05)).ceil().max(1.0) as usize + 1).min(MAX_DRAWN_CHARS);
        let budget = Budget::new();
        let mut options = Vec::new();
        if let Some(items) = inherited(doc, widget, b"Opt").as_array() {
            for option in items.iter().skip(top).take(rows) {
                let read = match doc.resolve(option) {
                    Object::Array(pair) if pair.len() == 2 => budget
                        .read(doc.resolve(&pair[0]), MAX_DRAWN_CHARS)
                        .zip(budget.read(doc.resolve(&pair[1]), per_option)),
                    other => budget.read(other, MAX_DRAWN_CHARS).map(|text| {
                        let display = super::cut_chars(text.clone(), per_option);
                        (text, display)
                    }),
                };
                // The budget is spent: the rows read so far are shown.
                let Some((export, display)) = read else {
                    break;
                };
                self.charge(export.len() + display.len());
                options.push((selected.contains(&export), display));
            }
        }
        w.frame_paint(false);
        w.open_tx();
        w.list_rows(&options, size, q);
        w.close_tx();
        w.finish()
    }

    /// The export values /V selects: a text string, a name, or an array of
    /// them (3.3.2), read within their own budget.
    fn selected(&self, widget: &Dictionary) -> BTreeSet<String> {
        let doc = self.doc;
        let budget = Budget::new();
        let mut out = BTreeSet::new();
        let value = inherited(doc, widget, b"V");
        let items: &[Object] = match value {
            Object::Array(items) => items,
            other => std::slice::from_ref(other),
        };
        for item in items.iter().take(MAX_OPTIONS) {
            let text = match doc.resolve(item) {
                Object::Name(name) => budget.read_name(name, MAX_DRAWN_CHARS),
                value @ Object::String(_) => budget.read(value, MAX_DRAWN_CHARS),
                _ => continue,
            };
            let Some(text) = text else {
                break;
            };
            self.charge(text.len().max(1));
            out.insert(text);
        }
        out
    }

    // -------------------------------------------------------------- buttons

    /// The on-state name (3.4.1).
    fn on_name(&self, widget: &Dictionary, on: Option<&[u8]>) -> Vec<u8> {
        let doc = self.doc;
        let own = on_state(doc, widget);
        if !own.is_empty() {
            return own;
        }
        let ap = doc.dict_get(widget, b"AP").as_dict();
        let down = ap.and_then(|ap| doc.dict_get(ap, b"D").as_dict());
        if let Some(key) = down.and_then(|d| d.keys().find(|k| k.as_slice() != b"Off")) {
            return key.clone();
        }
        let usable = |name: &[u8]| !name.is_empty() && name != b"Off";
        if let Some(on) = on.filter(|n| usable(n)) {
            return on.to_vec();
        }
        if let Some(state) = doc.dict_get(widget, b"AS").as_name().filter(|n| usable(n)) {
            return state.to_vec();
        }
        if let Some(value) = inherited(doc, widget, b"V").as_name().filter(|n| usable(n)) {
            return value.to_vec();
        }
        b"Yes".to_vec()
    }

    /// The /MK entry `key` when it is exactly one character.
    fn caption_char(&self, widget: &Dictionary, key: &[u8]) -> Option<char> {
        let mk = self.doc.dict_get(widget, b"MK").as_dict()?;
        let text = text_prefix(self.doc.dict_get(mk, key), 2);
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        }
    }

    fn caption(&self, widget: &Dictionary, key: &[u8]) -> Option<String> {
        let mk = self.doc.dict_get(widget, b"MK").as_dict()?;
        let value = self.doc.dict_get(mk, key);
        value.as_string()?;
        let text = text_prefix(value, MAX_DRAWN_CHARS);
        self.charge(text.len());
        Some(text)
    }

    /// A check box's or radio button's Off or on form.
    fn button_state(&self, widget: &Dictionary, frame: &Frame, radio: bool, on: bool) -> Form {
        let da = self.da(widget);
        let mut w = self.writer(widget, frame, &da);
        w.frame_paint(radio);
        if on {
            let mark = match self.caption_char(widget, b"CA") {
                None if radio => Mark::Dot,
                None | Some('4') => Mark::Check,
                Some('l') => Mark::Dot,
                Some('8') => Mark::Cross,
                Some('u') => Mark::Diamond,
                Some('n') => Mark::Square,
                Some('H') => Mark::Star,
                Some(c) => Mark::Glyph(win_ansi_code(c).unwrap_or(b'4')),
            };
            let dingbats = matches!(mark, Mark::Glyph(_)).then(|| self.dingbats());
            w.mark(mark, radio, dingbats);
        }
        w.finish()
    }

    fn push(&self, widget: &Dictionary, frame: &Frame, down: bool) -> Form {
        let da = self.da(widget);
        // An empty /AC is no alternate caption (W4): /CA shows.
        let caption = if down {
            self.caption(widget, b"AC")
                .filter(|c| !c.is_empty())
                .or_else(|| self.caption(widget, b"CA"))
        } else {
            self.caption(widget, b"CA")
        };
        let mut pressed = frame.clone();
        if down {
            pressed.bg = Some(pressed.bg.map_or(Color::Gray(0.75), |bg| bg.darker(0.7)));
        }
        let mut w = self.writer(widget, &pressed, &da);
        w.frame_paint(false);
        if let Some(caption) = caption.filter(|c| !c.is_empty()) {
            w.single_line(&caption, da.size, 1);
        }
        w.finish()
    }
}

/// What one widget's option or selection reads may still take, in
/// characters (`MAX_OPTION_CHARS`): bounds a widget whose many /Opt or /V
/// entries all reference one long string.
struct Budget(Cell<usize>);

impl Budget {
    fn new() -> Self {
        Self(Cell::new(MAX_OPTION_CHARS))
    }

    fn left(&self) -> usize {
        self.0.get()
    }

    fn spend(&self, chars: usize) {
        self.0
            .set(self.0.get().saturating_sub(chars.saturating_add(1)));
    }

    /// `value` as text, cut at `max` characters and at what is left; None
    /// once the budget is spent.
    fn read(&self, value: &Object, max: usize) -> Option<String> {
        let left = self.left();
        if left == 0 {
            return None;
        }
        let text = text_prefix(value, max.min(left));
        self.spend(text.chars().count());
        Some(text)
    }

    /// A name's bytes as text, under the same bounds.
    fn read_name(&self, name: &[u8], max: usize) -> Option<String> {
        let left = self.left();
        if left == 0 {
            return None;
        }
        let text = super::cut_chars(
            super::name_text(&name[..name.len().min(max.min(left).saturating_mul(4))]),
            max.min(left),
        );
        self.spend(text.chars().count());
        Some(text)
    }
}

// ------------------------------------------------------------------ /DA

/// A parsed /DA: font resource name, size (None: auto), colour.
#[derive(Clone, Debug, PartialEq)]
struct Da {
    font: Vec<u8>,
    size: Option<f64>,
    color: Option<Color>,
}

impl Da {
    fn parse(bytes: &[u8]) -> Self {
        let mut da = Self {
            font: b"Helv".to_vec(),
            size: Some(12.0),
            color: None,
        };
        if bytes.len() > MAX_DA_BYTES {
            return da;
        }
        let limits = ContentLimits {
            max_input_bytes: MAX_DA_BYTES,
            max_depth: 4,
            max_operands: 16,
            max_operations: 1_024,
            max_operator_bytes: 16,
            max_nodes: 16_384,
            max_owned_bytes: 64 * 1024,
            lex: LexLimits {
                max_token_bytes: MAX_DA_BYTES,
                max_tokens: 32_768,
            },
        };
        let mut parser = ContentParser::new(bytes, limits);
        while let Ok(Some(op)) = parser.next_operation() {
            let operands = &op.operands;
            let n = operands.len();
            let numbers = || {
                operands
                    .iter()
                    .map(operand_number)
                    .collect::<Option<Vec<f64>>>()
            };
            match op.operator.as_slice() {
                b"Tf" => {
                    da.font = match n.checked_sub(2).map(|at| &operands[at]) {
                        Some(ContentOperand::Name(name)) => name.clone(),
                        _ => b"Helv".to_vec(),
                    };
                    da.size = match operands.last().and_then(operand_number) {
                        None => Some(12.0),
                        Some(v) if !v.is_finite() || v <= 0.0 => None,
                        Some(v) => Some(v.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE)),
                    };
                }
                b"g" if n == 1 => {
                    if let Some(v) = numbers() {
                        da.color = Some(Color::Gray(unit(v[0])));
                    }
                }
                b"rg" if n == 3 => {
                    if let Some(v) = numbers() {
                        da.color = Some(Color::Rgb(unit(v[0]), unit(v[1]), unit(v[2])));
                    }
                }
                b"k" if n == 4 => {
                    if let Some(v) = numbers() {
                        da.color =
                            Some(Color::Cmyk(unit(v[0]), unit(v[1]), unit(v[2]), unit(v[3])));
                    }
                }
                _ => {}
            }
        }
        da
    }
}

fn operand_number(operand: &ContentOperand) -> Option<f64> {
    match operand {
        ContentOperand::Integer(v) => Some(*v as f64),
        ContentOperand::Real(v) => Some(*v),
        _ => None,
    }
}

/// A colour component: non-finite reads as 0, the rest is clamped to 0..1.
fn unit(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

// ------------------------------------------------------------------ colours

#[derive(Clone, Copy, Debug, PartialEq)]
enum Color {
    Gray(f64),
    Rgb(f64, f64, f64),
    Cmyk(f64, f64, f64, f64),
}

impl Color {
    /// An /MK colour array: 1, 3 or 4 numbers; anything else is absent.
    fn of(doc: &Document<'_>, value: &Object) -> Option<Self> {
        let items = value.as_array()?;
        let v: Vec<f64> = items
            .iter()
            .map(|item| doc.resolve(item).as_f64().map(unit))
            .collect::<Option<_>>()?;
        match v.as_slice() {
            [g] => Some(Self::Gray(*g)),
            [r, g, b] => Some(Self::Rgb(*r, *g, *b)),
            [c, m, y, k] => Some(Self::Cmyk(*c, *m, *y, *k)),
            _ => None,
        }
    }

    fn op(self, stroke: bool) -> String {
        let (ops, values): (&str, Vec<f64>) = match self {
            Self::Gray(g) => ("g", vec![g]),
            Self::Rgb(r, g, b) => ("rg", vec![r, g, b]),
            Self::Cmyk(c, m, y, k) => ("k", vec![c, m, y, k]),
        };
        let mut out: String = values.iter().map(|&v| num(v) + " ").collect();
        if stroke {
            out.push_str(&ops.to_ascii_uppercase());
        } else {
            out.push_str(ops);
        }
        out
    }

    fn fill(self) -> String {
        self.op(false)
    }

    fn stroke(self) -> String {
        self.op(true)
    }

    /// The colour moved `share` of the way toward white.
    fn lighter(self, share: f64) -> Self {
        let up = |v: f64| v + (1.0 - v) * share;
        match self {
            Self::Gray(g) => Self::Gray(up(g)),
            Self::Rgb(r, g, b) => Self::Rgb(up(r), up(g), up(b)),
            Self::Cmyk(c, m, y, k) => {
                let down = |v: f64| v * (1.0 - share);
                Self::Cmyk(down(c), down(m), down(y), down(k))
            }
        }
    }

    /// The colour with its lightness scaled by `factor`.
    fn darker(self, factor: f64) -> Self {
        match self {
            Self::Gray(g) => Self::Gray(g * factor),
            Self::Rgb(r, g, b) => Self::Rgb(r * factor, g * factor, b * factor),
            Self::Cmyk(c, m, y, k) => Self::Cmyk(c, m, y, 1.0 - (1.0 - k) * factor),
        }
    }
}

// ------------------------------------------------------------------ geometry

/// A number as content writes it: finite, clamped, rounded to 1/1000.
fn num(v: f64) -> String {
    let v = if v.is_finite() {
        v.clamp(-MAX_COORDINATE, MAX_COORDINATE)
    } else {
        0.0
    };
    let r = (v * 1000.0).round() / 1000.0;
    if r == 0.0 {
        "0".into()
    } else {
        compact_real(r)
    }
}

/// A name token, delimiters, whitespace and non-printable bytes as `#xx`.
fn pdf_name(name: &[u8]) -> String {
    let mut out = String::from("/");
    for &b in name {
        if (0x21..=0x7E).contains(&b) && !b"()<>[]{}/%#".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "#{b:02X}");
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Area {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Area {
    fn width(&self) -> f64 {
        (self.x1 - self.x0).max(0.0)
    }
    fn height(&self) -> f64 {
        (self.y1 - self.y0).max(0.0)
    }
    /// The area shrunk by `dx` and `dy` on each side, never inverted.
    fn shrink(&self, dx: f64, dy: f64) -> Self {
        let dx = dx.min(self.width() / 2.0).max(0.0);
        let dy = dy.min(self.height() / 2.0).max(0.0);
        Self {
            x0: self.x0 + dx,
            y0: self.y0 + dy,
            x1: self.x1 - dx,
            y1: self.y1 - dy,
        }
    }
    fn re(&self) -> String {
        format!(
            "{} {} {} {} re",
            num(self.x0),
            num(self.y0),
            num(self.width()),
            num(self.height())
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Solid,
    Dashed,
    Underline,
    Beveled,
    Inset,
}

/// The common frame of a widget's forms (3.1.1).
#[derive(Clone, Debug, PartialEq)]
struct Frame {
    /// The form's size (the rect's, swapped for 90 and 270), and the
    /// size content is laid out in (bounded).
    width: f64,
    height: f64,
    w: f64,
    h: f64,
    rotate: i64,
    border: f64,
    style: Style,
    dash: Vec<f64>,
    bg: Option<Color>,
    bc: Option<Color>,
}

/// Content geometry beyond this is not laid out.
const MAX_EXTENT: f64 = 1e9;

impl Frame {
    fn of(doc: &Document<'_>, widget: &Dictionary) -> Self {
        let rect = pdf_rect(doc, widget);
        let finite = |v: f64| if v.is_finite() { v.max(0.0) } else { f64::MAX };
        let (rw, rh) = (finite(rect[2] - rect[0]), finite(rect[3] - rect[1]));
        let mk = doc.dict_get(widget, b"MK").as_dict();
        let rotate = mk
            .map(|mk| to_int(doc.dict_get(mk, b"R")).rem_euclid(360))
            .filter(|r| matches!(r, 90 | 180 | 270))
            .unwrap_or(0);
        let (width, height) = if rotate == 90 || rotate == 270 {
            (rh, rw)
        } else {
            (rw, rh)
        };
        let bs = doc.dict_get(widget, b"BS").as_dict();
        let from_bs = bs
            .map(|bs| doc.dict_get(bs, b"W"))
            .filter(|w| !w.is_null())
            .map(|w| w.as_f64());
        let border = match from_bs {
            Some(w) => w,
            None => doc
                .dict_get(widget, b"Border")
                .as_array()
                .and_then(|b| b.get(2))
                .map_or(Some(1.0), |w| doc.resolve(w).as_f64()),
        };
        let border = border.filter(|w| w.is_finite() && *w >= 0.0).unwrap_or(1.0);
        let style = match bs.and_then(|bs| doc.dict_get(bs, b"S").as_name()) {
            Some(b"D") => Style::Dashed,
            Some(b"U") => Style::Underline,
            Some(b"B") => Style::Beveled,
            Some(b"I") => Style::Inset,
            _ => Style::Solid,
        };
        let dash = bs
            .and_then(|bs| doc.dict_get(bs, b"D").as_array())
            .and_then(|items| {
                if items.is_empty() || items.len() > 16 {
                    return None;
                }
                let v: Vec<f64> = items
                    .iter()
                    .map(|i| {
                        doc.resolve(i)
                            .as_f64()
                            .filter(|v| v.is_finite() && *v >= 0.0)
                    })
                    .collect::<Option<_>>()?;
                v.iter().any(|v| *v > 0.0).then_some(v)
            })
            .unwrap_or_else(|| vec![3.0]);
        let color = |key: &[u8]| mk.and_then(|mk| Color::of(doc, doc.dict_get(mk, key)));
        Self {
            width,
            height,
            w: width.min(MAX_EXTENT),
            h: height.min(MAX_EXTENT),
            rotate,
            border,
            style,
            dash,
            bg: color(b"BG"),
            bc: color(b"BC"),
        }
    }

    fn whole(&self) -> Area {
        Area {
            x0: 0.0,
            y0: 0.0,
            x1: self.w,
            y1: self.h,
        }
    }

    /// The border width as drawn: at most half the box.
    fn drawn_border(&self) -> f64 {
        self.border.min(self.w / 2.0).min(self.h / 2.0).max(0.0)
    }

    /// The box less the border (and the 3-D edge of B and I).
    fn inner(&self) -> Area {
        let edge = match self.style {
            Style::Beveled | Style::Inset => 2.0 * self.border,
            _ => self.border,
        };
        self.whole().shrink(edge, edge)
    }

    /// Where text is placed: the inner box less the margins.
    fn padded(&self) -> Area {
        let inner = self.inner();
        inner.shrink(
            MARGIN_X.min(inner.width() / 4.0),
            MARGIN_Y.min(inner.height() / 4.0),
        )
    }
}

// ------------------------------------------------------------------ writing

struct DaFont {
    name: Vec<u8>,
    source: FontSource,
    metrics: Metrics,
    family: Family,
}

/// The marks of check boxes and radio buttons: the six Acrobat styles
/// (/MK /CA 4, l, 8, u, n, H) as paths, anything else as its
/// ZapfDingbats glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    Check,
    Dot,
    Cross,
    Diamond,
    Square,
    Star,
    Glyph(u8),
}

/// One form being written.
struct Writer<'f> {
    frame: &'f Frame,
    font: DaFont,
    fallback: Family,
    han: Cjk,
    color: Option<Color>,
    out: String,
    fonts: Vec<(Vec<u8>, FontSource)>,
    lossy: bool,
}

/// The state of a text object being written: the font set, and the
/// origin of the last `Td`.
struct TextState {
    font: Option<(Vec<u8>, f64)>,
    origin: (f64, f64),
}

impl Writer<'_> {
    fn finish(self) -> Form {
        Form {
            width: self.frame.width,
            height: self.frame.height,
            rotate: self.frame.rotate,
            content: self.out.into_bytes(),
            fonts: self.fonts,
            lossy: self.lossy,
        }
    }

    fn line(&mut self, text: &str) {
        self.out.push_str(text);
        self.out.push('\n');
    }

    fn fonts_for(&self) -> Fonts<'_> {
        Fonts {
            latin: &self.font.metrics,
            family: self.fallback,
            han: self.han,
            latin_extended: true,
        }
    }

    /// The resource name of `slot`'s font, listed on first use (the DA
    /// font first).
    fn resource(&mut self, slot: Slot) -> Vec<u8> {
        if self.fonts.is_empty() {
            self.fonts
                .push((self.font.name.clone(), self.font.source.clone()));
        }
        let (name, source) = match slot {
            Slot::Latin => return self.font.name.clone(),
            Slot::Eight(eight) => (
                [&self.font.name[..], b"_", eight.suffix().as_bytes()].concat(),
                FontSource::Eight {
                    family: self.fallback,
                    eight,
                },
            ),
            Slot::Cjk(cjk) => (
                [&self.font.name[..], b"_", cjk.name().as_bytes()].concat(),
                FontSource::Cjk(cjk),
            ),
        };
        if !self.fonts.iter().any(|(n, _)| *n == name) {
            self.fonts.push((name.clone(), source));
        }
        name
    }

    /// The glyph box (top, bottom) in em of the fonts `glyphs` use.
    fn glyph_box<'g>(&self, glyphs: impl IntoIterator<Item = &'g Glyph>) -> (f64, f64) {
        let (mut top, mut bottom) = self.font.family.glyph_box();
        let (mut eight, mut cjk) = (false, false);
        for g in glyphs {
            match g.slot {
                Slot::Latin => {}
                Slot::Eight(_) => eight = true,
                Slot::Cjk(_) => cjk = true,
            }
            if eight && cjk {
                break;
            }
        }
        if eight {
            let (t, b) = self.fallback.glyph_box();
            (top, bottom) = (top.max(t), bottom.min(b));
        }
        if cjk {
            let (t, b) = Cjk::glyph_box();
            (top, bottom) = (top.max(t), bottom.min(b));
        }
        (top, bottom)
    }

    // -------------------------------------------------------------- frame

    /// Background and border (3.1.2), round for a radio button.
    fn frame_paint(&mut self, round: bool) {
        let f = self.frame;
        let (w, h) = (f.w, f.h);
        let (cx, cy, r) = (w / 2.0, h / 2.0, w.min(h) / 2.0);
        if let Some(bg) = f.bg {
            if round {
                let path = circle(cx, cy, r);
                self.line(&format!("{} {path} f", bg.fill()));
            } else {
                let re = f.whole().re();
                self.line(&format!("{} {re} f", bg.fill()));
            }
        }
        let bw = f.drawn_border();
        let Some(bc) = f.bc.filter(|_| bw > 0.0) else {
            return;
        };
        let dash = if f.style == Style::Dashed {
            let items: Vec<String> = f.dash.iter().map(|&v| num(v)).collect();
            format!("[{}] 0 d ", items.join(" "))
        } else {
            String::new()
        };
        if round {
            let path = circle(cx, cy, (r - bw / 2.0).max(0.0));
            self.line(&format!("q {} {} w {dash}{path} S Q", bc.stroke(), num(bw)));
            return;
        }
        if f.style == Style::Underline {
            self.line(&format!(
                "q {} {} w 0 {} m {} {} l S Q",
                bc.stroke(),
                num(bw),
                num(bw / 2.0),
                num(w),
                num(bw / 2.0)
            ));
            return;
        }
        let edge = f.whole().shrink(bw / 2.0, bw / 2.0).re();
        self.line(&format!("q {} {} w {dash}{edge} S Q", bc.stroke(), num(bw)));
        if matches!(f.style, Style::Beveled | Style::Inset) {
            // Beveled: lit from the top left, the edge a lighter and a
            // darker shade of the background (white when there is none).
            // Inset: the reverse, in fixed greys.
            let (light, dark) = if f.style == Style::Beveled {
                let bg = f.bg.unwrap_or(Color::Gray(1.0));
                (bg.lighter(0.65), bg.darker(0.55))
            } else {
                (Color::Gray(0.42), Color::Gray(0.82))
            };
            let a = f.whole().shrink(bw, bw);
            let t = bw.min(a.width() / 2.0).min(a.height() / 2.0);
            let pts = |p: &[(f64, f64)]| {
                let mut s = String::new();
                for (i, (x, y)) in p.iter().enumerate() {
                    let op = if i == 0 { "m" } else { "l" };
                    let _ = write!(s, "{} {} {op} ", num(*x), num(*y));
                }
                s + "h f"
            };
            let upper = pts(&[
                (a.x0, a.y0),
                (a.x0, a.y1),
                (a.x1, a.y1),
                (a.x1 - t, a.y1 - t),
                (a.x0 + t, a.y1 - t),
                (a.x0 + t, a.y0 + t),
            ]);
            let lower = pts(&[
                (a.x1, a.y1),
                (a.x1, a.y0),
                (a.x0, a.y0),
                (a.x0 + t, a.y0 + t),
                (a.x1 - t, a.y0 + t),
                (a.x1 - t, a.y1 - t),
            ]);
            self.line(&format!("q {} {upper} Q", light.fill()));
            self.line(&format!("q {} {lower} Q", dark.fill()));
        }
    }

    fn open_tx(&mut self) {
        self.line("/Tx BMC");
    }

    fn close_tx(&mut self) {
        self.line("EMC");
    }

    /// `q`, the clip to the inner box, `BT` and the text colour.
    fn begin_text(&mut self) -> TextState {
        let clip = self.frame.inner().re();
        self.line(&format!("q {clip} W n"));
        self.line("BT");
        let color = self.color.unwrap_or(Color::Gray(0.0)).fill();
        self.line(&color);
        TextState {
            font: None,
            origin: (0.0, 0.0),
        }
    }

    fn end_text(&mut self) {
        self.line("ET Q");
    }

    /// Shows `glyphs` from (x, y) at `size`, one show per font run.
    fn show(&mut self, state: &mut TextState, glyphs: &[Glyph], x: f64, y: f64, size: f64) {
        let mut rx = x;
        let mut start = 0;
        while start < glyphs.len() {
            let slot = glyphs[start].slot;
            let end = glyphs[start..]
                .iter()
                .position(|g| g.slot != slot)
                .map_or(glyphs.len(), |at| start + at);
            let run = &glyphs[start..end];
            let name = self.resource(slot);
            if state.font.as_ref() != Some(&(name.clone(), size)) {
                self.line(&format!("{} {} Tf", pdf_name(&name), num(size)));
                state.font = Some((name, size));
            }
            let (dx, dy) = (rx - state.origin.0, y - state.origin.1);
            self.line(&format!(
                "{} {} Td {} Tj",
                num(dx),
                num(dy),
                scripts::string(run)
            ));
            state.origin = (rx, y);
            rx += text::advance(run) * size;
            start = end;
        }
    }

    /// The x of a line `width` wide at `size` in `area` by quadding `q`.
    fn align(area: &Area, width: f64, size: f64, q: i64) -> f64 {
        let free = area.width() - width * size;
        match q {
            1 => area.x0 + free / 2.0,
            2 => area.x1 - width * size,
            _ => area.x0,
        }
    }

    /// The baseline that centres a glyph box (top, bottom) at `size` in
    /// `area`, kept inside the box.
    fn baseline(&self, area: &Area, (top, bottom): (f64, f64), size: f64) -> f64 {
        let y = (area.y0 + area.y1) / 2.0 - (top + bottom) / 2.0 * size;
        let inner = self.frame.inner();
        if inner.height() > 0.0 {
            y.clamp(inner.y0, inner.y1)
        } else {
            y.clamp(0.0, self.frame.h)
        }
    }

    // -------------------------------------------------------------- layouts

    /// One line, no wrapping (3.2.2), auto size when `size` is None.
    fn single_line(&mut self, text: &str, size: Option<f64>, q: i64) {
        let (glyphs, lossy) = scripts::encode(text, &self.fonts_for());
        self.lossy |= lossy;
        if glyphs.is_empty() {
            return;
        }
        let area = self.frame.padded();
        let gbox = self.glyph_box(&glyphs);
        let width = text::advance(&glyphs);
        let size = size.unwrap_or_else(|| {
            let by_height = area.height() / (gbox.0 - gbox.1);
            let by_width = if width > 0.0 {
                area.width() / width
            } else {
                f64::INFINITY
            };
            floor2(by_height.min(by_width)).clamp(MIN_AUTO_SIZE, MAX_FONT_SIZE)
        });
        let y = self.baseline(&area, gbox, size);
        let x = Self::align(&area, width, size, q);
        let mut state = self.begin_text();
        self.show(&mut state, &glyphs, x, y, size);
        self.end_text();
    }

    /// Wrapped lines from the top (3.2.3).
    fn multiline(&mut self, text: &str, size: Option<f64>, q: i64) {
        let (hard, lossy) = scripts::shape(text, &self.fonts_for());
        self.lossy |= lossy;
        if hard.iter().all(Vec::is_empty) {
            return;
        }
        let area = self.frame.padded();
        let gbox = self.glyph_box(hard.iter().flatten());
        let tall = gbox.0 - gbox.1;
        // Lines that fit the padded height at `s`.
        let fitting = |s: f64| -> usize {
            let room = area.height() - tall * s;
            if room < -1e-9 {
                0
            } else {
                ((room.max(0.0) / (LINE_PITCH * s) + 1e-9).floor() as usize + 1).min(MAX_LINES)
            }
        };
        let size = match size {
            Some(size) => size,
            None => {
                let mut chosen = MULTILINE_MIN;
                let mut s = MULTILINE_CAP;
                while s >= MULTILINE_MIN {
                    let fit = fitting(s);
                    if fit > 0 && scripts::wrap(&hard, s, area.width(), fit + 1).len() <= fit {
                        chosen = s;
                        break;
                    }
                    s -= MULTILINE_STEP;
                }
                chosen
            }
        };
        let pitch = LINE_PITCH * size;
        // Lines whose top is inside the box.
        let inner = self.frame.inner();
        let visible = ((area.y1 - inner.y0) / pitch).ceil();
        let limit = if visible.is_finite() && visible > 0.0 {
            (visible as usize).clamp(1, MAX_LINES)
        } else {
            1
        };
        let lines = scripts::wrap(&hard, size, area.width(), limit);
        let first = area.y1 - gbox.0 * size;
        let mut state = self.begin_text();
        for (i, line) in lines.iter().enumerate() {
            let y = first - i as f64 * pitch;
            if y + gbox.0 * size < inner.y0 {
                break;
            }
            if line.glyphs.is_empty() {
                continue;
            }
            let x = Self::align(&area, line.width, size, q);
            self.show(&mut state, &line.glyphs, x, y, size);
        }
        self.end_text();
    }

    /// The vertical dividers between comb cells (3.2.4), part of the border.
    fn comb_dividers(&mut self, cells: i64) {
        let f = self.frame;
        let bw = f.drawn_border();
        let Some(bc) = f.bc.filter(|_| bw > 0.0) else {
            return;
        };
        let inner = f.inner();
        let n = cells as f64;
        let count = usize::try_from(cells - 1).unwrap_or(0).min(MAX_DIVIDERS);
        if count == 0 {
            return;
        }
        let mut path = String::new();
        for i in 1..=count {
            let x = inner.x0 + inner.width() * i as f64 / n;
            let _ = write!(
                path,
                "{} {} m {} {} l ",
                num(x),
                num(inner.y0),
                num(x),
                num(inner.y1)
            );
        }
        self.line(&format!("q {} {} w {path}S Q", bc.stroke(), num(bw)));
    }

    /// One character per cell (3.2.4).
    fn comb(&mut self, text: &str, cells: i64, size: Option<f64>) {
        let (glyphs, lossy) = scripts::encode(text, &self.fonts_for());
        self.lossy |= lossy;
        let count = usize::try_from(cells).unwrap_or(0);
        let glyphs: Vec<Glyph> = glyphs.into_iter().take(count).collect();
        if glyphs.is_empty() {
            return;
        }
        let inner = self.frame.inner();
        let area = self.frame.padded();
        let cell = inner.width() / cells as f64;
        let gbox = self.glyph_box(&glyphs);
        let size = size.unwrap_or_else(|| {
            let by_height = area.height() / (gbox.0 - gbox.1);
            let widest = glyphs.iter().map(|g| g.advance).fold(0.0, f64::max) / 1000.0;
            let by_width = if widest > 0.0 {
                cell / widest
            } else {
                f64::INFINITY
            };
            floor2(by_height.min(by_width)).clamp(MIN_AUTO_SIZE, MAX_FONT_SIZE)
        });
        let y = self.baseline(&area, gbox, size);
        let mut state = self.begin_text();
        for (i, g) in glyphs.iter().enumerate() {
            let centre = inner.x0 + (i as f64 + 0.5) * cell;
            let x = centre - g.advance / 1000.0 * size / 2.0;
            self.show(&mut state, std::slice::from_ref(g), x, y, size);
        }
        self.end_text();
    }

    /// A list box's rows (3.3.2): one option per row, selected rows on a
    /// band.
    fn list_rows(&mut self, options: &[(bool, String)], size: f64, q: i64) {
        let inner = self.frame.inner();
        let area = self.frame.padded();
        let pitch = LINE_PITCH * size;
        let mut rows = Vec::with_capacity(options.len());
        for (selected, display) in options {
            let (glyphs, lossy) = scripts::encode(display, &self.fonts_for());
            self.lossy |= lossy;
            rows.push((*selected, glyphs));
        }
        let gbox = self.glyph_box(rows.iter().flat_map(|(_, g)| g.iter()));
        let row_top = |i: usize| inner.y1 - i as f64 * pitch;
        let mut bands = String::new();
        for (i, (selected, _)) in rows.iter().enumerate() {
            if *selected {
                let band = Area {
                    x0: inner.x0,
                    x1: inner.x1,
                    y0: (row_top(i) - pitch).max(inner.y0),
                    y1: row_top(i),
                };
                if band.height() > 0.0 {
                    let _ = write!(bands, "{} ", band.re());
                }
            }
        }
        if !bands.is_empty() {
            let clip = inner.re();
            self.line(&format!("q {clip} W n {} {bands}f Q", SELECTION.fill()));
        }
        if rows.iter().all(|(_, g)| g.is_empty()) {
            return;
        }
        let mut state = self.begin_text();
        for (i, (_, glyphs)) in rows.iter().enumerate() {
            if glyphs.is_empty() {
                continue;
            }
            let y = row_top(i) - pitch / 2.0 - (gbox.0 + gbox.1) / 2.0 * size;
            let x = Self::align(&area, text::advance(glyphs), size, q);
            self.show(&mut state, glyphs, x, y, size);
        }
        self.end_text();
    }

    // -------------------------------------------------------------- marks

    /// A button's on mark (3.4.2, 3.4.3), in the DA colour.
    fn mark(&mut self, mark: Mark, radio: bool, dingbats: Option<(Vec<u8>, FontSource)>) {
        let f = self.frame;
        let color = self.color.unwrap_or(Color::Gray(0.0));
        let (cx, cy) = (f.w / 2.0, f.h / 2.0);
        // The square the mark is drawn in: the inner box, or the square
        // inscribed in a radio button's inner circle.
        let side = if radio {
            let r = (f.w.min(f.h) / 2.0 - f.drawn_border()).max(0.0);
            r * std::f64::consts::SQRT_2
        } else {
            let inner = f.inner();
            inner.width().min(inner.height())
        };
        if side <= 0.0 {
            return;
        }
        // Unit-square points to form space.
        let at = |u: f64, v: f64| (cx + (u - 0.5) * side, cy + (v - 0.5) * side);
        let poly = |points: &[(f64, f64)]| {
            let mut s = String::new();
            for (i, &(u, v)) in points.iter().enumerate() {
                let (x, y) = at(u, v);
                let op = if i == 0 { "m" } else { "l" };
                let _ = write!(s, "{} {} {op} ", num(x), num(y));
            }
            s
        };
        let stroke = |s: &str, width: f64| {
            format!(
                "q {} {} w 1 J 1 j {s}S Q",
                color.stroke(),
                num(width * side)
            )
        };
        let content = match mark {
            Mark::Check => stroke(&poly(&[(0.17, 0.53), (0.40, 0.28), (0.83, 0.77)]), 0.12),
            Mark::Cross => stroke(
                &(poly(&[(0.24, 0.24), (0.76, 0.76)]) + &poly(&[(0.24, 0.76), (0.76, 0.24)])),
                0.12,
            ),
            Mark::Dot => {
                let r = if radio {
                    (f.w.min(f.h) / 2.0 - f.drawn_border()).max(0.0) * 0.5
                } else {
                    side * 0.3
                };
                format!("q {} {} f Q", color.fill(), circle(cx, cy, r))
            }
            Mark::Diamond => format!(
                "q {} {}h f Q",
                color.fill(),
                poly(&[(0.5, 0.12), (0.88, 0.5), (0.5, 0.88), (0.12, 0.5)])
            ),
            Mark::Square => format!(
                "q {} {}h f Q",
                color.fill(),
                poly(&[(0.22, 0.22), (0.78, 0.22), (0.78, 0.78), (0.22, 0.78)])
            ),
            Mark::Star => {
                let points: Vec<(f64, f64)> = (0..10)
                    .map(|i| {
                        let angle =
                            std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::PI / 5.0;
                        let r = if i % 2 == 0 { 0.42 } else { 0.17 };
                        (0.5 + r * angle.cos(), 0.47 + r * angle.sin())
                    })
                    .collect();
                format!("q {} {}h f Q", color.fill(), poly(&points))
            }
            Mark::Glyph(code) => {
                let Some((name, source)) = dingbats else {
                    return;
                };
                self.fonts.push((name.clone(), source));
                // A dingbat fits an em box; its advance is taken as 0.8 em
                // and its middle at 0.35 em, clipped to the inner box.
                let size = side * 0.8;
                let x = cx - 0.4 * size;
                let y = cy - 0.35 * size;
                let clip = f.inner().re();
                format!(
                    "q {clip} W n BT {} {} {} Tf {} {} Td {} Tj ET Q",
                    color.fill(),
                    pdf_name(&name),
                    num(size),
                    num(x),
                    num(y),
                    text::literal(&[code])
                )
            }
        };
        self.line(&content);
    }
}

/// A circle path of four Bézier arcs.
fn circle(cx: f64, cy: f64, r: f64) -> String {
    let k = r * 0.552_284_75;
    let p = |x: f64, y: f64| format!("{} {}", num(x), num(y));
    format!(
        "{} m {} {} {} c {} {} {} c {} {} {} c {} {} {} c h",
        p(cx + r, cy),
        p(cx + r, cy + k),
        p(cx + k, cy + r),
        p(cx, cy + r),
        p(cx - k, cy + r),
        p(cx - r, cy + k),
        p(cx - r, cy),
        p(cx - r, cy - k),
        p(cx - k, cy - r),
        p(cx, cy - r),
        p(cx + k, cy - r),
        p(cx + r, cy - k),
        p(cx + r, cy),
    )
}

/// `v` rounded down to 1/100, so a size that fits still fits once written.
fn floor2(v: f64) -> f64 {
    if v.is_finite() {
        (v * 100.0).floor() / 100.0
    } else {
        MAX_FONT_SIZE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn da_strings_parse_with_defaults() {
        let da = Da::parse(b"/TiRo 9 Tf 1 0 0 rg");
        assert_eq!(da.font, b"TiRo");
        assert_eq!(da.size, Some(9.0));
        assert_eq!(da.color, Some(Color::Rgb(1.0, 0.0, 0.0)));
        assert_eq!(Da::parse(b"/Helv -5 Tf").size, None);
        assert_eq!(Da::parse(b"/Helv 0 Tf").size, None);
        assert_eq!(Da::parse(b"/Helv 0.0000001 Tf").size, Some(MIN_FONT_SIZE));
        let soup = Da::parse(b"Tf Tf rg /");
        assert_eq!(
            (soup.font.as_slice(), soup.size, soup.color),
            (&b"Helv"[..], Some(12.0), None)
        );
        assert_eq!(Da::parse(&vec![b' '; MAX_DA_BYTES + 1]).size, Some(12.0));
        assert_eq!(
            Da::parse(b"0.5 g /F 1 2 3 Tf").color,
            Some(Color::Gray(0.5))
        );
    }

    #[test]
    fn numbers_and_names_are_written_safely() {
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(12.0), "12");
        assert!(!num(1e300).contains('e'));
        assert_eq!(pdf_name(b"A B#"), "/A#20B#23");
    }

    use crate::document::DocumentLimits;
    use crate::object::ObjectId;
    use std::time::{Duration, Instant};

    /// A one-page form: object 5 the widget `widget`, objects 6.. `extra`.
    fn pdf(widget: &str, extra: &[String]) -> Vec<u8> {
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Annots [5 0 R] >>".into(),
            "<< /Fields [5 0 R] /DA (/Helv 12 Tf 0 g) >>".into(),
            format!("<< /Type /Annot /Subtype /Widget /P 3 0 R {widget} >>"),
        ];
        objects.extend(extra.iter().cloned());
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }

    fn build(bytes: &[u8]) -> (Synth, usize) {
        let doc = Document::open(bytes, DocumentLimits::default()).unwrap();
        let widget = doc
            .resolve_id(ObjectId::new(5, 0).unwrap())
            .as_dict()
            .unwrap()
            .clone();
        let context = Context::of(&doc);
        let synth = context.synthesize(&widget).unwrap();
        (synth, context.work())
    }

    fn content(form: &Form) -> String {
        String::from_utf8_lossy(&form.content).into_owned()
    }

    /// Review fix 3: selection is a set lookup. A tall list at a tiny size
    /// shows 65,536 rows against a /V of 65,536 values; a scan of the
    /// values per row (4 billion comparisons) would not finish in time.
    #[test]
    fn a_long_selection_against_many_rows_is_a_set_lookup() {
        let count = 65_536;
        let opts: String = (0..count).map(|i| format!("(o{i})")).collect();
        let values: String = (0..count).step_by(2).map(|i| format!("(o{i})")).collect();
        let widget = format!(
            "/FT /Ch /T (x) /Rect [0 0 100 9000] /DA (/Helv 0.1 Tf 0 g) /Opt [{opts}] /V [{values}]"
        );
        let started = Instant::now();
        let (synth, _) = build(&pdf(&widget, &[]));
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "{:?}",
            started.elapsed()
        );
        let Synth::Single(form) = synth else {
            panic!("a single form");
        };
        let text = content(&form);
        // The selection band: one rectangle per selected row.
        let bands = text
            .lines()
            .find(|l| l.contains(" f Q") && l.contains(" re "))
            .unwrap();
        assert_eq!(
            bands.matches(" re").count(),
            count / 2 + 1,
            "the clip and each band"
        );
        assert!(text.contains("(o65535) Tj"));
    }

    /// Review fix 4: tens of thousands of options referencing one 64K-character string
    /// read at most the option budget, and the combo box then shows /V.
    #[test]
    fn option_reads_stay_within_their_budget() {
        let long = "a".repeat(65_535);
        // As many references as one parsed value may hold here: 40,000
        // reads of 64K characters each would be 2.6 billion characters.
        let refs = "6 0 R ".repeat(40_000);
        let half = "6 0 R ".repeat(30_000);
        let extra = [format!("({long})")];
        let started = Instant::now();
        let combo = format!(
            "/FT /Ch /Ff 131072 /T (x) /Rect [0 0 100 20] /DA (/Helv 12 Tf 0 g) /Opt [{refs}] /V ({long}b)"
        );
        let (synth, work) = build(&pdf(&combo, &extra));
        assert!(work < 3 * MAX_OPTION_CHARS, "combo work {work}");
        assert!(
            content(synth.forms()[0]).contains("ab) Tj"),
            "/V itself is shown"
        );
        let list = format!(
            "/FT /Ch /T (x) /Rect [0 0 100 9000] /DA (/Helv 0.1 Tf 0 g) /Opt [{half}] /V [{half}]"
        );
        let (_, work) = build(&pdf(&list, &extra));
        assert!(work < 3 * MAX_OPTION_CHARS, "list work {work}");
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "{:?}",
            started.elapsed()
        );
    }

    /// Review fix 7: an empty /AC is no alternate caption.
    #[test]
    fn an_empty_alternate_caption_shows_the_caption() {
        let widget = "/FT /Btn /Ff 65536 /T (x) /Rect [0 0 100 30] /DA (/Helv 12 Tf 0 g) /MK << /CA (Go) /AC () >>";
        let (synth, _) = build(&pdf(widget, &[]));
        let Synth::Push { normal, down } = synth else {
            panic!("a push button");
        };
        assert!(content(&normal).contains("(Go) Tj"));
        assert!(content(&down).contains("(Go) Tj"), "{}", content(&down));
        let widget = "/FT /Btn /Ff 65536 /T (x) /Rect [0 0 100 30] /DA (/Helv 12 Tf 0 g) /MK << /CA (Go) /AC (Down) >>";
        let (synth, _) = build(&pdf(widget, &[]));
        assert!(content(synth.forms()[1]).contains("(Down) Tj"));
    }
}
