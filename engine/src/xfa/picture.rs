// SPDX-License-Identifier: MIT
//! XFA picture clauses: how a field's canonical value (what the datasets
//! hold) is shown, and how what a user types is read back. Design:
//! `design/app-xfa.md` (Pictures).
//!
//! A picture is one or more alternatives separated by `|`. An alternative
//! is a sequence of category clauses — `date{...}`, `time{...}`,
//! `datetime{...}`, `num{...}`, `text{...}`, `zero{...}`, `null{...}` —
//! with literal text between them, or, without any category, one bare
//! body whose category the field's value type gives. A clause may name a
//! locale (`date(fr_FR){...}`) and may be a keyword (`date.long{}` or
//! `date{long}`, `num.currency{}`), which the locale's patterns resolve.
//! Inside a body, quoted text (`'...'`, `''` for a quote) is literal; a
//! run of one character is a symbol when the category knows it and
//! literal otherwise.
//!
//! Formatting tries the alternatives in order: `null{}` for an empty
//! value, `zero{}` for a numeric zero, then the others; the first that
//! formats wins. A value no alternative formats is shown as it is.
//! Reading back tries the same alternatives, then a lenient reading by
//! the value type. Bounded: `MAX_PICTURE_CHARS` and `MAX_ALTERNATIVES`;
//! anything malformed is `None` (the caller shows the value as it is),
//! never a panic.

use super::types::{Field, Form, Ui, Value, ValueKind};
use super::xml::{self, Element};

/// Characters of a picture read; a longer one is not used.
pub const MAX_PICTURE_CHARS: usize = 256;
/// Alternatives of a picture; one with more is not used.
pub const MAX_ALTERNATIVES: usize = 8;
/// Locales read from a `localeSet` packet.
const MAX_LOCALES: usize = 64;
/// Characters of a value or typed text formatted or read.
const MAX_VALUE_CHARS: usize = 512;

/// A picture clause's category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Date,
    Time,
    DateTime,
    Num,
    Text,
    Zero,
    Null,
}

impl Category {
    fn from_name(name: &str) -> Option<Category> {
        Some(match name {
            "date" => Category::Date,
            "time" => Category::Time,
            "datetime" => Category::DateTime,
            "num" => Category::Num,
            "text" => Category::Text,
            "zero" => Category::Zero,
            "null" => Category::Null,
            _ => return None,
        })
    }

    /// The category a bare picture has for a value of `kind`.
    fn of_kind(kind: ValueKind) -> Category {
        match kind {
            ValueKind::Date => Category::Date,
            ValueKind::Time => Category::Time,
            ValueKind::DateTime => Category::DateTime,
            ValueKind::Integer | ValueKind::Decimal | ValueKind::Float => Category::Num,
            _ => Category::Text,
        }
    }
}

/// One element of a clause body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Token {
    /// A run of `.1` copies of one unquoted character.
    Sym(char, usize),
    /// Quoted text.
    Lit(String),
    /// The unquoted `T` between a `datetime` body's date and time halves.
    Split,
}

/// A clause body: symbols and literals, or a locale pattern's name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Tokens(Vec<Token>),
    Keyword(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clause {
    /// `None` for a bare picture: the value type decides.
    pub category: Option<Category>,
    pub locale: Option<String>,
    pub body: Body,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Clause(Clause),
    Lit(String),
}

/// One `|` alternative.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alternative(pub Vec<Part>);

impl Alternative {
    /// The category of its only clause when it is `zero{}` or `null{}`.
    fn special(&self) -> Option<Category> {
        let mut clauses = self.0.iter().filter_map(|p| match p {
            Part::Clause(c) => Some(c.category),
            Part::Lit(_) => None,
        });
        match (clauses.next(), clauses.next()) {
            (Some(Some(c @ (Category::Zero | Category::Null))), None) => Some(c),
            _ => None,
        }
    }

    /// The text of a `zero{}` or `null{}` alternative: its literals.
    fn literal_text(&self) -> String {
        let mut out = String::new();
        for part in &self.0 {
            match part {
                Part::Lit(t) => out.push_str(t),
                Part::Clause(c) => {
                    if let Body::Tokens(tokens) = &c.body {
                        for t in tokens {
                            push_literal(t, &mut out);
                        }
                    }
                }
            }
        }
        out
    }
}

/// A parsed picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub alternatives: Vec<Alternative>,
}

const KEYWORDS: &[&str] = &[
    "short", "med", "medium", "long", "full", "default", "integer", "decimal", "numeric",
    "currency", "percent",
];

/// Reads a picture. `None` when it is empty, too long, has too many
/// alternatives, or is malformed (an unclosed quote or brace, an unknown
/// category).
pub fn parse(text: &str) -> Option<Pattern> {
    if text.chars().count() > MAX_PICTURE_CHARS {
        return None;
    }
    let chars: Vec<char> = text.trim().chars().collect();
    if chars.is_empty() {
        return None;
    }
    let mut alternatives = Vec::new();
    if !has_clause(&chars) {
        if unquoted(&chars).any(|c| c == '{' || c == '}') {
            return None;
        }
        for piece in split_unquoted(&chars, '|')? {
            if piece.iter().all(|c| c.is_whitespace()) {
                return None;
            }
            alternatives.push(Alternative(vec![Part::Clause(Clause {
                category: None,
                locale: None,
                body: body(&piece, None)?,
            })]));
        }
    } else {
        let mut current: Vec<Part> = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c == '|' {
                if current.is_empty() {
                    return None;
                }
                alternatives.push(Alternative(std::mem::take(&mut current)));
                i += 1;
            } else if c == '\'' {
                let (text, next) = quoted(&chars, i)?;
                push_lit(&mut current, &text);
                i = next;
            } else if let Some((category, keyword, locale, start)) = clause_head(&chars, i) {
                let end = close_brace(&chars, start)?;
                let pieces = split_unquoted(&chars[start..end], '|')?;
                let mut clauses = Vec::new();
                for piece in &pieces {
                    let body = if piece.iter().all(|c| c.is_whitespace()) && keyword.is_some() {
                        Body::Keyword(keyword.clone().unwrap_or_default())
                    } else {
                        body(piece, Some(category))?
                    };
                    clauses.push(Clause {
                        category: Some(category),
                        locale: locale.clone(),
                        body,
                    });
                }
                i = end + 1;
                if clauses.len() > 1 {
                    // `num{a|b}`: each piece its own alternative, so the
                    // clause must stand alone.
                    if !current.is_empty() || chars.get(i).is_some_and(|&c| c != '|') {
                        return None;
                    }
                    for clause in clauses {
                        alternatives.push(Alternative(vec![Part::Clause(clause)]));
                    }
                    if i < chars.len() {
                        i += 1;
                    }
                } else {
                    current.extend(clauses.into_iter().map(Part::Clause));
                }
            } else if c.is_ascii_alphabetic() {
                // A word that is not a clause where clauses are expected.
                return None;
            } else {
                push_lit(&mut current, &c.to_string());
                i += 1;
            }
            if alternatives.len() > MAX_ALTERNATIVES {
                return None;
            }
        }
        if !current.is_empty() {
            alternatives.push(Alternative(current));
        }
    }
    if alternatives.is_empty() || alternatives.len() > MAX_ALTERNATIVES {
        return None;
    }
    // An alternative of literals only says nothing.
    if alternatives
        .iter()
        .any(|a| !a.0.iter().any(|p| matches!(p, Part::Clause(_))))
    {
        return None;
    }
    Some(Pattern { alternatives })
}

fn push_lit(parts: &mut Vec<Part>, text: &str) {
    if let Some(Part::Lit(last)) = parts.last_mut() {
        last.push_str(text);
    } else {
        parts.push(Part::Lit(text.to_string()));
    }
}

/// The characters of `chars` outside quotes.
fn unquoted(chars: &[char]) -> impl Iterator<Item = char> + '_ {
    let mut quoted = false;
    chars.iter().filter_map(move |&c| {
        if c == '\'' {
            quoted = !quoted;
            return None;
        }
        (!quoted).then_some(c)
    })
}

/// Whether an unquoted category name followed by `{`, `(` or `.` occurs.
fn has_clause(chars: &[char]) -> bool {
    let mut i = 0;
    let mut quoted = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            quoted = !quoted;
            i += 1;
            continue;
        }
        if !quoted && c.is_ascii_alphabetic() && (i == 0 || !chars[i - 1].is_ascii_alphabetic()) {
            if clause_head(chars, i).is_some() {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// A clause head at `i`: `name[.keyword][(locale)]{`. Returns the
/// category, keyword, locale and the index after `{`.
fn clause_head(
    chars: &[char],
    i: usize,
) -> Option<(Category, Option<String>, Option<String>, usize)> {
    let word = |from: usize| {
        let mut end = from;
        while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
        end
    };
    let end = word(i);
    let name: String = chars[i..end].iter().collect();
    let category = Category::from_name(&name)?;
    let mut at = end;
    let mut keyword = None;
    if chars.get(at) == Some(&'.') {
        let kw_end = word(at + 1);
        let kw: String = chars[at + 1..kw_end].iter().collect();
        if !KEYWORDS.contains(&kw.as_str()) {
            return None;
        }
        keyword = Some(kw);
        at = kw_end;
    }
    let mut locale = None;
    if chars.get(at) == Some(&'(') {
        let close = chars[at..].iter().position(|&c| c == ')')? + at;
        let name: String = chars[at + 1..close].iter().collect();
        let name = name.trim().to_string();
        if name.is_empty() || name.len() > 32 {
            return None;
        }
        locale = Some(name);
        at = close + 1;
    }
    if chars.get(at) != Some(&'{') {
        return None;
    }
    Some((category, keyword, locale, at + 1))
}

/// The index of the unquoted `}` closing a body that starts at `start`.
fn close_brace(chars: &[char], start: usize) -> Option<usize> {
    let mut quoted = false;
    for (i, &c) in chars.iter().enumerate().skip(start) {
        match c {
            '\'' => quoted = !quoted,
            '}' if !quoted => return Some(i),
            '{' if !quoted => return None,
            _ => {}
        }
    }
    None
}

/// Quoted text from the quote at `i`: the text and the index after the
/// closing quote. `''` inside is one quote.
fn quoted(chars: &[char], i: usize) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut j = i + 1;
    if chars.get(j) == Some(&'\'') {
        return Some(("'".into(), j + 1));
    }
    while j < chars.len() {
        if chars[j] == '\'' {
            if chars.get(j + 1) == Some(&'\'') {
                out.push('\'');
                j += 2;
                continue;
            }
            return Some((out, j + 1));
        }
        out.push(chars[j]);
        j += 1;
    }
    None
}

/// `chars` split at unquoted `sep`. `None` on an unclosed quote.
fn split_unquoted(chars: &[char], sep: char) -> Option<Vec<Vec<char>>> {
    let mut out = vec![Vec::new()];
    let mut quoted = false;
    for &c in chars {
        if c == '\'' {
            quoted = !quoted;
        }
        if c == sep && !quoted {
            out.push(Vec::new());
        } else if let Some(last) = out.last_mut() {
            last.push(c);
        }
    }
    (!quoted).then_some(out)
}

/// A body's tokens, or its keyword when it is only one.
fn body(chars: &[char], category: Option<Category>) -> Option<Body> {
    let word: String = chars.iter().collect();
    let word = word.trim();
    if category.is_some() && KEYWORDS.contains(&word) {
        return Some(Body::Keyword(word.to_string()));
    }
    tokens(chars, category == Some(Category::DateTime)).map(Body::Tokens)
}

fn tokens(chars: &[char], datetime: bool) -> Option<Vec<Token>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' {
            let (text, next) = quoted(chars, i)?;
            if let Some(Token::Lit(last)) = out.last_mut() {
                last.push_str(&text);
            } else {
                out.push(Token::Lit(text));
            }
            i = next;
            continue;
        }
        let mut n = 1;
        while chars.get(i + n) == Some(&c) {
            n += 1;
        }
        if datetime && c == 'T' && n == 1 {
            if out.contains(&Token::Split) {
                return None;
            }
            out.push(Token::Split);
        } else {
            out.push(Token::Sym(c, n));
        }
        i += n;
    }
    Some(out)
}

