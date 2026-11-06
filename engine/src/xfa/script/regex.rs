// SPDX-License-Identifier: MIT
//! JavaScript regular expressions for form scripts: the pattern syntax
//! Designer forms use (a phone or postal code check, a split on dashes)
//! read into a tree, and a backtracking matcher over the text's chars.
//!
//! Rules:
//! - Syntax: literals and escapes (`\n`, `\t`, `\r`, `\f`, `\v`, `\0`,
//!   `\xHH`, `\uHHHH`, any escaped punctuator), `.`, classes `[...]` with
//!   ranges and negation, `\d \D \w \W \s \S` inside and outside classes,
//!   `^ $ \b \B`, groups `( )`, `(?: )`, lookaheads `(?= )` `(?! )`,
//!   back references `\1`..`\9`, alternation, and the quantifiers
//!   `* + ? {n} {n,} {n,m}`, each lazy with a trailing `?`. Flags `g`, `i`
//!   and `m`. Anything else (lookbehind, named groups, `u`/`y`/`s` flags)
//!   does not read: `Regex::new` gives `None` and the script sees
//!   `undefined`, as before this module.
//! - `.` matches anything but a line end; `^`/`$` match at the text's ends,
//!   and at line ends too with `m`; `i` compares simple case folds.
//! - Bounded: a pattern of at most `MAX_PATTERN` chars and `MAX_NODES`
//!   nodes, nested `MAX_NESTING` deep, counts at most `MAX_COUNT`; a match
//!   spends at most the steps it is given and recurses at most `MAX_DEPTH`
//!   deep (a repeat of one char, `[0-9]{3}`, `.*`, loops without
//!   recursing), and gives `Err(Exhausted)` past either.

use std::cell::Cell;

/// Pattern length read, in chars, at most.
pub const MAX_PATTERN: usize = 4096;
/// Nodes in one pattern, at most.
pub const MAX_NODES: usize = 4096;
/// Group nesting read, at most.
pub const MAX_NESTING: usize = 64;
/// A `{n,m}` count, at most.
pub const MAX_COUNT: u32 = 10_000;
/// Matcher recursion, at most (a few stack frames each: kept well inside
/// a WASM or worker thread's stack).
pub const MAX_DEPTH: usize = 400;

/// A match ran out of steps or depth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exhausted;

/// A read pattern.
#[derive(Clone, Debug)]
pub struct Regex {
    node: Node,
    /// Capturing groups (group 0, the whole match, not counted).
    groups: usize,
    pub source: String,
    pub global: bool,
    pub ignore_case: bool,
    pub multiline: bool,
}

/// Where each group matched, by char index: `[0]` the whole match.
pub type Captures = Vec<Option<(usize, usize)>>;

#[derive(Clone, Debug)]
enum Node {
    Empty,
    Char(char),
    Any,
    Class(Class),
    Start,
    End,
    /// `\b` (true) or `\B`.
    WordBoundary(bool),
    /// A group: its capture index (`None` for `(?:`).
    Group(Box<Node>, Option<usize>),
    /// A lookahead: negated?
    Look(Box<Node>, bool),
    BackRef(usize),
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat {
        node: Box<Node>,
        min: u32,
        max: Option<u32>,
        greedy: bool,
    },
}

#[derive(Clone, Debug)]
struct Class {
    negated: bool,
    items: Vec<Item>,
}

#[derive(Clone, Copy, Debug)]
enum Item {
    Range(char, char),
    /// `\d`, `\w`, `\s`: negated?
    Digit(bool),
    Word(bool),
    Space(bool),
}

