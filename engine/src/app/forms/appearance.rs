// SPDX-License-Identifier: AGPL-3.0-or-later
//! Widget appearance synthesis, as MuPDF 1.28's `pdf_update_appearance`
//! writes it (measured: `reference/app-forms-20261015`): text fields
//! (single line, multiline, comb, alignment, the DA font and size, auto
//! size), check boxes and radio buttons (Off and on states), combo and list
//! boxes, and push buttons (normal and down). The differences from MuPDF are
//! listed in `design/app-forms.md`. Signature fields get none.
use crate::document::Document;
use crate::object::{Dictionary, Object};

use super::text::{self, Family, Metrics, break_lines, literal, num, win_ansi};
use super::{
    COMB, Choice, MULTILINE, PASSWORD, WidgetType, choices, field_flags, inherited, on_state,
    pdf_rect, text_of, to_int, value_text, widget_type,
};

/// Characters of a value drawn at most; a longer value is cut there.
pub const MAX_DRAWN_CHARS: usize = 65_536;
/// MuPDF's line height, in font sizes, for every font.
const LINE_HEIGHT: f64 = 1.116;
/// MuPDF's baseline, in font sizes below the top of a single line.
const BASELINE: f64 = 0.8;
/// How far a selected list line's band reaches below its baseline.
const DESCENT: f64 = 0.21;
/// The bezier constant MuPDF draws circles with.
const CIRCLE: f64 = 0.551_915;
/// Acrobat's list selection colour.
const SELECTED: &str = ".6 .75686 .8549 rg\n";

/// A font an appearance's /Resources names.
#[derive(Clone, Debug, PartialEq)]
pub enum FontSource {
    /// The /AcroForm /DR font, as /DR holds it (a reference or a dictionary).
    Existing(Object),
    /// A new simple base 14 font with this /BaseFont (WinAnsiEncoding unless
    /// symbolic).
    New(&'static [u8]),
}

/// One appearance stream: a Form XObject of `width` by `height` (the
/// widget's size, swapped for a quarter turn of /MK /R) rotated by `rotate`.
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub width: f64,
    pub height: f64,
    pub rotate: i64,
    pub content: Vec<u8>,
    pub fonts: Vec<(Vec<u8>, FontSource)>,
}

impl Form {
    /// The stream dictionary, fonts named by `font_refs` (in `fonts` order).
    pub fn dictionary(&self, font_refs: &[Object]) -> Dictionary {
        let mut dict = Dictionary::new();
        dict.insert(b"Type".to_vec(), Object::name("XObject"));
        dict.insert(b"Subtype".to_vec(), Object::name("Form"));
        let bbox = [0.0, 0.0, self.width, self.height];
        dict.insert(b"BBox".to_vec(), super::rect_object(bbox));
        let matrix: [i64; 4] = match self.rotate {
            90 => [0, 1, -1, 0],
            180 => [-1, 0, 0, -1],
            270 => [0, -1, 1, 0],
            _ => [1, 0, 0, 1],
        };
        let matrix = matrix.into_iter().chain([0, 0]).map(Object::Integer);
        dict.insert(b"Matrix".to_vec(), Object::array(matrix));
        if !self.fonts.is_empty() {
            let fonts: Dictionary = (self.fonts.iter())
                .zip(font_refs)
                .map(|((name, _), font)| (name.clone(), font.clone()))
                .collect();
            let resources = Dictionary::from([(b"Font".to_vec(), Object::Dictionary(fonts))]);
            dict.insert(b"Resources".to_vec(), Object::Dictionary(resources));
        }
        dict
    }
}

/// A widget's synthesized appearance.
#[derive(Clone, Debug, PartialEq)]
pub enum Synth {
    /// A text or choice field: /AP /N.
    Single(Form),
    /// A check box or radio button: /AP /N with Off and the on state.
    States {
        off: Form,
        on_name: Vec<u8>,
        on: Form,
    },
    /// A push button: /AP /N and /D.
    Push { normal: Form, down: Form },
}

impl Synth {
    pub fn forms(&self) -> Vec<&Form> {
        match self {
            Self::Single(form) => vec![form],
            Self::States { off, on, .. } => vec![off, on],
            Self::Push { normal, down } => vec![normal, down],
        }
    }
}