fn push_literal(token: &Token, out: &mut String) {
    match token {
        Token::Sym(c, n) => out.extend(std::iter::repeat_n(*c, *n)),
        Token::Lit(t) => out.push_str(t),
        Token::Split => {}
    }
}

// ------------------------------------------------------------------ locale

/// What pictures need of a locale (`localeSet/locale`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Locale {
    pub name: String,
    pub months: [String; 12],
    pub months_abbr: [String; 12],
    /// Sunday first.
    pub days: [String; 7],
    pub days_abbr: [String; 7],
    pub meridiem: [String; 2],
    pub eras: [String; 2],
    /// `full`, `long`, `med`, `short`.
    pub date_patterns: [String; 4],
    pub time_patterns: [String; 4],
    /// `numeric`, `currency`, `percent`.
    pub num_patterns: [String; 3],
    pub decimal: String,
    pub grouping: String,
    pub percent: String,
    pub minus: String,
    pub currency: String,
}

fn strings<const N: usize>(items: [&str; N]) -> [String; N] {
    items.map(str::to_string)
}

impl Locale {
    /// The built-in `en_US`, used when the packet or a locale is missing.
    pub fn en_us() -> Locale {
        Locale {
            name: "en_US".into(),
            months: strings([
                "January",
                "February",
                "March",
                "April",
                "May",
                "June",
                "July",
                "August",
                "September",
                "October",
                "November",
                "December",
            ]),
            months_abbr: strings([
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ]),
            days: strings([
                "Sunday",
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
            ]),
            days_abbr: strings(["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]),
            meridiem: strings(["AM", "PM"]),
            eras: strings(["BC", "AD"]),
            date_patterns: strings([
                "EEEE, MMMM D, YYYY",
                "MMMM D, YYYY",
                "MMM D, YYYY",
                "M/D/YY",
            ]),
            time_patterns: strings(["h:MM:SS A Z", "h:MM:SS A Z", "h:MM:SS A", "h:MM A"]),
            num_patterns: strings(["z,zz9.zzz", "$z,zz9.99|($z,zz9.99)", "z,zz9%"]),
            decimal: ".".into(),
            grouping: ",".into(),
            percent: "%".into(),
            minus: "-".into(),
            currency: "$".into(),
        }
    }

    /// A `locale` element over `base` (what it leaves out stays `base`'s).
    fn read(e: &Element, base: &Locale) -> Locale {
        let mut out = base.clone();
        out.name = e.attr("name").unwrap_or("").to_string();
        let calendar = e.child("calendarSymbols");
        let names = |list: &str, item: &str, abbr: bool| -> Vec<String> {
            calendar
                .into_iter()
                .flat_map(|c| c.children_named(list))
                .find(|l| (l.attr("abbr") == Some("1")) == abbr)
                .map(|l| {
                    l.children_named(item)
                        .map(|m| m.text().trim().to_string())
                        .collect()
                })
                .unwrap_or_default()
        };
        fill(&mut out.months, names("monthNames", "month", false));
        fill(&mut out.months_abbr, names("monthNames", "month", true));
        fill(&mut out.days, names("dayNames", "day", false));
        fill(&mut out.days_abbr, names("dayNames", "day", true));
        fill(&mut out.meridiem, names("meridiemNames", "meridiem", false));
        fill(&mut out.eras, names("eraNames", "era", false));
        let named = |list: &str, item: &str, name: &str| -> Option<String> {
            e.child(list)?
                .children_named(item)
                .find(|p| p.attr("name") == Some(name))
                .map(|p| p.text())
        };
        for (slot, name) in out
            .date_patterns
            .iter_mut()
            .zip(["full", "long", "med", "short"])
        {
            if let Some(p) = named("datePatterns", "datePattern", name).filter(|p| !p.is_empty()) {
                *slot = p;
            }
        }
        for (slot, name) in out
            .time_patterns
            .iter_mut()
            .zip(["full", "long", "med", "short"])
        {
            if let Some(p) = named("timePatterns", "timePattern", name).filter(|p| !p.is_empty()) {
                *slot = p;
            }
        }
        for (slot, name) in out
            .num_patterns
            .iter_mut()
            .zip(["numeric", "currency", "percent"])
        {
            if let Some(p) =
                named("numberPatterns", "numberPattern", name).filter(|p| !p.is_empty())
            {
                *slot = p;
            }
        }
        let symbol =
            |name: &str| named("numberSymbols", "numberSymbol", name).filter(|s| !s.is_empty());
        if let Some(s) = symbol("decimal") {
            out.decimal = s;
        }
        if let Some(s) = symbol("grouping") {
            out.grouping = s;
        }
        if let Some(s) = symbol("percent") {
            out.percent = s;
        }
        if let Some(s) = symbol("minus") {
            out.minus = s;
        }
        if let Some(s) =
            named("currencySymbols", "currencySymbol", "symbol").filter(|s| !s.is_empty())
        {
            out.currency = s;
        }
        out
    }

    fn date_pattern(&self, keyword: &str) -> &str {
        match keyword {
            "full" => &self.date_patterns[0],
            "long" => &self.date_patterns[1],
            "short" => &self.date_patterns[3],
            _ => &self.date_patterns[2],
        }
    }

    fn time_pattern(&self, keyword: &str) -> &str {
        match keyword {
            "full" => &self.time_patterns[0],
            "long" => &self.time_patterns[1],
            "short" => &self.time_patterns[3],
            _ => &self.time_patterns[2],
        }
    }
}

/// `slot` overwritten by `items` when there are exactly as many.
fn fill<const N: usize>(slot: &mut [String; N], items: Vec<String>) {
    if items.len() == N && items.iter().all(|s| !s.is_empty()) {
        for (s, item) in slot.iter_mut().zip(items) {
            *s = item;
        }
    }
}

/// The locales of a `localeSet` packet.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LocaleSet {
    pub locales: Vec<Locale>,
}

static EN_US: std::sync::OnceLock<Locale> = std::sync::OnceLock::new();

impl LocaleSet {
    /// The packet's locales (none when it is missing or not XML).
    pub fn read(packet: Option<&str>) -> LocaleSet {
        let Some(root) = packet.and_then(|text| xml::parse(text).ok()) else {
            return LocaleSet::default();
        };
        let base = Locale::en_us();
        let set = if root.local() == "localeSet" {
            Some(&root)
        } else {
            root.child("localeSet")
        };
        let locales = set
            .into_iter()
            .flat_map(|s| s.children_named("locale"))
            .take(MAX_LOCALES)
            .map(|l| Locale::read(l, &base))
            .filter(|l| !l.name.is_empty())
            .collect();
        LocaleSet { locales }
    }