impl Regex {
    /// `pattern` read with `flags`; `None` when either does not read
    /// (module rules).
    pub fn new(pattern: &str, flags: &str) -> Option<Regex> {
        let (mut global, mut ignore_case, mut multiline) = (false, false, false);
        for f in flags.chars() {
            let seen = match f {
                'g' => &mut global,
                'i' => &mut ignore_case,
                'm' => &mut multiline,
                _ => return None,
            };
            if *seen {
                return None;
            }
            *seen = true;
        }
        let chars: Vec<char> = pattern.chars().collect();
        if chars.len() > MAX_PATTERN {
            return None;
        }
        let mut reader = Reader {
            chars: &chars,
            at: 0,
            groups: 0,
            nodes: 0,
            nesting: 0,
            bad: false,
        };
        let node = reader.alternation()?;
        if reader.at != chars.len() || reader.bad {
            return None;
        }
        Some(Regex {
            node,
            groups: reader.groups,
            source: pattern.to_string(),
            global,
            ignore_case,
            multiline,
        })
    }

    /// Capturing groups.
    pub fn groups(&self) -> usize {
        self.groups
    }

    /// The flags as JavaScript writes them.
    pub fn flags(&self) -> String {
        let mut out = String::new();
        for (on, f) in [
            (self.global, 'g'),
            (self.ignore_case, 'i'),
            (self.multiline, 'm'),
        ] {
            if on {
                out.push(f);
            }
        }
        out
    }

    /// The first match starting at or after `from`, spending at most
    /// `steps` (what is left of them written back).
    pub fn find(
        &self,
        text: &[char],
        from: usize,
        steps: &mut usize,
    ) -> Result<Option<Captures>, Exhausted> {
        let mut start = from;
        while start <= text.len() {
            if let Some(caps) = self.match_at(text, start, steps)? {
                return Ok(Some(caps));
            }
            start += 1;
        }
        Ok(None)
    }

    /// A match starting exactly at `at`.
    pub fn match_at(
        &self,
        text: &[char],
        at: usize,
        steps: &mut usize,
    ) -> Result<Option<Captures>, Exhausted> {
        let m = Matcher {
            re: self,
            text,
            steps: Cell::new(*steps),
            depth: Cell::new(0),
            exhausted: Cell::new(false),
        };
        let mut caps: Captures = vec![None; self.groups + 1];
        let mut end = None;
        let found = m.seq(
            std::slice::from_ref(&self.node),
            at,
            &mut caps,
            &mut |p, _| {
                end = Some(p);
                true
            },
        );
        *steps = m.steps.get();
        if m.exhausted.get() {
            return Err(Exhausted);
        }
        Ok(match (found, end) {
            (true, Some(end)) => {
                caps[0] = Some((at, end));
                Some(caps)
            }
            _ => None,
        })
    }
}

// ---------------------------------------------------------------- reading

struct Reader<'c> {
    chars: &'c [char],
    at: usize,
    groups: usize,
    nodes: usize,
    nesting: usize,
    /// A count out of order (`{2,1}`) was read: the pattern does not read.
    bad: bool,
}