/// The form's defaults: /AcroForm /DA, /Q and /DR /Font.
pub struct Context<'d, 'a> {
    doc: &'d Document<'a>,
    acro: Option<&'d Dictionary>,
    fonts: Option<&'d Dictionary>,
}

impl<'d, 'a> Context<'d, 'a> {
    pub fn of(doc: &'d Document<'a>) -> Self {
        let catalog = doc.resolve_id(doc.root()).as_dict();
        let acro = catalog.and_then(|catalog| doc.dict_get(catalog, b"AcroForm").as_dict());
        let dr = acro.and_then(|acro| doc.dict_get(acro, b"DR").as_dict());
        let fonts = dr.and_then(|dr| doc.dict_get(dr, b"Font").as_dict());
        Self { doc, acro, fonts }
    }

    /// /AcroForm /NeedAppearances is true.
    pub fn need_appearances(&self) -> bool {
        (self.acro)
            .is_some_and(|acro| self.doc.dict_get(acro, b"NeedAppearances").as_bool() == Some(true))
    }

    /// MuPDF's `pdf_update_appearance` for a widget: None for a signature
    /// field or a widget without a field type.
    pub fn synthesize(&self, widget: &Dictionary) -> Option<Synth> {
        self.synthesize_with(widget, None)
    }

    /// `synthesize`, a button's on state being `on` when its /AP has none.
    pub fn synthesize_with(&self, widget: &Dictionary, on: Option<&[u8]>) -> Option<Synth> {
        let doc = self.doc;
        if doc.dict_get(widget, b"Subtype").as_name() != Some(b"Widget")
            || inherited(doc, widget, b"FT").as_name().is_none()
        {
            return None;
        }
        let rect = pdf_rect(doc, widget);
        let (mut w, mut h) = (rect[2] - rect[0], rect[3] - rect[1]);
        let mk = doc.dict_get(widget, b"MK").as_dict();
        let rotate = mk.map_or(0, |mk| to_int(doc.dict_get(mk, b"R")));
        let rotate = match rotate.rem_euclid(360) {
            r @ (90 | 180 | 270) => r,
            _ => 0,
        };
        if rotate == 90 || rotate == 270 {
            (w, h) = (h, w);
        }
        let style = Style {
            width: w,
            height: h,
            rotate,
            border: self.border_width(widget),
            background: mk.and_then(|mk| color(doc, doc.dict_get(mk, b"BG"))),
            border_color: mk.and_then(|mk| color(doc, doc.dict_get(mk, b"BC"))),
            caption: mk.map(|mk| text_of(doc.dict_get(mk, b"CA"))),
        };
        let da = self.default_appearance(widget);
        match widget_type(doc, widget) {
            WidgetType::Text => Some(Synth::Single(self.text_field(widget, &style, &da))),
            WidgetType::Combo => Some(Synth::Single(self.combo(widget, &style, &da))),
            WidgetType::List => Some(Synth::Single(self.list(widget, &style, &da))),
            WidgetType::Checkbox | WidgetType::Radio => Some(self.button(widget, &style, &da, on)),
            WidgetType::PushButton => Some(self.push_button(&style, &da)),
            WidgetType::Signature => None,
        }
    }

    /// /BS /W, else /Border's width, else 1.
    fn border_width(&self, widget: &Dictionary) -> f64 {
        let doc = self.doc;
        let bs = doc.dict_get(widget, b"BS").as_dict();
        let width = match bs.map(|bs| doc.dict_get(bs, b"W")) {
            Some(width) if !width.is_null() => width.as_f64(),
            _ => doc
                .dict_get(widget, b"Border")
                .as_array()
                .and_then(|border| border.get(2))
                .and_then(|width| doc.resolve(width).as_f64()),
        };
        width.filter(|w| w.is_finite() && *w >= 0.0).unwrap_or(1.0)
    }

    /// The widget's inherited /DA, else the form's, parsed.
    fn default_appearance(&self, widget: &Dictionary) -> Da {
        let doc = self.doc;
        let da = match inherited(doc, widget, b"DA") {
            Object::String(da) => Some(da.as_slice()),
            _ => (self.acro).and_then(|acro| doc.dict_get(acro, b"DA").as_string()),
        };
        Da::parse(da.unwrap_or(b""))
    }