    /// The locale named `name`; else one of the same language; else the
    /// built-in `en_US`.
    pub fn get(&self, name: &str) -> &Locale {
        let name = name.replace('-', "_");
        if let Some(l) = self.locales.iter().find(|l| l.name == name) {
            return l;
        }
        let language = name.split('_').next().unwrap_or("");
        if !language.is_empty() && language != "en" {
            if let Some(l) =
                (self.locales.iter()).find(|l| l.name.split('_').next() == Some(language))
            {
                return l;
            }
        }
        if let Some(l) = self.locales.iter().find(|l| l.name == "en_US") {
            return l;
        }
        EN_US.get_or_init(Locale::en_us)
    }
}

// ------------------------------------------------------------ field level

/// What `f` shows: its value through its format picture, when it has one
/// that formats the value; the value as it is otherwise. Check buttons,
/// choice lists and rich text are shown as they are.
pub fn shown(form: &Form, f: &Field) -> String {
    let plain = f.value.plain();
    if !matches!(f.value, Value::Text(_) | Value::Empty) || !formats(&f.ui) {
        return plain;
    }
    let Some(pattern) = f.picture.as_ref().and_then(|p| parse(&p.0)) else {
        return plain;
    };
    let locale = field_locale(form, f);
    format(&pattern, f.value_kind, &plain, &form.locales, locale).unwrap_or(plain)
}

/// Whether `f`'s value passes the `validate` picture `picture` (XFA's
/// `formatTest`): the picture formats it. An empty value, a picture this
/// reader does not parse, or a field that is not formatted (a check
/// button, a choice list, rich text) passes.
pub fn validates(form: &Form, f: &Field, picture: &str) -> bool {
    let plain = f.value.plain();
    if plain.trim().is_empty() || !matches!(f.value, Value::Text(_)) || !formats(&f.ui) {
        return true;
    }
    let Some(pattern) = parse(picture) else {
        return true;
    };
    let locale = field_locale(form, f);
    format(&pattern, f.value_kind, &plain, &form.locales, locale).is_some()
}

fn formats(ui: &Ui) -> bool {
    matches!(
        ui,
        Ui::TextEdit { .. } | Ui::NumericEdit | Ui::DateTimeEdit | Ui::Unknown
    )
}

fn field_locale<'f>(form: &'f Form, f: &'f Field) -> &'f str {
    if f.locale.is_empty() {
        &form.locale
    } else {
        &f.locale
    }
}

/// The canonical value of `typed` in `f`: read with its edit picture,
/// else its format picture, else leniently by its value type. `None`
/// when nothing reads it (the caller keeps the text as typed).
pub fn canonical(form: &Form, f: &Field, typed: &str) -> Option<String> {
    if !formats(&f.ui) || matches!(f.value, Value::Html(_)) {
        return None;
    }
    let locale = field_locale(form, f);
    let locales = &form.locales;
    for picture in [&f.edit_picture, &f.picture].into_iter().flatten() {
        if let Some(pattern) = parse(&picture.0)
            && let Some(value) = parse_value(&pattern, f.value_kind, typed, locales, locale)
        {
            return Some(value);
        }
    }
    lenient(f.value_kind, typed, locales.get(locale))
}

// ------------------------------------------------------------- formatting

/// `value` (canonical) through `pattern` in the locale `locale` of
/// `locales`, for a value of `kind`. `None` when no alternative formats
/// it.
pub fn format(
    pattern: &Pattern,
    kind: ValueKind,
    value: &str,
    locales: &LocaleSet,
    locale: &str,
) -> Option<String> {
    if value.chars().count() > MAX_VALUE_CHARS {
        return None;
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return pattern
            .alternatives
            .iter()
            .find(|a| a.special() == Some(Category::Null))
            .map(Alternative::literal_text);
    }
    if Decimal::parse(trimmed).is_some_and(|d| d.is_zero())
        && let Some(zero) =
            (pattern.alternatives.iter()).find(|a| a.special() == Some(Category::Zero))
    {
        return Some(zero.literal_text());
    }
    pattern
        .alternatives
        .iter()
        .filter(|a| a.special().is_none())
        .find_map(|a| format_alternative(a, kind, value, locales, locale))
}

fn format_alternative(
    alt: &Alternative,
    kind: ValueKind,
    value: &str,
    locales: &LocaleSet,
    locale: &str,
) -> Option<String> {
    let mut out = String::new();
    for part in &alt.0 {
        match part {
            Part::Lit(t) => out.push_str(t),
            Part::Clause(clause) => {
                let category = clause.category.unwrap_or(Category::of_kind(kind));
                let loc = locales.get(clause.locale.as_deref().unwrap_or(locale));
                let bodies = resolve(clause, category, loc)?;
                let text = bodies
                    .iter()
                    .find_map(|tokens| format_clause(category, tokens, kind, value, loc))?;
                out.push_str(&text);
            }
        }
    }
    Some(out)
}

/// A clause's token lists: its own, or its keyword's locale pattern (with
/// that pattern's alternatives).
fn resolve(clause: &Clause, category: Category, loc: &Locale) -> Option<Vec<Vec<Token>>> {
    let keyword = match &clause.body {
        Body::Tokens(t) => return Some(vec![t.clone()]),
        Body::Keyword(k) => k.as_str(),
    };
    let pattern = match (category, keyword) {
        (Category::Date, k) if !is_num_keyword(k) => loc.date_pattern(k).to_string(),
        (Category::Time, k) if !is_num_keyword(k) => loc.time_pattern(k).to_string(),
        (Category::DateTime, k) if !is_num_keyword(k) => {
            // A quoted T would be literal; the split is unquoted.
            format!("{} T{}", loc.date_pattern(k), loc.time_pattern(k))
        }
        (Category::Num, "integer") => "z,zzz,zzz,zzz,zz9".to_string(),
        (Category::Num, "decimal" | "numeric" | "default") => loc.num_patterns[0].clone(),
        (Category::Num, "currency") => loc.num_patterns[1].clone(),
        (Category::Num, "percent") => loc.num_patterns[2].clone(),
        _ => return None,
    };
    let chars: Vec<char> = pattern.chars().take(MAX_PICTURE_CHARS).collect();
    let mut out = Vec::new();
    for piece in split_unquoted(&chars, '|')?.iter().take(MAX_ALTERNATIVES) {
        out.push(tokens(piece, category == Category::DateTime)?);
    }
    Some(out)
}

fn is_num_keyword(k: &str) -> bool {
    matches!(
        k,
        "integer" | "decimal" | "numeric" | "currency" | "percent"
    )
}

fn format_clause(
    category: Category,
    tokens: &[Token],
    kind: ValueKind,
    value: &str,
    loc: &Locale,
) -> Option<String> {
    let value = value.trim();
    match category {
        Category::Date => {
            let date_part = match value.split_once('T') {
                Some((d, _)) => d,
                None if kind == ValueKind::DateTime => value.split(' ').next().unwrap_or(value),
                None => value,
            };
            format_date(tokens, Date::canonical(date_part)?, loc)
        }
        Category::Time => {
            let time_part = match value.split_once('T') {
                Some((_, t)) => t,
                None => value,
            };
            format_time(tokens, Time::canonical(time_part)?, loc)
        }
        Category::DateTime => {
            let (d, t) = value.split_once('T')?;
            let (date, time) = (Date::canonical(d)?, Time::canonical(t)?);
            let split = tokens.iter().position(|t| *t == Token::Split);
            let (dt, tt) = match split {
                Some(at) => (&tokens[..at], &tokens[at + 1..]),
                None => (tokens, &[][..]),
            };
            let mut out = format_date(dt, date, loc)?;
            out.push_str(&format_time(tt, time, loc)?);
            Some(out)
        }
        Category::Num => format_number(tokens, &Decimal::parse(value)?, loc),
        Category::Text => format_text(tokens, value),
        Category::Zero | Category::Null => None,
    }
}

// ------------------------------------------------------------------ dates

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Date {
    y: i32,
    m: u32,
    d: u32,
}

fn leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn month_days(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

fn digits_of(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 9 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

impl Date {
    fn new(y: i32, m: u32, d: u32) -> Option<Date> {
        ((1..=9999).contains(&y) && (1..=12).contains(&m) && d >= 1 && d <= month_days(y, m))
            .then_some(Date { y, m, d })
    }

    /// `YYYY-MM-DD` or `YYYYMMDD`.
    fn canonical(s: &str) -> Option<Date> {
        let s = s.trim();
        let (y, m, d) = if s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' {
            (s.get(..4)?, s.get(5..7)?, s.get(8..)?)
        } else if s.len() == 8 {
            (s.get(..4)?, s.get(4..6)?, s.get(6..)?)
        } else {
            return None;
        };
        Date::new(digits_of(y)? as i32, digits_of(m)?, digits_of(d)?)
    }

    fn to_canonical(self) -> String {
        format!("{:04}-{:02}-{:02}", self.y, self.m, self.d)
    }

    /// Days since 1970-01-01.
    fn days(self) -> i64 {
        let y = i64::from(self.y) - i64::from(self.m <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.m);
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(self.d) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// 0 for Sunday.
    fn weekday(self) -> u32 {
        (self.days() + 4).rem_euclid(7) as u32
    }

    fn day_of_year(self) -> u32 {
        (1..self.m).map(|m| month_days(self.y, m)).sum::<u32>() + self.d
    }

    fn from_day_of_year(y: i32, doy: u32) -> Option<Date> {
        let mut left = doy;
        for m in 1..=12 {
            let n = month_days(y, m);
            if left >= 1 && left <= n {
                return Date::new(y, m, left);
            }
            left = left.checked_sub(n)?;
        }
        None
    }

    /// The ISO 8601 week of the year.
    fn iso_week(self) -> u32 {
        let iso_weekday = (self.weekday() + 6) % 7 + 1;
        let week = (self.day_of_year() as i32 - iso_weekday as i32 + 10) / 7;
        let weeks_in = |y: i32| {
            let jan1 = Date { y, m: 1, d: 1 }.weekday();
            if jan1 == 4 || (jan1 == 3 && leap(y)) {
                53i32
            } else {
                52
            }
        };
        if week < 1 {
            weeks_in(self.y - 1) as u32
        } else if week > weeks_in(self.y) {
            1
        } else {
            week as u32
        }
    }

    /// The week of the month, weeks starting on Sunday, the first holding
    /// the 1st.
    fn week_of_month(self) -> u32 {
        let first = Date { d: 1, ..self }.weekday();
        (self.d + first - 1) / 7 + 1
    }
}

/// A symbol run's length is one of `allowed`.
fn sized(n: usize, allowed: &[usize]) -> Option<usize> {
    allowed.contains(&n).then_some(n)
}

fn format_date(tokens: &[Token], date: Date, loc: &Locale) -> Option<String> {
    let mut out = String::new();
    for token in tokens {
        let Token::Sym(c, n) = *token else {
            push_literal(token, &mut out);
            continue;
        };
        let m = date.m as usize - 1;
        match c {
            'D' => match sized(n, &[1, 2])? {
                1 => out.push_str(&date.d.to_string()),
                _ => out.push_str(&format!("{:02}", date.d)),
            },
            'J' => match sized(n, &[1, 3])? {
                1 => out.push_str(&date.day_of_year().to_string()),
                _ => out.push_str(&format!("{:03}", date.day_of_year())),
            },
            'M' => match sized(n, &[1, 2, 3, 4])? {
                1 => out.push_str(&date.m.to_string()),
                2 => out.push_str(&format!("{:02}", date.m)),
                3 => out.push_str(&loc.months_abbr[m]),
                _ => out.push_str(&loc.months[m]),
            },
            'E' => match sized(n, &[1, 3, 4])? {
                1 => out.push_str(&(date.weekday() + 1).to_string()),
                3 => out.push_str(&loc.days_abbr[date.weekday() as usize]),
                _ => out.push_str(&loc.days[date.weekday() as usize]),
            },
            'e' => {
                sized(n, &[1])?;
                out.push_str(&((date.weekday() + 6) % 7 + 1).to_string());
            }
            'G' => {
                sized(n, &[1])?;
                out.push_str(&loc.eras[1]);
            }
            'Y' => match sized(n, &[2, 4])? {
                2 => out.push_str(&format!("{:02}", date.y % 100)),
                _ => out.push_str(&format!("{:04}", date.y)),
            },
            'w' => {
                sized(n, &[1])?;
                out.push_str(&date.week_of_month().to_string());
            }
            'W' => {
                sized(n, &[2])?;
                out.push_str(&format!("{:02}", date.iso_week()));
            }
            _ => push_literal(token, &mut out),
        }
    }
    Some(out)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct Time {
    h: u32,
    m: u32,
    s: u32,
    ms: u32,
    /// Minutes east of UTC, when the value says.
    zone: Option<i32>,
}

impl Time {
    /// `HH[:MM[:SS[.FFF]]]` or `HHMM[SS]`, then `Z` or `±HH[:MM]`.
    fn canonical(s: &str) -> Option<Time> {
        let s = s.trim();
        let (clock, zone) = match s.find(['Z', 'z', '+', '-']) {
            Some(at) => (&s[..at], Some(zone_of(&s[at..])?)),
            None => (s, None),
        };
        let (clock, ms) = match clock.split_once(['.', ',']) {
            Some((c, f)) => {
                let f = f.get(..f.len().min(3))?;
                let scale = 10u32.pow(3 - f.len() as u32);
                (c, digits_of(f)? * scale)
            }
            None => (clock, 0),
        };
        let parts: Vec<&str> = if clock.contains(':') {
            clock.split(':').collect()
        } else {
            (0..clock.len().div_ceil(2))
                .map(|i| clock.get(i * 2..(i * 2 + 2).min(clock.len())).unwrap_or(""))
                .collect()
        };
        if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.len() != 2) {
            return None;
        }
        let at = |i: usize| parts.get(i).map_or(Some(0), |p| digits_of(p));
        let (h, m, sec) = (at(0)?, at(1)?, at(2)?);
        (h < 24 && m < 60 && sec < 60).then_some(Time {
            h,
            m,
            s: sec,
            ms,
            zone,
        })
    }

    fn to_canonical(self) -> String {
        let mut out = format!("{:02}:{:02}:{:02}", self.h, self.m, self.s);
        if self.ms != 0 {
            out.push_str(&format!(".{:03}", self.ms));
        }
        if let Some(z) = self.zone {
            out.push_str(&zone_text(z, true));
        }
        out
    }
}

/// `Z`, `±HH`, `±HHMM` or `±HH:MM` as minutes east of UTC.
fn zone_of(s: &str) -> Option<i32> {
    if s.eq_ignore_ascii_case("z") {
        return Some(0);
    }
    let sign = match s.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let rest: String = s[1..].chars().filter(|&c| c != ':').collect();
    let (h, m) = match rest.len() {
        2 => (digits_of(&rest)?, 0),
        4 => (digits_of(&rest[..2])?, digits_of(&rest[2..])?),
        _ => return None,
    };
    (h < 24 && m < 60).then_some(sign * (h * 60 + m) as i32)
}

fn zone_text(z: i32, colon: bool) -> String {
    if z == 0 {
        return "Z".into();
    }
    let sign = if z < 0 { '-' } else { '+' };
    let (h, m) = (z.abs() / 60, z.abs() % 60);
    if colon {
        format!("{sign}{h:02}:{m:02}")
    } else {
        format!("{sign}{h:02}{m:02}")
    }
}

fn format_time(tokens: &[Token], t: Time, loc: &Locale) -> Option<String> {
    let mut out = String::new();
    let pad = |v: u32, n: usize| {
        if n == 2 {
            format!("{v:02}")
        } else {
            v.to_string()
        }
    };
    for token in tokens {
        let Token::Sym(c, n) = *token else {
            push_literal(token, &mut out);
            continue;
        };
        match c {
            'h' => out.push_str(&pad(
                if t.h % 12 == 0 { 12 } else { t.h % 12 },
                sized(n, &[1, 2])?,
            )),
            'k' => out.push_str(&pad(t.h % 12, sized(n, &[1, 2])?)),
            'H' => out.push_str(&pad(t.h, sized(n, &[1, 2])?)),
            'K' => out.push_str(&pad(if t.h == 0 { 24 } else { t.h }, sized(n, &[1, 2])?)),
            'M' => out.push_str(&pad(t.m, sized(n, &[1, 2])?)),
            'S' => out.push_str(&pad(t.s, sized(n, &[1, 2])?)),
            'F' => {
                sized(n, &[3])?;
                out.push_str(&format!("{:03}", t.ms));
            }
            'A' => {
                sized(n, &[1])?;
                out.push_str(&loc.meridiem[usize::from(t.h >= 12)]);
            }
            'Z' | 'z' => {
                sized(n, &[1, 2])?;
                if let Some(z) = t.zone {
                    out.push_str(&zone_text(z, c == 'z' || n == 2));
                }
            }
            _ => push_literal(token, &mut out),
        }
    }
    Some(out)
}

// ---------------------------------------------------------------- numbers

/// A decimal number as digits: no leading zeros in `int`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Decimal {
    neg: bool,
    int: Vec<u8>,
    frac: Vec<u8>,
}

impl Decimal {
    /// A canonical number: `[-+]digits[.digits]`, or an exponent form.
    fn parse(s: &str) -> Option<Decimal> {
        let s = s.trim();
        if s.is_empty() || s.len() > MAX_VALUE_CHARS {
            return None;
        }
        if s.contains(['e', 'E']) {
            let f: f64 = s.parse().ok()?;
            if !f.is_finite() || f.abs() > 1e300 {
                return None;
            }
            return Decimal::parse(&format!("{f}"));
        }
        let (neg, body) = match s.as_bytes()[0] {
            b'-' => (true, &s[1..]),
            b'+' => (false, &s[1..]),
            _ => (false, s),
        };
        let (int, frac) = body.split_once('.').unwrap_or((body, ""));
        if int.is_empty() && frac.is_empty() {
            return None;
        }
        if !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
            return None;
        }
        let int: Vec<u8> = int
            .bytes()
            .map(|b| b - b'0')
            .skip_while(|&d| d == 0)
            .collect();
        Some(Decimal {
            neg,
            int,
            frac: frac.bytes().map(|b| b - b'0').collect(),
        })
    }

    fn is_zero(&self) -> bool {
        self.int.iter().chain(&self.frac).all(|&d| d == 0)
    }

    /// Times 10^k.
    fn shift(&mut self, k: i32) {
        if k > 0 {
            for _ in 0..k {
                let d = if self.frac.is_empty() {
                    0
                } else {
                    self.frac.remove(0)
                };
                self.int.push(d);
            }
        } else {
            for _ in 0..(-k) {
                let d = self.int.pop().unwrap_or(0);
                self.frac.insert(0, d);
            }
        }
        while self.int.first() == Some(&0) {
            self.int.remove(0);
        }
    }

    /// Rounded half away from zero to `places` fraction digits.
    fn round(&mut self, places: usize) {
        if self.frac.len() <= places {
            self.frac.resize(places, 0);
            return;
        }
        let up = self.frac[places] >= 5;
        self.frac.truncate(places);
        if !up {
            return;
        }
        let mut carry = true;
        for d in self.frac.iter_mut().rev().chain(self.int.iter_mut().rev()) {
            if *d == 9 {
                *d = 0;
            } else {
                *d += 1;
                carry = false;
                break;
            }
        }
        if carry {
            self.int.insert(0, 1);
        }
    }

    fn to_canonical(&self) -> String {
        let mut out = String::new();
        if self.neg && !self.is_zero() {
            out.push('-');
        }
        if self.int.is_empty() {
            out.push('0');
        }
        out.extend(self.int.iter().map(|d| char::from(b'0' + d)));
        if !self.frac.is_empty() {
            out.push('.');
            out.extend(self.frac.iter().map(|d| char::from(b'0' + d)));
        }
        out
    }
}

/// A number picture's elements.
#[derive(Clone, Debug, PartialEq, Eq)]
enum NumItem {
    /// `9`, `Z`, `z` or `8`.
    Digit(char),
    /// `.` and `V` (shown) or `v` (implied).
    Radix(bool),
    Group,
    Currency,
    Percent,
    /// `s`, `S`, `-` or `+`.
    Sign(char),
    LParen,
    RParen,
    /// `CR`, `cr`, `DB` or `db`.
    Credit(&'static str),
    Exp,
    Lit(String),
}

fn num_items(tokens: &[Token]) -> Option<Vec<NumItem>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        i += 1;
        let Token::Sym(c, n) = *token else {
            let mut text = String::new();
            push_literal(token, &mut text);
            out.push(NumItem::Lit(text));
            continue;
        };
        let pair = |second: char| {
            n == 1 && matches!(tokens.get(i), Some(Token::Sym(s, 1)) if *s == second)
        };
        let item = match c {
            '9' | 'Z' | 'z' | '8' => {
                out.extend(std::iter::repeat_n(NumItem::Digit(c), n));
                continue;
            }
            '.' | 'V' if n == 1 => NumItem::Radix(true),
            'v' if n == 1 => NumItem::Radix(false),
            ',' => {
                out.extend(std::iter::repeat_n(NumItem::Group, n));
                continue;
            }
            '$' => NumItem::Currency,
            '%' => NumItem::Percent,
            's' | 'S' | '-' | '+' => {
                out.extend(std::iter::repeat_n(NumItem::Sign(c), n));
                continue;
            }
            '(' if n == 1 => NumItem::LParen,
            ')' if n == 1 => NumItem::RParen,
            'E' if n == 1 => NumItem::Exp,
            'C' if pair('R') => {
                i += 1;
                NumItem::Credit("CR")
            }
            'c' if pair('r') => {
                i += 1;
                NumItem::Credit("cr")
            }
            'D' if pair('B') => {
                i += 1;
                NumItem::Credit("DB")
            }
            'd' if pair('b') => {
                i += 1;
                NumItem::Credit("db")
            }
            '.' | 'V' | 'v' | '(' | ')' | 'E' => return None,
            _ => NumItem::Lit(std::iter::repeat_n(c, n).collect()),
        };
        out.push(item);
    }
    if out
        .iter()
        .filter(|i| matches!(i, NumItem::Radix(_)))
        .count()
        > 1
        || out.iter().filter(|i| **i == NumItem::Exp).count() > 1
    {
        return None;
    }
    Some(out)
}