impl Reader<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn node(&mut self, n: Node) -> Option<Node> {
        self.nodes += 1;
        (self.nodes <= MAX_NODES).then_some(n)
    }

    fn alternation(&mut self) -> Option<Node> {
        let mut branches = vec![self.concat()?];
        while self.eat('|') {
            branches.push(self.concat()?);
        }
        if branches.len() == 1 {
            branches.pop()
        } else {
            self.node(Node::Alt(branches))
        }
    }

    fn concat(&mut self) -> Option<Node> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            items.push(self.quantified(atom)?);
        }
        match items.len() {
            0 => self.node(Node::Empty),
            1 => items.pop(),
            _ => self.node(Node::Concat(items)),
        }
    }

    fn quantified(&mut self, atom: Node) -> Option<Node> {
        let (min, max) = match self.peek() {
            Some('*') => {
                self.at += 1;
                (0, None)
            }
            Some('+') => {
                self.at += 1;
                (1, None)
            }
            Some('?') => {
                self.at += 1;
                (0, Some(1))
            }
            Some('{') => match self.counts() {
                Some(c) => c,
                // `{` that is no count is a literal (as browsers read it).
                None => return Some(atom),
            },
            _ => return Some(atom),
        };
        if matches!(
            atom,
            Node::Start | Node::End | Node::WordBoundary(_) | Node::Look(..)
        ) {
            return None;
        }
        let greedy = !self.eat('?');
        self.node(Node::Repeat {
            node: Box::new(atom),
            min,
            max,
            greedy,
        })
    }

    /// `{n}`, `{n,}` or `{n,m}` at the cursor (consumed), else `None`
    /// with the cursor where it was.
    fn counts(&mut self) -> Option<(u32, Option<u32>)> {
        let start = self.at;
        self.at += 1;
        let number = |r: &mut Reader<'_>| -> Option<u32> {
            let from = r.at;
            while r.peek().is_some_and(|c| c.is_ascii_digit()) {
                r.at += 1;
            }
            if r.at == from {
                return None;
            }
            let text: String = r.chars[from..r.at].iter().collect();
            text.parse::<u32>().ok().filter(|n| *n <= MAX_COUNT)
        };
        let parsed = (|| {
            let min = number(self)?;
            let max = if self.eat(',') {
                if self.peek() == Some('}') {
                    None
                } else {
                    Some(number(self)?)
                }
            } else {
                Some(min)
            };
            if !self.eat('}') {
                return None;
            }
            if max.is_some_and(|m| m < min) {
                self.bad = true;
                return None;
            }
            Some((min, max))
        })();
        if parsed.is_none() {
            self.at = start;
        }
        parsed
    }

    fn atom(&mut self) -> Option<Node> {
        let c = self.peek()?;
        self.at += 1;
        match c {
            '.' => self.node(Node::Any),
            '^' => self.node(Node::Start),
            '$' => self.node(Node::End),
            '(' => self.group(),
            '[' => {
                let class = self.class()?;
                self.node(Node::Class(class))
            }
            '\\' => self.escape(),
            '*' | '+' | '?' => None,
            '{' => {
                // A count with nothing before it does not read; a lone `{`
                // is a literal.
                self.at -= 1;
                if self.counts().is_some() {
                    return None;
                }
                self.at += 1;
                self.node(Node::Char('{'))
            }
            other => self.node(Node::Char(other)),
        }
    }

    fn group(&mut self) -> Option<Node> {
        self.nesting += 1;
        if self.nesting > MAX_NESTING {
            return None;
        }
        let kind = if self.eat('?') {
            match self.peek()? {
                ':' => 0,
                '=' => 1,
                '!' => 2,
                _ => return None,
            }
        } else {
            3
        };
        if kind != 3 {
            self.at += 1;
        }
        let index = (kind == 3).then(|| {
            self.groups += 1;
            self.groups
        });
        let inner = self.alternation()?;
        if !self.eat(')') {
            return None;
        }
        self.nesting -= 1;
        let node = match kind {
            1 => Node::Look(Box::new(inner), false),
            2 => Node::Look(Box::new(inner), true),
            _ => Node::Group(Box::new(inner), index),
        };
        self.node(node)
    }

    fn escape(&mut self) -> Option<Node> {
        let c = self.peek()?;
        self.at += 1;
        let node = match c {
            'd' | 'D' => Node::Class(Class::of(Item::Digit(c == 'D'))),
            'w' | 'W' => Node::Class(Class::of(Item::Word(c == 'W'))),
            's' | 'S' => Node::Class(Class::of(Item::Space(c == 'S'))),
            'b' => Node::WordBoundary(true),
            'B' => Node::WordBoundary(false),
            '1'..='9' => Node::BackRef(c as usize - '0' as usize),
            _ => {
                self.at -= 1;
                Node::Char(self.escaped_char()?)
            }
        };
        self.node(node)
    }

    /// The char an escape (after `\`) stands for: control escapes, hex
    /// and Unicode escapes, or the char itself.
    fn escaped_char(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += 1;
        Some(match c {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            'f' => '\u{c}',
            'v' => '\u{b}',
            '0' => '\0',
            'x' => self.hex(2)?,
            'u' => self.hex(4)?,
            other if other.is_ascii_alphanumeric() => return None,
            other => other,
        })
    }

    fn hex(&mut self, n: usize) -> Option<char> {
        let digits: String = self.chars.get(self.at..self.at + n)?.iter().collect();
        let v = u32::from_str_radix(&digits, 16).ok()?;
        self.at += n;
        char::from_u32(v)
    }

    fn class(&mut self) -> Option<Class> {
        let negated = self.eat('^');
        let mut items = Vec::new();
        let mut first = true;
        loop {
            let c = self.peek()?;
            if c == ']' && !first {
                self.at += 1;
                break;
            }
            if c == ']' {
                // `[]` matches nothing, `[^]` anything.
                self.at += 1;
                return Some(Class { negated, items });
            }
            first = false;
            let low = match self.class_atom()? {
                ClassAtom::Item(item) => {
                    items.push(item);
                    continue;
                }
                ClassAtom::Char(c) => c,
            };
            if self.peek() == Some('-') && self.chars.get(self.at + 1) != Some(&']') {
                self.at += 1;
                match self.class_atom()? {
                    ClassAtom::Char(high) if high >= low => items.push(Item::Range(low, high)),
                    ClassAtom::Char(_) => return None,
                    ClassAtom::Item(item) => {
                        items.push(Item::Range(low, low));
                        items.push(Item::Range('-', '-'));
                        items.push(item);
                    }
                }
            } else {
                items.push(Item::Range(low, low));
            }
        }
        Some(Class { negated, items })
    }

    fn class_atom(&mut self) -> Option<ClassAtom> {
        let c = self.peek()?;
        self.at += 1;
        if c != '\\' {
            return Some(ClassAtom::Char(c));
        }
        let e = self.peek()?;
        Some(match e {
            'd' | 'D' => {
                self.at += 1;
                ClassAtom::Item(Item::Digit(e == 'D'))
            }
            'w' | 'W' => {
                self.at += 1;
                ClassAtom::Item(Item::Word(e == 'W'))
            }
            's' | 'S' => {
                self.at += 1;
                ClassAtom::Item(Item::Space(e == 'S'))
            }
            'b' => {
                self.at += 1;
                ClassAtom::Char('\u{8}')
            }
            _ => ClassAtom::Char(self.escaped_char()?),
        })
    }
}