    /// The inherited /Q, else the form's.
    fn quadding(&self, widget: &Dictionary) -> i64 {
        let doc = self.doc;
        let q = inherited(doc, widget, b"Q");
        let q = if q.is_null() {
            self.acro
                .map_or(&Object::Null, |acro| doc.dict_get(acro, b"Q"))
        } else {
            q
        };
        to_int(q)
    }

    /// The font the DA names: the /DR font when it is a WinAnsi simple font,
    /// otherwise a new base 14 font of its family (Helvetica when /DR lacks
    /// it), under the DA's name.
    fn font(&self, name: &[u8]) -> (FontSource, Metrics) {
        let doc = self.doc;
        let entry = self.fonts.and_then(|fonts| fonts.get(name));
        let Some(dict) = entry.and_then(|value| text::font_dict(doc, value)) else {
            return (
                FontSource::New(b"Helvetica"),
                Metrics::family(Family::Helvetica),
            );
        };
        let simple = matches!(
            doc.dict_get(dict, b"Subtype").as_name(),
            Some(b"Type1" | b"TrueType" | b"MMType1")
        );
        let win_ansi = doc.dict_get(dict, b"Encoding").as_name() == Some(b"WinAnsiEncoding");
        let base = doc.dict_get(dict, b"BaseFont").as_name().unwrap_or(b"");
        let family = Family::of(base);
        match entry {
            Some(value) if simple && win_ansi && !family.symbolic() => (
                FontSource::Existing(value.clone()),
                Metrics::font(doc, dict),
            ),
            _ => {
                let family = if family.symbolic() {
                    Family::Helvetica
                } else {
                    family
                };
                (FontSource::New(family.base_font()), Metrics::family(family))
            }
        }
    }

    /// ZapfDingbats from /DR when a font there is it with its own encoding,
    /// else a new one.
    fn dingbats(&self) -> (Vec<u8>, FontSource) {
        let doc = self.doc;
        let found = self.fonts.and_then(|fonts| {
            fonts.iter().find(|(_, value)| {
                text::font_dict(doc, value).is_some_and(|dict| {
                    let base = doc.dict_get(dict, b"BaseFont").as_name();
                    base == Some(b"ZapfDingbats") && dict.get(b"Encoding".as_slice()).is_none()
                })
            })
        });
        match found {
            Some((name, value)) => (name.clone(), FontSource::Existing(value.clone())),
            None => (b"ZaDb".to_vec(), FontSource::New(b"ZapfDingbats")),
        }
    }

    /// MuPDF's `pdf_write_tx_widget_appearance`.
    fn text_field(&self, widget: &Dictionary, style: &Style, da: &Da) -> Form {
        let doc = self.doc;
        let flags = field_flags(doc, widget);
        let mut value = value_text(doc, widget);
        if flags & PASSWORD != 0 {
            value = "*".repeat(value.chars().take(MAX_DRAWN_CHARS).count());
        }
        let (source, metrics) = self.font(&da.font);
        let mut out = style.frame("/Tx BMC\nq\n");
        let b = style.border;
        let text = Text {
            metrics: &metrics,
            da,
            quadding: self.quadding(widget),
        };
        if flags & COMB != 0 {
            let max = to_int(inherited(doc, widget, b"MaxLen")).max(0);
            if style.border_color.is_some() && max > 1 {
                let cell = (style.width - 2.0 * b) / max as f64;
                for i in 1..max.min(4096) {
                    let x = num(b + cell * i as f64);
                    out.push_str(&format!(
                        "{x} {} m {x} {} l s\n",
                        num(b),
                        num(style.height - b)
                    ));
                }
            }
            text.comb(&mut out, &value, style.width, style.height, max);
        } else if flags & MULTILINE != 0 {
            text.multiline(&mut out, &value, style, 2.0 * b, &[]);
        } else {
            text.single(&mut out, &value, style.width, style.height, 2.0 * b);
        }
        out.push_str("Q\nEMC\n");
        style.form(out, vec![(da.font.clone(), source)])
    }

    /// A combo box: its value's display text on one line.
    fn combo(&self, widget: &Dictionary, style: &Style, da: &Da) -> Form {
        let doc = self.doc;
        let value = value_text(doc, widget);
        let shown = choices(doc, widget)
            .into_iter()
            .find(|choice| choice.export == value)
            .map_or(value, |choice| choice.display);
        let (source, metrics) = self.font(&da.font);
        let mut out = style.frame("/Tx BMC\nq\n");
        let text = Text {
            metrics: &metrics,
            da,
            quadding: self.quadding(widget),
        };
        text.single(
            &mut out,
            &shown,
            style.width,
            style.height,
            2.0 * style.border,
        );
        out.push_str("Q\nEMC\n");
        style.form(out, vec![(da.font.clone(), source)])
    }