fn format_number(tokens: &[Token], value: &Decimal, loc: &Locale) -> Option<String> {
    let items = num_items(tokens)?;
    let radix_at = items.iter().position(|i| matches!(i, NumItem::Radix(_)));
    let is_digit = |i: &NumItem| matches!(i, NumItem::Digit(_));
    let int_positions = items[..radix_at.unwrap_or(items.len())]
        .iter()
        .filter(|i| is_digit(i))
        .count();
    let frac_positions = match radix_at {
        Some(at) => items[at..].iter().filter(|i| is_digit(i)).count(),
        None => 0,
    };
    let signed = items.iter().any(|i| {
        matches!(
            i,
            NumItem::Sign(_) | NumItem::LParen | NumItem::RParen | NumItem::Credit(_)
        )
    });
    let mut d = value.clone();
    if items.contains(&NumItem::Percent) {
        d.shift(2);
    }
    let mut exponent = 0i32;
    if items.contains(&NumItem::Exp) && !d.is_zero() {
        let wanted = int_positions.max(1) as i32;
        let magnitude = if d.int.is_empty() {
            -(d.frac.iter().take_while(|&&x| x == 0).count() as i32)
        } else {
            d.int.len() as i32
        };
        exponent = magnitude - wanted;
        d.shift(-exponent);
    }
    d.round(frac_positions);
    let neg = d.neg && !d.is_zero();
    if (neg && !signed) || d.int.len() > int_positions {
        return None;
    }
    let mut out = String::new();
    let lead = int_positions - d.int.len();
    let (mut int_index, mut frac_index) = (0usize, 0usize);
    let mut in_frac = false;
    let mut significant = false;
    let mut last_space = false;
    for item in &items {
        match item {
            NumItem::Digit(k) if !in_frac => {
                let at = int_index;
                int_index += 1;
                last_space = false;
                if at >= lead {
                    out.push(char::from(b'0' + d.int[at - lead]));
                    significant = true;
                } else {
                    match k {
                        '9' => {
                            out.push('0');
                            significant = true;
                        }
                        'Z' => {
                            out.push(' ');
                            last_space = true;
                        }
                        _ => {}
                    }
                }
            }
            NumItem::Digit(k) => {
                let at = frac_index;
                frac_index += 1;
                let digit = d.frac.get(at).copied().unwrap_or(0);
                let trailing = d.frac[at.min(d.frac.len())..].iter().all(|&x| x == 0);
                match (k, trailing) {
                    ('9', _) | (_, false) => out.push(char::from(b'0' + digit)),
                    ('Z', true) => out.push(' '),
                    _ => {}
                }
            }
            NumItem::Group => {
                if significant {
                    out.push_str(&loc.grouping);
                } else if last_space {
                    out.push(' ');
                }
            }
            NumItem::Radix(shown) => {
                in_frac = true;
                if *shown {
                    out.push_str(&loc.decimal);
                }
            }
            NumItem::Currency => out.push_str(&loc.currency),
            NumItem::Percent => out.push_str(&loc.percent),
            NumItem::Sign(c) => match (c, neg) {
                (_, true) => out.push_str(&loc.minus),
                ('S', false) => out.push(' '),
                ('+', false) => out.push('+'),
                _ => {}
            },
            NumItem::LParen => out.push(if neg { '(' } else { ' ' }),
            NumItem::RParen => out.push(if neg { ')' } else { ' ' }),
            NumItem::Credit(text) => {
                if neg {
                    out.push_str(text);
                } else {
                    out.push_str("  ");
                }
            }
            NumItem::Exp => {
                out.push('E');
                out.push(if exponent < 0 { '-' } else { '+' });
                out.push_str(&exponent.abs().to_string());
            }
            NumItem::Lit(t) => out.push_str(t),
        }
    }
    Some(out)
}