enum ClassAtom {
    Item(Item),
    Char(char),
}

impl Class {
    fn of(item: Item) -> Class {
        Class {
            negated: false,
            items: vec![item],
        }
    }

    fn matches(&self, c: char, ignore_case: bool) -> bool {
        let hit = |c: char| self.items.iter().any(|item| item.matches(c));
        let found = hit(c) || (ignore_case && (hit(lower(c)) || hit(upper(c))));
        found != self.negated
    }
}

impl Item {
    fn matches(self, c: char) -> bool {
        match self {
            Item::Range(a, b) => a <= c && c <= b,
            Item::Digit(neg) => c.is_ascii_digit() != neg,
            Item::Word(neg) => is_word(c) != neg,
            Item::Space(neg) => is_space(c) != neg,
        }
    }
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// JavaScript's `\s`: white space and line terminators.
fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

fn is_line_end(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

fn lower(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(one), None) => one,
        _ => c,
    }
}

fn upper(c: char) -> char {
    let mut u = c.to_uppercase();
    match (u.next(), u.next()) {
        (Some(one), None) => one,
        _ => c,
    }
}

// --------------------------------------------------------------- matching

struct Matcher<'r, 't> {
    re: &'r Regex,
    text: &'t [char],
    steps: Cell<usize>,
    depth: Cell<usize>,
    exhausted: Cell<bool>,
}

type Cont<'k> = dyn FnMut(usize, &mut Captures) -> bool + 'k;

impl Matcher<'_, '_> {
    /// One step off the budget; false (and exhausted) past it.
    fn step(&self) -> bool {
        if self.exhausted.get() {
            return false;
        }
        match self.steps.get().checked_sub(1) {
            Some(left) => {
                self.steps.set(left);
                true
            }
            None => {
                self.exhausted.set(true);
                false
            }
        }
    }