    /// A list box: the options from /TI, one after another as multiline
    /// text, the selected ones highlighted.
    fn list(&self, widget: &Dictionary, style: &Style, da: &Da) -> Form {
        let doc = self.doc;
        let top = usize::try_from(to_int(inherited(doc, widget, b"TI"))).unwrap_or(0);
        let all: Vec<Choice> = choices(doc, widget);
        let selected = self.selection(widget);
        let shown: Vec<(&Choice, bool)> = (all.iter().skip(top))
            .map(|choice| (choice, selected.contains(&choice.export)))
            .collect();
        let (source, metrics) = self.font(&da.font);
        let mut out = style.frame("/Tx BMC\nq\n");
        let text = Text {
            metrics: &metrics,
            da,
            quadding: self.quadding(widget),
        };
        let value: Vec<&str> = shown.iter().map(|(c, _)| c.display.as_str()).collect();
        let marks: Vec<bool> = shown.iter().map(|(_, on)| *on).collect();
        text.multiline(
            &mut out,
            &value.join("\n"),
            style,
            2.0 * style.border,
            &marks,
        );
        out.push_str("Q\nEMC\n");
        style.form(out, vec![(da.font.clone(), source)])
    }

    /// The selected export values: /V as a string or an array of strings.
    fn selection(&self, widget: &Dictionary) -> Vec<String> {
        let doc = self.doc;
        match inherited(doc, widget, b"V") {
            Object::Array(items) => items.iter().map(|v| text_of(doc.resolve(v))).collect(),
            Object::Name(name) => vec![super::name_text(name)],
            value => vec![text_of(value)],
        }
    }

    /// A check box or radio button's Off and on states. The on state is the
    /// widget's own (its /AP /N or /D key, else `hint`, else /AS or /V when
    /// not Off), else Yes.
    fn button(&self, widget: &Dictionary, style: &Style, da: &Da, hint: Option<&[u8]>) -> Synth {
        let doc = self.doc;
        let radio = widget_type(doc, widget) == WidgetType::Radio;
        let mut on_name = on_state(doc, widget);
        if on_name.is_empty() {
            let down = doc.dict_get(
                doc.dict_get(widget, b"AP").as_dict().unwrap_or(&EMPTY),
                b"D",
            );
            on_name = (down.as_dict())
                .and_then(|states| states.keys().find(|key| key.as_slice() != b"Off"))
                .cloned()
                .unwrap_or_default();
        }
        if on_name.is_empty()
            && let Some(hint) = hint
        {
            on_name = hint.to_vec();
        }
        if on_name.is_empty() {
            let named = |key: &[u8]| match inherited(doc, widget, key) {
                Object::Name(name) if name.as_slice() != b"Off" => Some(name.clone()),
                _ => None,
            };
            let own = doc
                .dict_get(widget, b"AS")
                .as_name()
                .filter(|n| *n != b"Off");
            on_name = (own.map(<[u8]>::to_vec))
                .or_else(|| named(b"V"))
                .unwrap_or_else(|| b"Yes".to_vec());
        }
        let s = style.width.min(style.height);
        let b = style.border;
        let caption = style.caption.as_deref().map(win_ansi).unwrap_or_default();
        let glyph = match caption.as_slice() {
            [glyph] => Some(*glyph),
            _ => None,
        };
        let circle_dot = radio && glyph.is_none_or(|glyph| glyph == b'l');
        let mut base = String::from("q\n");
        if radio {
            if let Some(bg) = &style.background {
                base.push_str(&bg.op(false));
                circle(&mut base, s / 2.0, s / 2.0, s / 2.0);
                base.push_str("f\n");
            }
            base.push_str(&format!("{} w\n", num(b)));
            if let (Some(bc), true) = (&style.border_color, b > 0.0) {
                base.push_str(&bc.op(true));
                circle(&mut base, s / 2.0 - b / 2.0, s / 2.0, s / 2.0);
                base.push_str("s\n");
            }
        } else {
            if let Some(bg) = &style.background {
                base.push_str(&format!(
                    "{}0 0 {} {} re\nf\n",
                    bg.op(false),
                    num(s),
                    num(s)
                ));
            }
            base.push_str(&format!("{} w\n", num(b)));
            if let (Some(bc), true) = (&style.border_color, b > 0.0) {
                let (half, side) = (num(b / 2.0), num(s - b));
                base.push_str(&format!(
                    "{}{half} {half} {side} {side} re\nS\n",
                    bc.op(true)
                ));
            }
        }
        let mut on = base.clone();
        let mark = da
            .color
            .as_ref()
            .map_or_else(|| "0 g\n".into(), |c| c.op(false));
        let mut fonts = Vec::new();
        if circle_dot {
            on.push_str(&mark);
            circle(&mut on, s / 4.0 - b / 2.0, s / 2.0, s / 2.0);
            on.push_str("f\n");
        } else {
            let (name, source) = self.dingbats();
            let glyph = literal(&[glyph.unwrap_or(b'3')]);
            on.push_str("BT\n");
            on.push_str(&mark);
            on.push_str(&format!("{} {} Td\n", num(b + s * 0.1), num(s * 0.2)));
            on.push_str(&format!("/{} {} Tf\n", name_token(&name), num(s)));
            on.push_str(&glyph);
            on.push_str(" Tj\nET\n");
            fonts.push((name, source));
        }
        base.push_str("Q\n");
        on.push_str("Q\n");
        Synth::States {
            off: style.form(base, Vec::new()),
            on_name,
            on: style.form(on, fonts),
        }
    }