// ------------------------------------------------------------------- text

fn text_symbol(c: char) -> Option<fn(char) -> bool> {
    Some(match c {
        'X' => |_| true,
        'A' => char::is_alphabetic,
        '9' => |c: char| c.is_ascii_digit(),
        'O' | '0' => char::is_alphanumeric,
        _ => return None,
    })
}

fn format_text(tokens: &[Token], value: &str) -> Option<String> {
    let mut chars = value.chars();
    let mut out = String::new();
    for token in tokens {
        match *token {
            Token::Sym(c, n) => match text_symbol(c) {
                Some(accepts) => {
                    for _ in 0..n {
                        let next = chars.next().filter(|&ch| accepts(ch))?;
                        out.push(next);
                    }
                }
                None => push_literal(token, &mut out),
            },
            _ => push_literal(token, &mut out),
        }
    }
    chars.next().is_none().then_some(out)
}

// --------------------------------------------------------------- reading

/// What `text` (as shown or typed) is as a canonical value, read with
/// `pattern`'s alternatives for a value of `kind`. `None` when none
/// reads it.
pub fn parse_value(
    pattern: &Pattern,
    kind: ValueKind,
    text: &str,
    locales: &LocaleSet,
    locale: &str,
) -> Option<String> {
    if text.chars().count() > MAX_VALUE_CHARS {
        return None;
    }
    let trimmed = text.trim();
    for alt in &pattern.alternatives {
        match alt.special() {
            Some(Category::Null) if alt.literal_text().trim() == trimmed => {
                return Some(String::new());
            }
            Some(Category::Zero) if alt.literal_text().trim() == trimmed => {
                return Some("0".into());
            }
            _ => {}
        }
    }
    if trimmed.is_empty() {
        return Some(String::new());
    }
    pattern
        .alternatives
        .iter()
        .filter(|a| a.special().is_none())
        .find_map(|a| parse_alternative(a, kind, trimmed, locales, locale))
}

fn parse_alternative(
    alt: &Alternative,
    kind: ValueKind,
    text: &str,
    locales: &LocaleSet,
    locale: &str,
) -> Option<String> {
    // One clause with literals around it is the common shape; a compound
    // date and time alternative is read clause by clause.
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0usize;
    let mut date = None;
    let mut time = None;
    let mut other = None;
    let clauses = alt
        .0
        .iter()
        .filter(|p| matches!(p, Part::Clause(_)))
        .count();
    for (index, part) in alt.0.iter().enumerate() {
        match part {
            Part::Lit(t) => at = match_literal(&chars, at, t)?,
            Part::Clause(clause) => {
                let category = clause.category.unwrap_or(Category::of_kind(kind));
                let loc = locales.get(clause.locale.as_deref().unwrap_or(locale));
                let bodies = resolve(clause, category, loc)?;
                let last = !alt.0[index + 1..]
                    .iter()
                    .any(|p| matches!(p, Part::Clause(_)));
                // A number or text clause takes the text up to the next
                // literal (or the end).
                let end = match alt.0.get(index + 1) {
                    Some(Part::Lit(t)) if !t.trim().is_empty() => {
                        find_from(&chars, at, t.trim()).unwrap_or(chars.len())
                    }
                    _ => chars.len(),
                };
                let (value, next) = bodies.iter().find_map(|tokens| match category {
                    Category::Date => {
                        read_date(tokens, &chars, at, loc).map(|(d, n)| (Read::Date(d), n))
                    }
                    Category::Time => {
                        read_time(tokens, &chars, at, loc).map(|(t, n)| (Read::Time(t), n))
                    }
                    Category::DateTime => {
                        let split = tokens.iter().position(|t| *t == Token::Split);
                        let (dt, tt) = match split {
                            Some(s) => (&tokens[..s], &tokens[s + 1..]),
                            None => (&tokens[..], &[][..]),
                        };
                        let (d, n) = read_date(dt, &chars, at, loc)?;
                        let (t, n) = read_time(tt, &chars, n, loc)?;
                        Some((Read::DateTime(d, t), n))
                    }
                    Category::Num => {
                        let piece: String = chars[at..end].iter().collect();
                        read_number(tokens, &piece, loc).map(|v| (Read::Other(v), end))
                    }
                    Category::Text => {
                        let piece: String = chars[at..end].iter().collect();
                        read_text(tokens, &piece).map(|v| (Read::Other(v), end))
                    }
                    Category::Zero | Category::Null => None,
                })?;
                if last && clauses == 1 && !chars[next..].iter().all(|c| c.is_whitespace()) {
                    // Literals after the clause are matched below.
                    if !matches!(alt.0.get(index + 1), Some(Part::Lit(_))) {
                        return None;
                    }
                }
                at = next;
                match value {
                    Read::Date(d) => date = Some(d),
                    Read::Time(t) => time = Some(t),
                    Read::DateTime(d, t) => {
                        date = Some(d);
                        time = Some(t);
                    }
                    Read::Other(v) => other = Some(v),
                }
            }
        }
    }
    if !chars[at..].iter().all(|c| c.is_whitespace()) {
        return None;
    }
    if let Some(v) = other {
        return Some(v);
    }
    match (date, time, kind) {
        (Some(d), Some(t), _) => Some(format!("{}T{}", d.to_canonical(), t.to_canonical())),
        (Some(d), None, _) => Some(d.to_canonical()),
        (None, Some(t), _) => Some(t.to_canonical()),
        _ => None,
    }
}

enum Read {
    Date(Date),
    Time(Time),
    DateTime(Date, Time),
    Other(String),
}

fn find_from(chars: &[char], at: usize, needle: &str) -> Option<usize> {
    let needle: Vec<char> = needle.chars().collect();
    if needle.is_empty() {
        return None;
    }
    (at..chars.len()).find(|&i| chars[i..].starts_with(&needle))
}

/// `chars` from `at` past `literal`: a space matches any run of
/// whitespace (none included), letters compare without case.
fn match_literal(chars: &[char], mut at: usize, literal: &str) -> Option<usize> {
    for l in literal.chars() {
        if l.is_whitespace() {
            while chars.get(at).is_some_and(|c| c.is_whitespace()) {
                at += 1;
            }
            continue;
        }
        let c = *chars.get(at)?;
        if c != l && c.to_lowercase().ne(l.to_lowercase()) {
            return None;
        }
        at += 1;
    }
    Some(at)
}

/// Up to `max` (at least `min`) ASCII digits at `at`.
fn read_digits(chars: &[char], at: usize, min: usize, max: usize) -> Option<(u32, usize)> {
    let mut end = at;
    while end < chars.len() && end - at < max && chars[end].is_ascii_digit() {
        end += 1;
    }
    if end - at < min {
        return None;
    }
    let text: String = chars[at..end].iter().collect();
    Some((text.parse().ok()?, end))
}

/// The longest of `names` at `at`, without case: its index and the end.
fn read_name(chars: &[char], at: usize, names: &[String]) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for (index, name) in names.iter().enumerate() {
        let n = name.chars().count();
        if n == 0 || at + n > chars.len() {
            continue;
        }
        let here: String = chars[at..at + n].iter().collect();
        if here.to_lowercase() == name.to_lowercase() && best.is_none_or(|(_, e)| at + n > e) {
            best = Some((index, at + n));
        }
    }
    best
}

/// A two-digit year: 00–29 in this century's 2000s, else the 1900s.
fn century(yy: u32) -> i32 {
    if yy < 30 {
        2000 + yy as i32
    } else {
        1900 + yy as i32
    }
}