    /// Whether the single-char node `node` matches the char at `pos`
    /// (`None` when `node` is not a single-char node).
    fn one(&self, node: &Node, pos: usize) -> Option<bool> {
        let c = self.text.get(pos).copied();
        Some(match node {
            Node::Char(want) => {
                c.is_some_and(|c| c == *want || (self.re.ignore_case && lower(c) == lower(*want)))
            }
            Node::Any => c.is_some_and(|c| !is_line_end(c)),
            Node::Class(class) => c.is_some_and(|c| class.matches(c, self.re.ignore_case)),
            _ => return None,
        })
    }

    fn at_start(&self, pos: usize) -> bool {
        pos == 0 || (self.re.multiline && is_line_end(self.text[pos - 1]))
    }

    fn at_end(&self, pos: usize) -> bool {
        pos == self.text.len() || (self.re.multiline && is_line_end(self.text[pos]))
    }

    fn word_at(&self, pos: usize) -> bool {
        self.text.get(pos).is_some_and(|c| is_word(*c))
    }

    /// Matches `nodes` in turn from `pos`, then `k` with where they ended.
    fn seq(&self, nodes: &[Node], mut pos: usize, caps: &mut Captures, k: &mut Cont<'_>) -> bool {
        let depth = self.depth.get() + 1;
        if depth > MAX_DEPTH {
            self.exhausted.set(true);
            return false;
        }
        self.depth.set(depth);
        let found = self.seq_inner(nodes, &mut pos, caps, k);
        self.depth.set(depth - 1);
        found
    }

    fn seq_inner(
        &self,
        mut nodes: &[Node],
        pos: &mut usize,
        caps: &mut Captures,
        k: &mut Cont<'_>,
    ) -> bool {
        // Single-char nodes and assertions in a row, without recursing.
        loop {
            if !self.step() {
                return false;
            }
            let Some((first, rest)) = nodes.split_first() else {
                return k(*pos, caps);
            };
            match first {
                Node::Empty => {}
                Node::Start => {
                    if !self.at_start(*pos) {
                        return false;
                    }
                }
                Node::End => {
                    if !self.at_end(*pos) {
                        return false;
                    }
                }
                Node::WordBoundary(want) => {
                    let edge = (*pos > 0 && self.word_at(*pos - 1)) != self.word_at(*pos);
                    if edge != *want {
                        return false;
                    }
                }
                Node::Concat(items) => {
                    let mut all: Vec<Node> = items.clone();
                    all.extend_from_slice(rest);
                    return self.seq(&all, *pos, caps, k);
                }
                node => match self.one(node, *pos) {
                    Some(true) => *pos += 1,
                    Some(false) => return false,
                    None => return self.complex(first, rest, *pos, caps, k),
                },
            }
            nodes = rest;
        }
    }

    fn complex(
        &self,
        node: &Node,
        rest: &[Node],
        pos: usize,
        caps: &mut Captures,
        k: &mut Cont<'_>,
    ) -> bool {
        match node {
            Node::Group(inner, index) => {
                let saved = index.map(|i| caps[i]);
                let found = self.seq(
                    std::slice::from_ref(&**inner),
                    pos,
                    caps,
                    &mut |end, caps| {
                        if let Some(i) = index {
                            caps[*i] = Some((pos, end));
                        }
                        self.seq(rest, end, caps, k)
                    },
                );
                if !found && let (Some(i), Some(old)) = (index, saved) {
                    caps[*i] = old;
                }
                found
            }
            Node::Alt(branches) => {
                for branch in branches {
                    let snapshot = caps.clone();
                    if self.seq(std::slice::from_ref(branch), pos, caps, &mut |end, caps| {
                        self.seq(rest, end, caps, k)
                    }) {
                        return true;
                    }
                    *caps = snapshot;
                    if self.exhausted.get() {
                        return false;
                    }
                }
                false
            }
            Node::Look(inner, negated) => {
                let mut trial = caps.clone();
                let hit = self.seq(
                    std::slice::from_ref(&**inner),
                    pos,
                    &mut trial,
                    &mut |_, _| true,
                );
                if self.exhausted.get() || hit == *negated {
                    return false;
                }
                if !negated {
                    *caps = trial;
                }
                self.seq(rest, pos, caps, k)
            }
            Node::BackRef(i) => {
                let Some((a, b)) = caps.get(*i).copied().flatten() else {
                    // A group that did not take part matches empty.
                    return self.seq(rest, pos, caps, k);
                };
                let len = b - a;
                if pos + len > self.text.len() {
                    return false;
                }
                let same = (0..len).all(|j| {
                    let (x, y) = (self.text[a + j], self.text[pos + j]);
                    x == y || (self.re.ignore_case && lower(x) == lower(y))
                });
                same && self.seq(rest, pos + len, caps, k)
            }
            Node::Repeat {
                node,
                min,
                max,
                greedy,
            } => {
                if self.one(node, pos).is_some() {
                    self.repeat_one(node, *min, *max, *greedy, rest, pos, caps, k)
                } else {
                    self.repeat(node, *min, *max, *greedy, 0, rest, pos, caps, k)
                }
            }
            // Reached only for the kinds `seq_inner` handles itself.
            _ => false,
        }
    }