    /// MuPDF's push button: a bevel in the background colour and the
    /// caption centred; the down state swaps the bevel and moves the caption.
    fn push_button(&self, style: &Style, da: &Da) -> Synth {
        let (w, h, b) = (style.width, style.height, style.border);
        let (source, metrics) = self.font(&da.font);
        let caption = style.caption.as_deref().map(win_ansi).unwrap_or_default();
        let caption: Vec<u8> = caption.into_iter().take(MAX_DRAWN_CHARS).collect();
        let advance = metrics.width(&caption);
        let size = if da.size != 0.0 {
            da.size
        } else if advance > 0.0 {
            (h - 14.0).min((w - 14.0) / advance).max(1.0)
        } else {
            (h - 14.0).max(1.0)
        };
        let x = (w - advance * size) / 2.0;
        let y = (h - size) / 2.0 + 0.2 * size;
        let state = |down: bool| {
            let mut out = format!("q\n{} w\n", num(b));
            if let Some(bg) = &style.background {
                let bg = bg.rgb();
                out.push_str(&bg.op(false));
                out.push_str(&format!("0 0 {} {} re\nf\n", num(w), num(h)));
            }
            if let (Some(bc), true) = (&style.border_color, b > 0.0) {
                out.push_str(&bc.rgb().op(true));
                let (half, ww, hh) = (num(b / 2.0), num(w - b), num(h - b));
                out.push_str(&format!("{half} {half} {ww} {hh} re\nS\n"));
            }
            if let Some(bg) = &style.background {
                let bg = bg.rgb();
                let light = bg.map(|v| (v + 1.0) / 2.0);
                let dark = bg.map(|v| v / 2.0);
                let (first, second) = if down { (dark, light) } else { (light, dark) };
                let (i, o) = (b + 2.0, b);
                let p = |x: f64, y: f64| format!("{} {}", num(x), num(y));
                out.push_str(&first.op(false));
                out.push_str(&format!(
                    "{} m {} l {} l {} l {} l {} l f\n",
                    p(o, o),
                    p(o, h - o),
                    p(w - o, h - o),
                    p(w - i, h - i),
                    p(i, h - i),
                    p(i, i)
                ));
                out.push_str(&second.op(false));
                out.push_str(&format!(
                    "{} m {} l {} l {} l {} l {} l f\n",
                    p(o, o),
                    p(i, i),
                    p(w - i, i),
                    p(w - i, h - i),
                    p(w - o, h - o),
                    p(w - o, o)
                ));
            }
            if down {
                out.push_str("1 0 0 1 2 -2 cm\n");
            }
            out.push_str("BT\n");
            if let Some(color) = &da.color {
                out.push_str(&color.op(false));
            }
            out.push_str(&format!("{} {} Td\n", num(x), num(y)));
            if !caption.is_empty() {
                out.push_str(&format!("/{} {} Tf\n", name_token(&da.font), num(size)));
                out.push_str(&literal(&caption));
                out.push_str(" Tj\n");
            }
            out.push_str("ET\nQ\n");
            style.form(out, vec![(da.font.clone(), source.clone())])
        };
        Synth::Push {
            normal: state(false),
            down: state(true),
        }
    }
}