fn read_date(
    tokens: &[Token],
    chars: &[char],
    mut at: usize,
    loc: &Locale,
) -> Option<(Date, usize)> {
    let (mut y, mut m, mut d, mut doy) = (None, None, None, None);
    for token in tokens {
        let Token::Sym(c, n) = *token else {
            let mut text = String::new();
            push_literal(token, &mut text);
            at = match_literal(chars, at, &text)?;
            continue;
        };
        match c {
            'D' => {
                let (v, e) = read_digits(chars, at, sized(n, &[1, 2])?, 2)?;
                d = Some(v);
                at = e;
            }
            'J' => {
                let (v, e) = read_digits(chars, at, sized(n, &[1, 3])?, 3)?;
                doy = Some(v);
                at = e;
            }
            'M' => match sized(n, &[1, 2, 3, 4])? {
                k @ (1 | 2) => {
                    let (v, e) = read_digits(chars, at, k, 2)?;
                    m = Some(v);
                    at = e;
                }
                3 => {
                    let (i, e) = read_name(chars, at, &loc.months_abbr)?;
                    m = Some(i as u32 + 1);
                    at = e;
                }
                _ => {
                    let (i, e) = read_name(chars, at, &loc.months)?;
                    m = Some(i as u32 + 1);
                    at = e;
                }
            },
            'E' => match sized(n, &[1, 3, 4])? {
                1 => at = read_digits(chars, at, 1, 1)?.1,
                3 => at = read_name(chars, at, &loc.days_abbr)?.1,
                _ => at = read_name(chars, at, &loc.days)?.1,
            },
            'e' | 'w' => {
                sized(n, &[1])?;
                at = read_digits(chars, at, 1, 1)?.1;
            }
            'W' => {
                sized(n, &[2])?;
                at = read_digits(chars, at, 2, 2)?.1;
            }
            'G' => {
                sized(n, &[1])?;
                at = read_name(chars, at, &loc.eras)?.1;
            }
            'Y' => match sized(n, &[2, 4])? {
                2 => {
                    let (v, e) = read_digits(chars, at, 2, 2)?;
                    y = Some(century(v));
                    at = e;
                }
                _ => {
                    let (v, e) = read_digits(chars, at, 4, 4)?;
                    y = Some(v as i32);
                    at = e;
                }
            },
            _ => {
                let mut text = String::new();
                push_literal(token, &mut text);
                at = match_literal(chars, at, &text)?;
            }
        }
    }
    let y = y?;
    let date = match (m, d, doy) {
        (Some(m), Some(d), _) => Date::new(y, m, d)?,
        (Some(m), None, None) => Date::new(y, m, 1)?,
        (None, None, Some(j)) => Date::from_day_of_year(y, j)?,
        _ => return None,
    };
    Some((date, at))
}

fn read_time(
    tokens: &[Token],
    chars: &[char],
    mut at: usize,
    loc: &Locale,
) -> Option<(Time, usize)> {
    let mut t = Time::default();
    let mut hour12: Option<u32> = None;
    let mut pm = None;
    let mut any = false;
    for token in tokens {
        let Token::Sym(c, n) = *token else {
            let mut text = String::new();
            push_literal(token, &mut text);
            at = match_literal(chars, at, &text)?;
            continue;
        };
        let digits =
            |at: usize| -> Option<(u32, usize)> { read_digits(chars, at, sized(n, &[1, 2])?, 2) };
        match c {
            'h' => {
                let (v, e) = digits(at)?;
                if !(1..=12).contains(&v) {
                    return None;
                }
                hour12 = Some(v % 12);
                at = e;
            }
            'k' => {
                let (v, e) = digits(at)?;
                if v > 11 {
                    return None;
                }
                hour12 = Some(v);
                at = e;
            }
            'H' => {
                let (v, e) = digits(at)?;
                if v > 23 {
                    return None;
                }
                t.h = v;
                at = e;
            }
            'K' => {
                let (v, e) = digits(at)?;
                if !(1..=24).contains(&v) {
                    return None;
                }
                t.h = v % 24;
                at = e;
            }
            'M' => {
                let (v, e) = digits(at)?;
                if v > 59 {
                    return None;
                }
                t.m = v;
                at = e;
            }
            'S' => {
                let (v, e) = digits(at)?;
                if v > 59 {
                    return None;
                }
                t.s = v;
                at = e;
            }
            'F' => {
                sized(n, &[3])?;
                let (v, e) = read_digits(chars, at, 3, 3)?;
                t.ms = v;
                at = e;
            }
            'A' => {
                sized(n, &[1])?;
                let (i, e) = read_name(chars, at, &loc.meridiem)?;
                pm = Some(i == 1);
                at = e;
            }
            'Z' | 'z' => {
                sized(n, &[1, 2])?;
                let mut end = at;
                while end < chars.len()
                    && (chars[end].is_ascii_digit()
                        || matches!(chars[end], '+' | '-' | ':' | 'Z' | 'z'))
                {
                    end += 1;
                }
                if end > at {
                    let text: String = chars[at..end].iter().collect();
                    t.zone = Some(zone_of(&text)?);
                }
                at = end;
            }
            _ => {
                let mut text = String::new();
                push_literal(token, &mut text);
                at = match_literal(chars, at, &text)?;
                continue;
            }
        }
        any = true;
    }
    if !any {
        return None;
    }
    if let Some(h) = hour12 {
        t.h = h + if pm == Some(true) { 12 } else { 0 };
    }
    Some((t, at))
}

fn read_text(tokens: &[Token], text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    let mut out = String::new();
    for token in tokens {
        match *token {
            Token::Sym(c, n) => match text_symbol(c) {
                Some(accepts) => {
                    for _ in 0..n {
                        let ch = *chars.get(at).filter(|&&ch| accepts(ch))?;
                        out.push(ch);
                        at += 1;
                    }
                }
                None => {
                    let mut lit = String::new();
                    push_literal(token, &mut lit);
                    at = match_literal(&chars, at, &lit)?;
                }
            },
            _ => {
                let mut lit = String::new();
                push_literal(token, &mut lit);
                at = match_literal(&chars, at, &lit)?;
            }
        }
    }
    chars[at..].iter().all(|c| c.is_whitespace()).then_some(out)
}

/// A number as shown with a `num` body: the body's literals and the
/// locale's symbols taken out, then read as `lenient_number` reads it
/// (divided by 100 when the body has `%`).
fn read_number(tokens: &[Token], text: &str, loc: &Locale) -> Option<String> {
    let items = num_items(tokens)?;
    let mut text = text.to_string();
    for item in &items {
        if let NumItem::Lit(t) = item
            && !t.trim().is_empty()
        {
            text = text.replacen(t.trim(), " ", 1);
        }
    }
    let percent = items.contains(&NumItem::Percent);
    let mut d = number_text(&text, loc)?;
    if percent {
        d.shift(-2);
    }
    if let Some(at) = items.iter().position(|i| *i == NumItem::Radix(false))
        && !text.contains(loc.decimal.as_str())
    {
        // `v`: an implied radix, the digits after it are the fraction.
        let places = items[at..]
            .iter()
            .filter(|i| matches!(i, NumItem::Digit(_)))
            .count();
        d.shift(-(places as i32));
    }
    Some(d.to_canonical())
}

/// A number typed or shown: spaces, the grouping and currency symbols and
/// `%` removed, the locale's decimal symbol read as `.`, negative in a
/// leading or trailing minus, parentheses, `CR` or `DB`.
fn number_text(text: &str, loc: &Locale) -> Option<Decimal> {
    let mut t: String = text.trim().to_string();
    let mut neg = false;
    if t.starts_with('(') && t.ends_with(')') && t.len() >= 2 {
        neg = true;
        t = t[1..t.len() - 1].to_string();
    }
    for mark in ["CR", "cr", "DB", "db"] {
        if let Some(rest) = t.trim_end().strip_suffix(mark) {
            neg = true;
            t = rest.to_string();
        }
    }
    if !loc.currency.is_empty() {
        t = t.replace(loc.currency.as_str(), "");
    }
    t = t.replace(loc.percent.as_str(), "").replace('%', "");
    if !loc.grouping.trim().is_empty() && loc.grouping != loc.decimal {
        t = t.replace(loc.grouping.as_str(), "");
    }
    t.retain(|c| !c.is_whitespace() && c != '\u{a0}' && c != '\u{202f}');
    for minus in [loc.minus.as_str(), "-", "\u{2212}"] {
        if minus.is_empty() {
            continue;
        }
        if let Some(rest) = t.strip_prefix(minus).or_else(|| t.strip_suffix(minus)) {
            neg = !neg;
            t = rest.to_string();
            break;
        }
    }
    if let Some(rest) = t.strip_prefix('+') {
        t = rest.to_string();
    }
    if loc.decimal != "." {
        if t.contains('.') {
            // `.` is neither symbol here: not a number in this locale.
            return None;
        }
        t = t.replace(loc.decimal.as_str(), ".");
    }
    if t.is_empty() || t.starts_with(['+', '-']) {
        return None;
    }
    let mut d = Decimal::parse(&t)?;
    if t.contains(['e', 'E']) {
        return None;
    }
    d.neg = neg;
    Some(d)
}

/// `text` read without a picture, by the value type: numbers with their
/// grouping and symbols removed, dates in the canonical form, the
/// locale's patterns, numeric day, month and year in the locale's order,
/// or a month name with a day and year; times as `H:MM[:SS] [AM|PM]`.
/// `None` for text and anything that does not read.
pub fn lenient(kind: ValueKind, text: &str, loc: &Locale) -> Option<String> {
    if text.chars().count() > MAX_VALUE_CHARS {
        return None;
    }
    let text = text.trim();
    if text.is_empty() {
        return Some(String::new());
    }
    match kind {
        ValueKind::Integer | ValueKind::Decimal | ValueKind::Float => {
            Some(number_text(text, loc)?.to_canonical())
        }
        ValueKind::Date => lenient_date(text, loc).map(Date::to_canonical),
        ValueKind::Time => lenient_time(text, loc).map(Time::to_canonical),
        ValueKind::DateTime => {
            if let Some((d, t)) = text.split_once('T')
                && let (Some(d), Some(t)) = (Date::canonical(d), Time::canonical(t))
            {
                return Some(format!("{}T{}", d.to_canonical(), t.to_canonical()));
            }
            let (d, t) = text.rsplit_once(' ').unwrap_or((text, ""));
            let date = lenient_date(d, loc)?;
            match lenient_time(t, loc) {
                Some(t) => Some(format!("{}T{}", date.to_canonical(), t.to_canonical())),
                None => Some(format!("{}T00:00:00", date.to_canonical())),
            }
        }
        _ => None,
    }
}

fn lenient_date(text: &str, loc: &Locale) -> Option<Date> {
    if let Some(d) = Date::canonical(text) {
        return Some(d);
    }
    let chars: Vec<char> = text.chars().collect();
    for keyword in ["full", "long", "med", "short"] {
        let pattern: Vec<char> = loc.date_pattern(keyword).chars().collect();
        if let Some(tokens) = tokens(&pattern, false)
            && let Some((d, end)) = read_date(&tokens, &chars, 0, loc)
            && chars[end..].iter().all(|c| c.is_whitespace())
        {
            return Some(d);
        }
    }
    // Words and numbers.
    let mut numbers: Vec<&str> = Vec::new();
    let mut month = None;
    for word in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if word.bytes().all(|b| b.is_ascii_digit()) {
            numbers.push(word);
        } else if let Some(m) = month_named(word, loc) {
            if month.is_some() {
                return None;
            }
            month = Some(m);
        } else if !day_named(word, loc) {
            return None;
        }
    }
    let number = |s: &str| digits_of(s);
    let year = |s: &str| -> Option<i32> {
        match s.len() {
            4 => Some(number(s)? as i32),
            1 | 2 => Some(century(number(s)?)),
            _ => None,
        }
    };
    match (month, numbers.as_slice()) {
        (Some(m), [a, b]) => {
            let (day, y) = if a.len() == 4 { (b, a) } else { (a, b) };
            Date::new(year(y)?, m, number(day)?)
        }
        (None, [a, b, c]) => {
            if a.len() == 4 {
                return Date::new(year(a)?, number(b)?, number(c)?);
            }
            let (mut d, mut m, mut y) = (None, None, None);
            for (value, slot) in [a, b, c].into_iter().zip(numeric_order(loc)) {
                match slot {
                    'D' => d = number(value),
                    'M' => m = number(value),
                    _ => y = year(value),
                }
            }
            Date::new(y?, m?, d?)
        }
        _ => None,
    }
}