    /// A repeat of one char: counts how many match, then tries the rest
    /// from each count in turn (most first when greedy), in a loop.
    #[allow(clippy::too_many_arguments)]
    fn repeat_one(
        &self,
        node: &Node,
        min: u32,
        max: Option<u32>,
        greedy: bool,
        rest: &[Node],
        pos: usize,
        caps: &mut Captures,
        k: &mut Cont<'_>,
    ) -> bool {
        let limit = max.map_or(usize::MAX, |m| m as usize);
        let mut n = 0usize;
        while n < limit && self.one(node, pos + n) == Some(true) {
            if !self.step() {
                return false;
            }
            n += 1;
        }
        let min = min as usize;
        if n < min {
            return false;
        }
        let counts: Box<dyn Iterator<Item = usize>> = if greedy {
            Box::new((min..=n).rev())
        } else {
            Box::new(min..=n)
        };
        for count in counts {
            if self.seq(rest, pos + count, caps, k) {
                return true;
            }
            if self.exhausted.get() {
                return false;
            }
        }
        false
    }

    /// A repeat of anything else, one iteration per recursion; an
    /// iteration that matches empty ends the repeat (as JavaScript does).
    #[allow(clippy::too_many_arguments)]
    fn repeat(
        &self,
        node: &Node,
        min: u32,
        max: Option<u32>,
        greedy: bool,
        count: u32,
        rest: &[Node],
        pos: usize,
        caps: &mut Captures,
        k: &mut Cont<'_>,
    ) -> bool {
        if count < min {
            return self.repeat_more(node, min, max, greedy, count, rest, pos, caps, k);
        }
        if greedy {
            let snapshot = caps.clone();
            if self.repeat_more(node, min, max, greedy, count, rest, pos, caps, k) {
                return true;
            }
            *caps = snapshot;
            !self.exhausted.get() && self.seq(rest, pos, caps, k)
        } else {
            if self.seq(rest, pos, caps, k) {
                return true;
            }
            !self.exhausted.get()
                && self.repeat_more(node, min, max, greedy, count, rest, pos, caps, k)
        }
    }