static EMPTY: Dictionary = Dictionary::new();

/// A name token's text: regular characters as they are, the rest `#xx`.
fn name_token(name: &[u8]) -> String {
    let mut out = String::new();
    for &byte in name {
        let regular = byte > b' ' && byte < 0x7F && !b"()<>[]{}/%#".contains(&byte);
        if regular {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("#{byte:02X}"));
        }
    }
    out
}

/// A /DA string read as MuPDF's `pdf_parse_default_appearance`: the last
/// `Tf` sets the font and size (Helv 12 by default), the last `g`, `rg` or
/// `k` the colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Da {
    pub font: Vec<u8>,
    pub size: f64,
    pub color: Option<Color>,
}

impl Da {
    pub fn parse(da: &[u8]) -> Self {
        let mut out = Self {
            font: b"Helv".to_vec(),
            size: 12.0,
            color: None,
        };
        let mut numbers: Vec<f64> = Vec::new();
        let mut name: Option<Vec<u8>> = None;
        for token in tokens(da) {
            if let Some(rest) = token.strip_prefix(b"/") {
                name = Some(rest.to_vec());
                continue;
            }
            if let Some(number) = std::str::from_utf8(token)
                .ok()
                .and_then(|t| t.parse::<f64>().ok())
            {
                if number.is_finite() && numbers.len() < 64 {
                    numbers.push(number);
                }
                continue;
            }
            let last = |n: usize| {
                numbers
                    .get(numbers.len().saturating_sub(n)..)
                    .filter(|v| v.len() == n)
            };
            match token {
                b"Tf" => {
                    if let Some(font) = name.take() {
                        out.font = font;
                    }
                    if let Some(size) = last(1).and_then(|v| v.first()) {
                        out.size = size.clamp(-10_000.0, 10_000.0);
                    }
                }
                b"g" => out.color = last(1).map(|v| Color(v.to_vec())),
                b"rg" => out.color = last(3).map(|v| Color(v.to_vec())),
                b"k" => out.color = last(4).map(|v| Color(v.to_vec())),
                _ => {}
            }
            numbers.clear();
        }
        out
    }
}

/// Whitespace- and delimiter-separated tokens; a name keeps its slash.
fn tokens(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = None;
    for (at, &byte) in bytes.iter().enumerate() {
        let space = byte.is_ascii_whitespace() || byte == 0;
        let delimiter = b"()<>[]{}%".contains(&byte);
        if byte == b'/' || space || delimiter {
            if let Some(from) = start.take() {
                out.extend(bytes.get(from..at));
            }
            if byte == b'/' {
                start = Some(at);
            }
            continue;
        }
        if start.is_none() {
            start = Some(at);
        }
    }
    if let Some(from) = start {
        out.extend(bytes.get(from..));
    }
    out
}

/// A colour of 1, 3 or 4 components (gray, RGB, CMYK).
#[derive(Clone, Debug, PartialEq)]
pub struct Color(Vec<f64>);

impl Color {
    /// The fill (`g`, `rg`, `k`) or stroke operator with its operands.
    fn op(&self, stroke: bool) -> String {
        let name = match (self.0.len(), stroke) {
            (1, false) => "g",
            (1, true) => "G",
            (3, false) => "rg",
            (3, true) => "RG",
            (_, false) => "k",
            (_, true) => "K",
        };
        let values: Vec<String> = self.0.iter().map(|&v| num(v)).collect();
        format!("{} {name}\n", values.join(" "))
    }

    fn rgb(&self) -> Self {
        match self.0.as_slice() {
            [g] => Self(vec![*g; 3]),
            [c, m, y, k] => Self(vec![
                1.0 - (c + k).min(1.0),
                1.0 - (m + k).min(1.0),
                1.0 - (y + k).min(1.0),
            ]),
            other => Self(other.to_vec()),
        }
    }