/// The order of day, month and year in the locale's short date pattern.
fn numeric_order(loc: &Locale) -> [char; 3] {
    let mut order = Vec::new();
    for c in loc.date_patterns[3].chars() {
        if matches!(c, 'D' | 'M' | 'Y') && !order.contains(&c) {
            order.push(c);
        }
    }
    match order.as_slice() {
        [a, b, c] => [*a, *b, *c],
        _ => ['M', 'D', 'Y'],
    }
}

fn month_named(word: &str, loc: &Locale) -> Option<u32> {
    let lower = word.to_lowercase();
    let at = |names: &[String]| names.iter().position(|n| n.to_lowercase() == lower);
    let found = at(&loc.months)
        .or_else(|| at(&loc.months_abbr))
        .or_else(|| {
            (lower.chars().count() >= 3)
                .then(|| {
                    loc.months
                        .iter()
                        .position(|n| n.to_lowercase().starts_with(&lower))
                })
                .flatten()
        })?;
    Some(found as u32 + 1)
}

fn day_named(word: &str, loc: &Locale) -> bool {
    let lower = word.to_lowercase();
    loc.days
        .iter()
        .chain(&loc.days_abbr)
        .any(|n| n.to_lowercase() == lower)
}

fn lenient_time(text: &str, loc: &Locale) -> Option<Time> {
    if let Some(t) = Time::canonical(text) {
        return Some(t);
    }
    let lower = text.trim().to_lowercase();
    let mut clock = lower.as_str();
    let mut pm = None;
    for (index, name) in loc.meridiem.iter().enumerate() {
        let name = name.to_lowercase();
        if let Some(rest) = clock.strip_suffix(name.as_str()) {
            pm = Some(index == 1);
            clock = rest;
            break;
        }
    }
    let parts: Vec<&str> = clock.trim().split(':').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    let mut values = [0u32; 3];
    for (slot, part) in values.iter_mut().zip(&parts) {
        if part.len() > 2 {
            return None;
        }
        *slot = digits_of(part)?;
    }
    let [mut h, m, s] = values;
    match pm {
        Some(after) => {
            if !(1..=12).contains(&h) {
                return None;
            }
            h = h % 12 + if after { 12 } else { 0 };
        }
        None if h > 23 => return None,
        None => {}
    }
    (m < 60 && s < 60).then_some(Time {
        h,
        m,
        s,
        ..Time::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn en() -> LocaleSet {
        LocaleSet::default()
    }

    fn fmt(picture: &str, kind: ValueKind, value: &str) -> Option<String> {
        format(&parse(picture)?, kind, value, &en(), "en_US")
    }

    fn back(picture: &str, kind: ValueKind, text: &str) -> Option<String> {
        parse_value(&parse(picture)?, kind, text, &en(), "en_US")
    }

    #[test]
    fn dates_format_and_read_back() {
        let d = ValueKind::Date;
        assert_eq!(
            fmt("date{MMMM D, YYYY}", d, "2026-11-05").as_deref(),
            Some("November 5, 2026")
        );
        assert_eq!(
            fmt("date{MMM D, YYYY}", d, "2026-11-05").as_deref(),
            Some("Nov 5, 2026")
        );
        assert_eq!(
            fmt("date{MMMM, YYYY}", d, "2026-11-05").as_deref(),
            Some("November, 2026")
        );
        assert_eq!(
            fmt("date{DD/MM/YY}", d, "2026-01-09").as_deref(),
            Some("09/01/26")
        );
        assert_eq!(
            fmt("date{EEEE', the 'D}", d, "2026-11-05").as_deref(),
            Some("Thursday, the 5")
        );
        assert_eq!(
            fmt("date{EEE e E}", d, "2026-11-08").as_deref(),
            Some("Sun 7 1")
        );
        assert_eq!(
            fmt("date{JJJ J}", d, "2024-12-31").as_deref(),
            Some("366 366")
        );
        assert_eq!(
            fmt("date{YYYY-'W'WW w G}", d, "2021-01-03").as_deref(),
            Some("2021-W53 2 AD")
        );
        assert_eq!(
            fmt("date{long}", d, "2026-11-05").as_deref(),
            Some("November 5, 2026")
        );
        assert_eq!(
            fmt("date.full{}", d, "2026-11-05").as_deref(),
            Some("Thursday, November 5, 2026")
        );
        assert_eq!(
            fmt("date{short}", d, "2026-11-05").as_deref(),
            Some("11/5/26")
        );
        // A bare picture takes the value type's category.
        assert_eq!(
            fmt("MM/DD/YYYY", d, "20261105").as_deref(),
            Some("11/05/2026")
        );
        // Not a date: shown as it is (the caller's fallback).
        assert_eq!(fmt("date{MMMM D, YYYY}", d, "2026-02-30"), None);
        assert_eq!(fmt("date{MMMM D, YYYY}", d, "soon"), None);

        assert_eq!(
            back("date{MMMM D, YYYY}", d, "November 5, 2026").as_deref(),
            Some("2026-11-05")
        );
        assert_eq!(
            back("date{MMMM D, YYYY}", d, " november 5,2026 ").as_deref(),
            Some("2026-11-05")
        );
        assert_eq!(
            back("date{MMM D, YYYY}", d, "Nov 5, 2026").as_deref(),
            Some("2026-11-05")
        );
        assert_eq!(
            back("date{long}", d, "December 1, 2026").as_deref(),
            Some("2026-12-01")
        );
        assert_eq!(
            back("date{DD/MM/YY}", d, "09/01/26").as_deref(),
            Some("2026-01-09")
        );
        assert_eq!(
            back("date{YYYY.JJJ}", d, "2024.366").as_deref(),
            Some("2024-12-31")
        );
        assert_eq!(back("date{MMMM D, YYYY}", d, "12/01/2026"), None);
        assert_eq!(back("date{MMMM D, YYYY}", d, "November 31, 2026"), None);
        for value in ["2026-11-05", "2024-02-29", "1999-12-31", "2026-01-01"] {
            for picture in [
                "date{MMMM D, YYYY}",
                "date{MMM D, YYYY}",
                "date{DD.MM.YYYY}",
                "date{EEEE, MMMM D, YYYY}",
                "date{YYYYMMDD}",
            ] {
                let shown = fmt(picture, d, value).unwrap();
                assert_eq!(
                    back(picture, d, &shown).as_deref(),
                    Some(value),
                    "{picture} {shown}"
                );
            }
        }
    }

    #[test]
    fn times_and_date_times() {
        let t = ValueKind::Time;
        assert_eq!(
            fmt("time{h:MM A}", t, "14:05:09").as_deref(),
            Some("2:05 PM")
        );
        assert_eq!(
            fmt("time{hh:MM:SS A}", t, "00:05:09").as_deref(),
            Some("12:05:09 AM")
        );
        assert_eq!(
            fmt("time{HH:MM:SS.FFF}", t, "09:05:09.250").as_deref(),
            Some("09:05:09.250")
        );
        assert_eq!(fmt("time{k K}", t, "00:00:00").as_deref(), Some("0 24"));
        assert_eq!(
            fmt("time{HH:MM z}", t, "09:05:00-05:00").as_deref(),
            Some("09:05 -05:00")
        );
        assert_eq!(
            fmt("time{HH:MM Z}", t, "09:05:00Z").as_deref(),
            Some("09:05 Z")
        );
        assert_eq!(
            fmt("time{short}", t, "14:05:00").as_deref(),
            Some("2:05 PM")
        );
        assert_eq!(
            back("time{h:MM A}", t, "2:05 pm").as_deref(),
            Some("14:05:00")
        );
        assert_eq!(
            back("time{h:MM A}", t, "12:30 AM").as_deref(),
            Some("00:30:00")
        );
        assert_eq!(back("time{HH:MM:SS}", t, "25:00:00"), None);
        let dt = ValueKind::DateTime;
        assert_eq!(
            fmt(
                "datetime{MMM D, YYYY' at 'Th:MM A}",
                dt,
                "2026-11-05T14:05:00"
            )
            .as_deref(),
            Some("Nov 5, 2026 at 2:05 PM")
        );
        assert_eq!(
            fmt("date{YYYY-MM-DD} time{HH:MM}", dt, "2026-11-05T14:05:00").as_deref(),
            Some("2026-11-05 14:05")
        );
        assert_eq!(
            back(
                "datetime{MMM D, YYYY' at 'Th:MM A}",
                dt,
                "Nov 5, 2026 at 2:05 PM"
            )
            .as_deref(),
            Some("2026-11-05T14:05:00")
        );
        assert_eq!(
            back("date{YYYY-MM-DD} time{HH:MM}", dt, "2026-11-05 14:05").as_deref(),
            Some("2026-11-05T14:05:00")
        );
    }

    #[test]
    fn numbers_format_with_grouping_signs_and_alternatives() {
        let n = ValueKind::Decimal;
        assert_eq!(
            fmt("num{$z,zz9.99}", n, "1234.5").as_deref(),
            Some("$1,234.50")
        );
        assert_eq!(fmt("num{$z,zz9.99}", n, "4.5").as_deref(), Some("$4.50"));
        assert_eq!(fmt("num{$z,zz9.99}", n, "0").as_deref(), Some("$0.00"));
        assert_eq!(fmt("num{$z,zz9.99}", n, "12345"), None, "too many digits");
        assert_eq!(fmt("num{$z,zz9.99}", n, "-5"), None, "no sign symbol");
        assert_eq!(
            fmt("num{z,zz9.99|(z,zz9.99)}", n, "-1234.567").as_deref(),
            Some("(1,234.57)")
        );
        assert_eq!(
            fmt("num{(zzz,zzz,zz9.99)}", n, "1234.5").as_deref(),
            Some(" 1,234.50 ")
        );
        assert_eq!(
            fmt("num{(zzz,zzz,zz9.99)}", n, "-1234.5").as_deref(),
            Some("(1,234.50)")
        );
        assert_eq!(fmt("num{zzz9.99%}", n, "0.125").as_deref(), Some("12.50%"));
        assert_eq!(fmt("num{ZZZ9}", n, "42").as_deref(), Some("  42"));
        assert_eq!(fmt("num{Z,ZZ9}", n, "42").as_deref(), Some("   42"));
        assert_eq!(fmt("num{9,999}", n, "42").as_deref(), Some("0,042"));
        assert_eq!(fmt("num{z9.zz}", n, "3.10").as_deref(), Some("3.1"));
        assert_eq!(fmt("num{z9.ZZ}", n, "3").as_deref(), Some("3.  "));
        assert_eq!(fmt("num{s9.9}", n, "-0.04").as_deref(), Some("0.0"));
        assert_eq!(fmt("num{S9}", n, "7").as_deref(), Some(" 7"));
        assert_eq!(fmt("num{+9}", n, "7").as_deref(), Some("+7"));
        assert_eq!(fmt("num{zz9CR}", n, "-7").as_deref(), Some("7CR"));
        assert_eq!(fmt("num{9.99E}", n, "1234").as_deref(), Some("1.23E+3"));
        assert_eq!(fmt("num{zz9v99}", n, "1.5").as_deref(), Some("150"));
        assert_eq!(fmt("num.currency{}", n, "-3").as_deref(), Some("($3.00)"));
        assert_eq!(fmt("num{currency}", n, "3").as_deref(), Some("$3.00"));
        assert_eq!(
            fmt("num.integer{}", n, "1234567").as_deref(),
            Some("1,234,567")
        );
        assert_eq!(fmt("num{z,zz9}", n, "1e3").as_deref(), Some("1,000"));
        assert_eq!(fmt("num{z9}", n, "abc"), None);
        assert_eq!(fmt("null{'none'}|num{z9}", n, "").as_deref(), Some("none"));
        assert_eq!(
            fmt("zero{'nil'}|num{z9}", n, "0.00").as_deref(),
            Some("nil")
        );
        assert_eq!(fmt("zero{'nil'}|num{z9}", n, "5").as_deref(), Some("5"));
        assert_eq!(fmt("num{z9}", n, ""), None);

        assert_eq!(
            back("num{$z,zz9.99}", n, "$1,234.50").as_deref(),
            Some("1234.50")
        );
        assert_eq!(
            back("num{$z,zz9.99}", n, "1234.5").as_deref(),
            Some("1234.5")
        );
        assert_eq!(
            back("num{(zzz,zzz,zz9.99)}", n, "(1,234.50)").as_deref(),
            Some("-1234.50")
        );
        assert_eq!(
            back("num{zzz9.99%}", n, "12.50%").as_deref(),
            Some("0.1250")
        );
        assert_eq!(back("num{zz9v99}", n, "150").as_deref(), Some("1.50"));
        assert_eq!(back("null{'none'}|num{z9}", n, "none").as_deref(), Some(""));
        assert_eq!(back("num{z9}", n, "twelve"), None);
        for value in ["1234.50", "0.00", "-17.25", "999999.99"] {
            let picture = "num{z,zzz,zz9.99|(z,zzz,zz9.99)}";
            let shown = fmt(picture, n, value).unwrap();
            assert_eq!(back(picture, n, &shown).as_deref(), Some(value), "{shown}");
        }
    }

    #[test]
    fn locale_symbols_come_from_the_locale_set() {
        let set = LocaleSet::read(Some(
            "<localeSet xmlns=\"http://www.xfa.org/schema/xfa-locale-set/2.7/\">\
             <locale name=\"fr_FR\"><calendarSymbols name=\"gregorian\">\
             <monthNames><month>janvier</month><month>février</month><month>mars</month><month>avril</month>\
             <month>mai</month><month>juin</month><month>juillet</month><month>août</month>\
             <month>septembre</month><month>octobre</month><month>novembre</month><month>décembre</month></monthNames>\
             </calendarSymbols><datePatterns><datePattern name=\"short\">DD/MM/YY</datePattern></datePatterns>\
             <numberSymbols><numberSymbol name=\"decimal\">,</numberSymbol>\
             <numberSymbol name=\"grouping\">\u{a0}</numberSymbol></numberSymbols>\
             <currencySymbols><currencySymbol name=\"symbol\">€</currencySymbol></currencySymbols></locale></localeSet>",
        ));
        assert_eq!(set.locales.len(), 1);
        let fr = set.get("fr_FR");
        assert_eq!((fr.decimal.as_str(), fr.grouping.as_str()), (",", "\u{a0}"));
        assert_eq!(fr.months[10], "novembre");
        assert_eq!(fr.days[0], "Sunday", "missing names stay en_US's");
        assert_eq!(set.get("fr_CA").name, "fr_FR", "same language");
        assert_eq!(set.get("de_DE").name, "en_US", "built in");
        let p = parse("num{z,zzz,zz9.99 $}").unwrap();
        let shown = format(&p, ValueKind::Decimal, "1234567.5", &set, "fr_FR").unwrap();
        assert_eq!(shown, "1\u{a0}234\u{a0}567,50 €");
        assert_eq!(
            parse_value(&p, ValueKind::Decimal, &shown, &set, "fr_FR").as_deref(),
            Some("1234567.50")
        );
        let d = parse("date{D MMMM YYYY}").unwrap();
        assert_eq!(
            format(&d, ValueKind::Date, "2026-11-05", &set, "fr_FR").as_deref(),
            Some("5 novembre 2026")
        );
        // A clause's own locale wins over the field's.
        let us = parse("num(en_US){z,zz9.99}").unwrap();
        assert_eq!(
            format(&us, ValueKind::Decimal, "1234.5", &set, "fr_FR").as_deref(),
            Some("1,234.50")
        );
        // Lenient reading in the locale's order and symbols.
        assert_eq!(
            lenient(ValueKind::Date, "05/11/2026", fr).as_deref(),
            Some("2026-11-05")
        );
        assert_eq!(
            lenient(ValueKind::Decimal, "1 234,5", fr).as_deref(),
            Some("1234.5")
        );
        assert_eq!(lenient(ValueKind::Decimal, "1.5", fr), None);
    }

    #[test]
    fn lenient_reading_by_type() {
        let en = Locale::en_us();
        let date = |t: &str| lenient(ValueKind::Date, t, &en);
        assert_eq!(date("11/05/2026").as_deref(), Some("2026-11-05"));
        assert_eq!(date("2026-11-05").as_deref(), Some("2026-11-05"));
        assert_eq!(date("Nov 5, 2026").as_deref(), Some("2026-11-05"));
        assert_eq!(date("5 November 2026").as_deref(), Some("2026-11-05"));
        assert_eq!(
            date("Thursday, November 5, 2026").as_deref(),
            Some("2026-11-05")
        );
        assert_eq!(date("2026/11/05").as_deref(), Some("2026-11-05"));
        assert_eq!(date("11/5/26").as_deref(), Some("2026-11-05"));
        assert_eq!(date("13/45/2026"), None);
        assert_eq!(date("tomorrow"), None);
        let num = |t: &str| lenient(ValueKind::Decimal, t, &en);
        assert_eq!(num("$1,234.50").as_deref(), Some("1234.50"));
        assert_eq!(num("(12)").as_deref(), Some("-12"));
        assert_eq!(num("-0012.0").as_deref(), Some("-12.0"));
        assert_eq!(num("1,2,3").as_deref(), Some("123"));
        assert_eq!(num("1.2.3"), None);
        assert_eq!(num("12abc"), None);
        assert_eq!(
            lenient(ValueKind::Time, "2:05 PM", &en).as_deref(),
            Some("14:05:00")
        );
        assert_eq!(
            lenient(ValueKind::Time, "14:05", &en).as_deref(),
            Some("14:05:00")
        );
        assert_eq!(lenient(ValueKind::Text, "x", &en), None);
        assert_eq!(lenient(ValueKind::Date, "  ", &en).as_deref(), Some(""));
    }

    #[test]
    fn text_pictures() {
        let t = ValueKind::Text;
        assert_eq!(
            fmt("text{999-999-9999}", t, "4165551234").as_deref(),
            Some("416-555-1234")
        );
        assert_eq!(
            fmt("text{A9A 9A9}", t, "K1A0B1").as_deref(),
            Some("K1A 0B1")
        );
        assert_eq!(
            fmt("text{'('999') 'XXX}", t, "416abc").as_deref(),
            Some("(416) abc")
        );
        assert_eq!(fmt("text{999}", t, "12a"), None);
        assert_eq!(fmt("text{999}", t, "1234"), None, "left over");
        assert_eq!(
            back("text{999-999-9999}", t, "416-555-1234").as_deref(),
            Some("4165551234")
        );
        assert_eq!(
            back("text{A9A 9A9}", t, "K1A 0B1").as_deref(),
            Some("K1A0B1")
        );
        assert_eq!(back("text{999}", t, "abc"), None);
    }

    #[test]
    fn malformed_pictures_are_not_used() {
        for bad in [
            "",
            "   ",
            "date{MMMM D, YYYY",
            "date{'D}",
            "dat{D}",
            "date(){D}",
            "date{D}x{",
            "date{a{b}}",
            "date{D} words",
            "'unclosed",
            "|",
            "date{D}||date{M}",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
        let long = format!("num{{{}}}", "9".repeat(MAX_PICTURE_CHARS));
        assert_eq!(parse(&long), None);
        let many = ["num{9}"; MAX_ALTERNATIVES + 1].join("|");
        assert_eq!(parse(&many), None);
        let eight = ["num{9}"; MAX_ALTERNATIVES].join("|");
        assert!(parse(&eight).is_some());
        // Symbols of a size the category does not have: no format.
        assert_eq!(fmt("date{DDD}", ValueKind::Date, "2026-11-05"), None);
        assert_eq!(fmt("num{9..9}", ValueKind::Decimal, "1"), None);
        assert_eq!(fmt("num{9.9.9}", ValueKind::Decimal, "1"), None);
        assert_eq!(fmt("time{FF}", ValueKind::Time, "10:00:00"), None);
        // Hostile values never panic.
        for value in [
            "9".repeat(600),
            "-".into(),
            ".".into(),
            "1e999".into(),
            "\u{0}".into(),
        ] {
            let _ = fmt("num{z,zz9.99}", ValueKind::Decimal, &value);
            let _ = back("num{z,zz9.99}", ValueKind::Decimal, &value);
            let _ = back("date{MMMM D, YYYY}", ValueKind::Date, &value);
            let _ = lenient(ValueKind::Date, &value, &Locale::en_us());
        }
    }
}