    /// One more iteration of `repeat`'s node, then the repeat again.
    #[allow(clippy::too_many_arguments)]
    fn repeat_more(
        &self,
        node: &Node,
        min: u32,
        max: Option<u32>,
        greedy: bool,
        count: u32,
        rest: &[Node],
        pos: usize,
        caps: &mut Captures,
        k: &mut Cont<'_>,
    ) -> bool {
        if max.is_some_and(|m| count >= m) {
            return false;
        }
        self.seq(std::slice::from_ref(node), pos, caps, &mut |end, caps| {
            (end != pos || count < min)
                && self.repeat(node, min, max, greedy, count + 1, rest, end, caps, k)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(pattern: &str, flags: &str, text: &str) -> Option<String> {
        let re = Regex::new(pattern, flags).unwrap_or_else(|| panic!("{pattern} reads"));
        let chars: Vec<char> = text.chars().collect();
        let mut steps = 1_000_000;
        re.find(&chars, 0, &mut steps).unwrap().map(|caps| {
            chars[caps[0].unwrap().0..caps[0].unwrap().1]
                .iter()
                .collect()
        })
    }

    #[test]
    fn the_status_certificate_phone_patterns() {
        let plain = r"^[0-9]{3}[\s-]?[0-9]{3}[\s-]?[0-9]{4}$";
        let bracketed = r"^\({1}[0-9]{3}\){1}\s?[0-9]{3}[\s-]?[0-9]{4}$";
        for ok in ["6135551234", "613-555-1234", "613 555 1234"] {
            assert!(find(plain, "", ok).is_some(), "{ok}");
        }
        assert!(find(bracketed, "", "(613) 555-1234").is_some());
        for bad in ["613555123", "abc", "613-555-12345", "(613 555-1234"] {
            assert!(find(plain, "", bad).is_none(), "{bad}");
            assert!(find(bracketed, "", bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn classes_groups_alternation_and_quantifiers() {
        assert_eq!(find("a(b|c)+d", "", "xxabcbd").as_deref(), Some("abcbd"));
        assert_eq!(find("a.*?b", "", "aXbYb").as_deref(), Some("aXb"));
        assert_eq!(find("a.*b", "", "aXbYb").as_deref(), Some("aXbYb"));
        assert_eq!(find(r"\d+", "", "ab 123 c").as_deref(), Some("123"));
        assert_eq!(find(r"[^a-c]+", "", "abcxyzabc").as_deref(), Some("xyz"));
        assert_eq!(find(r"\bcat\b", "", "concat cat").as_deref(), Some("cat"));
        assert_eq!(find("HELLO", "i", "say hello").as_deref(), Some("hello"));
        assert_eq!(find(r"(\w)\1", "", "abccd").as_deref(), Some("cc"));
        assert_eq!(find("x(?=y)", "", "xzxy").as_deref(), Some("x"));
        assert_eq!(find("x(?!y)", "", "xyxz").as_deref(), Some("x"));
        assert_eq!(find("^b", "m", "a\nb").as_deref(), Some("b"));
        assert_eq!(find("^b", "", "a\nb"), None);
        assert_eq!(
            find("(a|ab)(c|bcd)(d*)", "", "abcd").as_deref(),
            Some("abcd")
        );
        assert_eq!(find("(?:ab){2}", "", "ababab").as_deref(), Some("abab"));
        assert_eq!(find("(a*)*b", "", "aaab").as_deref(), Some("aaab"));
        assert_eq!(find(r"A\x42", "", "zAB").as_deref(), Some("AB"));
        assert_eq!(find("a{,2}", "", "a{,2}").as_deref(), Some("a{,2}"));
    }

    #[test]
    fn what_does_not_read_is_refused() {
        for bad in ["(", "a)", "[a", "*a", "a{2,1}", "(?<n>a)", "(?<=a)b", r"\k"] {
            assert!(Regex::new(bad, "").is_none(), "{bad}");
        }
        assert!(Regex::new("a", "gg").is_none());
        assert!(Regex::new("a", "u").is_none());
        assert!(Regex::new(&"a".repeat(MAX_PATTERN + 1), "").is_none());
    }

    #[test]
    fn a_runaway_match_stops_at_its_budget() {
        // Catastrophic backtracking: nested repeats over a long miss.
        let re = Regex::new("(a+)+$", "").unwrap();
        let text: Vec<char> = "a".repeat(40).chars().chain(['b']).collect();
        let mut steps = 100_000;
        assert_eq!(re.find(&text, 0, &mut steps), Err(Exhausted));
        // Deep recursion is refused, not a stack overflow.
        let re = Regex::new("(?:a|b)*c", "").unwrap();
        let text: Vec<char> = "ab".repeat(5_000).chars().collect();
        let mut steps = usize::MAX;
        assert_eq!(re.find(&text, 0, &mut steps), Err(Exhausted));
    }
}