    fn map(&self, f: impl Fn(f64) -> f64) -> Self {
        Self(self.0.iter().map(|&v| f(v)).collect())
    }
}

/// A colour array of 1, 3 or 4 numbers; anything else is no colour.
fn color(doc: &Document<'_>, value: &Object) -> Option<Color> {
    let items = value.as_array()?;
    if !matches!(items.len(), 1 | 3 | 4) {
        return None;
    }
    let values = items.iter().map(|v| {
        doc.resolve(v)
            .as_f64()
            .filter(|v| v.is_finite())
            .unwrap_or(0.0)
    });
    Some(Color(values.collect()))
}

/// The widget's frame: size, rotation, border, colours, caption.
struct Style {
    width: f64,
    height: f64,
    rotate: i64,
    border: f64,
    background: Option<Color>,
    border_color: Option<Color>,
    caption: Option<String>,
}

impl Style {
    /// `prefix`, the background, the border and the clip of a text widget.
    fn frame(&self, prefix: &str) -> String {
        let (w, h, b) = (self.width, self.height, self.border);
        let mut out = String::from(prefix);
        if let Some(bg) = &self.background {
            out.push_str(&format!(
                "{}0 0 {} {} re\nf\n",
                bg.op(false),
                num(w),
                num(h)
            ));
        }
        out.push_str(&format!("{} w\n", num(b)));
        if let (Some(bc), true) = (&self.border_color, b > 0.0) {
            let half = num(b / 2.0);
            out.push_str(&format!(
                "{}{half} {half} {} {} re\ns\n",
                bc.op(true),
                num(w - b),
                num(h - b)
            ));
        }
        out.push_str(&format!(
            "{} {} {} {} re\nW\nn\n",
            num(b),
            num(b),
            num(w - 2.0 * b),
            num(h - 2.0 * b)
        ));
        out
    }

    fn form(&self, content: String, fonts: Vec<(Vec<u8>, FontSource)>) -> Form {
        Form {
            width: self.width,
            height: self.height,
            rotate: self.rotate,
            content: content.into_bytes(),
            fonts,
        }
    }
}

/// Variable text in one font (MuPDF's `write_variable_text`).
struct Text<'m> {
    metrics: &'m Metrics,
    da: &'m Da,
    quadding: i64,
}

impl Text<'_> {
    fn begin(&self, out: &mut String) {
        out.push_str("BT\n");
        if let Some(color) = &self.da.color {
            out.push_str(&color.op(false));
        }
    }

    fn show(&self, out: &mut String, bytes: &[u8], size: f64) {
        out.push_str(&format!(
            "/{} {} Tf\n",
            name_token(&self.da.font),
            num(size)
        ));
        out.push_str(&literal(bytes));
        out.push_str(" Tj\n");
    }

    fn encode(value: &str) -> Vec<u8> {
        let cut: String = value.chars().take(MAX_DRAWN_CHARS).collect();
        win_ansi(&cut)
    }

    /// One line, auto size fitting the height and the width.
    fn single(&self, out: &mut String, value: &str, w: f64, h: f64, padding: f64) {
        let (w, h) = (w - 2.0 * padding, h - 2.0 * padding);
        let bytes = Self::encode(value);
        let advance = self.metrics.width(&bytes);
        let size = auto_size(self.da.size, advance, w, h);
        let tx = match self.quadding {
            1 => (w - advance * size) / 2.0,
            2 => w - advance * size,
            _ => 0.0,
        };
        let ty = (h - size) / 2.0;
        self.begin(out);
        let y = padding + h - BASELINE * size - ty;
        out.push_str(&format!("{} {} Td\n", num(padding + tx), num(y)));
        if !bytes.is_empty() {
            self.show(out, &bytes, size);
        }
        out.push_str("ET\n");
    }

    /// Lines from the top, 12 pt when the DA size is 0; `marks` highlights
    /// the lines of the selected list options (one per option, in order).
    fn multiline(
        &self,
        out: &mut String,
        value: &str,
        style: &Style,
        padding: f64,
        marks: &[bool],
    ) {
        let (w, h) = (style.width - 2.0 * padding, style.height - 2.0 * padding);
        let size = if self.da.size == 0.0 {
            12.0
        } else {
            self.da.size
        };
        let step = LINE_HEIGHT * size;
        // Lines below the clip are never drawn.
        let visible = if step > 0.0 {
            ((h / step).ceil().max(0.0) as usize).saturating_add(2)
        } else {
            usize::MAX
        };
        let mut lines = Vec::new();
        let mut owners = Vec::new();
        if marks.is_empty() {
            lines = break_lines(self.metrics, &Self::encode(value), size, w, visible);
        } else {
            for (option, text) in value.split('\n').enumerate() {
                if lines.len() >= visible {
                    break;
                }
                let broken = break_lines(self.metrics, &Self::encode(text), size, w, visible);
                let count = broken.len().max(1);
                if broken.is_empty() {
                    lines.push(text::Line {
                        bytes: Vec::new(),
                        width: 0.0,
                    });
                } else {
                    lines.extend(broken);
                }
                owners.extend(std::iter::repeat_n(option, count));
            }
        }
        for (index, owner) in owners.iter().enumerate() {
            if marks.get(*owner) == Some(&true) {
                // The line's band, from just below its descenders.
                let baseline = padding + h - (index as f64 + 1.0) * step;
                out.push_str(SELECTED);
                out.push_str(&format!(
                    "{} {} {} {} re\nf\n",
                    num(style.border),
                    num(baseline - DESCENT * size),
                    num(style.width - 2.0 * style.border),
                    num(step)
                ));
            }
        }
        self.begin(out);
        out.push_str(&format!("{} {} Td\n", num(padding), num(padding + h)));
        let mut previous = 0.0;
        for line in &lines {
            let x = match self.quadding {
                1 => (w - line.width * size) / 2.0,
                2 => w - line.width * size,
                _ => 0.0,
            };
            out.push_str(&format!("{} {} Td\n", num(x - previous), num(-step)));
            previous = x;
            if !line.bytes.is_empty() {
                self.show(out, &line.bytes, size);
            }
        }
        out.push_str("ET\n");
    }

    /// One character per cell of `max` cells across the whole width.
    fn comb(&self, out: &mut String, value: &str, w: f64, h: f64, max: i64) {
        let bytes = Self::encode(value);
        let advance = self.metrics.width(&bytes);
        let size = auto_size(self.da.size, advance, w, h);
        let ty = (h - size) / 2.0;
        self.begin(out);
        out.push_str(&format!("0 {} Td\n", num(h - BASELINE * size - ty)));
        if !bytes.is_empty() && size != 0.0 && max > 0 {
            let cell = w * 1000.0 / size / max as f64;
            out.push_str(&format!(
                "/{} {} Tf\n[",
                name_token(&self.da.font),
                num(size)
            ));
            let mut carry = 0.0;
            for &byte in &bytes {
                let pad = (cell - self.metrics.advance(byte)) / 2.0;
                out.push_str(&num(-(carry + pad)));
                out.push_str(&literal(&[byte]));
                carry = pad;
            }
            out.push_str("] TJ\n");
        }
        out.push_str("ET\n");
    }
}

/// The DA size, or for 0 the height, shrunk to fit `advance` (the text's
/// width at size 1) into the width.
fn auto_size(size: f64, advance: f64, w: f64, h: f64) -> f64 {
    if size != 0.0 {
        size
    } else if advance > 0.0 {
        h.min(w / advance)
    } else {
        h
    }
}

/// MuPDF's `draw_circle`: four beziers from the top, clockwise.
fn circle(out: &mut String, r: f64, cx: f64, cy: f64) {
    let m = r * CIRCLE;
    let p = |x: f64, y: f64| format!("{} {}", num(x), num(y));
    out.push_str(&format!("{} m\n", p(cx, cy + r)));
    out.push_str(&format!(
        "{} {} {} c\n",
        p(cx + m, cy + r),
        p(cx + r, cy + m),
        p(cx + r, cy)
    ));
    out.push_str(&format!(
        "{} {} {} c\n",
        p(cx + r, cy - m),
        p(cx + m, cy - r),
        p(cx, cy - r)
    ));
    out.push_str(&format!(
        "{} {} {} c\n",
        p(cx - m, cy - r),
        p(cx - r, cy - m),
        p(cx - r, cy)
    ));
    out.push_str(&format!(
        "{} {} {} c\n",
        p(cx - r, cy + m),
        p(cx - m, cy + r),
        p(cx, cy + r)
    ));
}
