// SPDX-License-Identifier: MIT
//! A reader for the JavaScript that Designer forms carry: tokens, then a
//! syntax tree for the subset `eval` runs (statements: `var`, `if`,
//! `switch`, `while`, `do`, `for`, `return`, `break`, `continue`, blocks,
//! function declarations, expression statements; expressions: literals,
//! names, `this`, member access, calls, `new`, the arithmetic, comparison
//! and logical operators, assignment and the conditional operator).
//!
//! Rules:
//! - Reading never fails. A statement the reader cannot follow becomes
//!   `Stmt::Skipped` and the reader resumes at the next statement (the
//!   next `;`, line end or `}` outside any bracket), so one unknown
//!   construct costs the statement that holds it, not the script.
//! - A missing `;` at a line end, before `}` or at the end ends a
//!   statement, as JavaScript's semicolon insertion does.
//! - Regular expression literals keep their pattern and flags (`regex`
//!   runs them). `try`/`catch`, `throw`, `for ... in`, labels, `in`,
//!   `instanceof`, `delete`, `void` and the bit operators are read enough
//!   to keep the token stream in step; none runs.
//! - Nesting of expressions, blocks and functions is bounded by
//!   `MAX_DEPTH`; deeper input is skipped as unknown.

use std::rc::Rc;

/// Expression and block nesting the reader follows, at most.
pub const MAX_DEPTH: usize = 48;

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Ident(String),
    Num(f64),
    Str(String),
    Punct(&'static str),
    /// A regular expression literal: its pattern and flags.
    Regex(String, String),
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub tok: Tok,
    /// A line end lies between this token and the one before it.
    pub newline_before: bool,
}

/// Punctuators, longest first.
const PUNCTS: &[&str] = &[
    ">>>=", "===", "!==", ">>>", "<<=", ">>=", "==", "!=", "<=", ">=", "&&", "||", "++", "--",
    "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>", "(", ")", "{", "}", "[", "]", ";",
    ",", ".", "?", ":", "=", "+", "-", "*", "/", "%", "!", "<", ">", "&", "|", "^", "~",
];

const KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "continue",
    "default",
    "delete",
    "do",
    "else",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
];

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || c == '$'
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '$'
}

/// The tokens of `src`. Whitespace and comments are dropped; an unknown
/// character is skipped.
pub fn lex(src: &str) -> Vec<Token> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut newline = false;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            newline = true;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                if chars[i] == '\n' {
                    newline = true;
                }
                i += 1;
            }
            i = (i + 2).min(chars.len());
            continue;
        }
        let tok = if is_ident_start(c) {
            let start = i;
            while i < chars.len() && is_ident_char(chars[i]) {
                i += 1;
            }
            Tok::Ident(chars[start..i].iter().collect())
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            Tok::Num(lex_number(&chars, &mut i))
        } else if c == '"' || c == '\'' {
            Tok::Str(lex_string(&chars, &mut i))
        } else if c == '/' && regex_allowed(out.last()) {
            let (pattern, flags) = lex_regex(&chars, &mut i);
            Tok::Regex(pattern, flags)
        } else {
            match PUNCTS.iter().find(|p| {
                chars[i..]
                    .iter()
                    .zip(p.chars())
                    .filter(|(a, b)| *a == b)
                    .count()
                    == p.len()
            }) {
                Some(p) => {
                    i += p.len();
                    Tok::Punct(p)
                }
                None => {
                    i += 1;
                    continue;
                }
            }
        };
        out.push(Token {
            tok,
            newline_before: newline,
        });
        newline = false;
    }
    out.push(Token {
        tok: Tok::Eof,
        newline_before: true,
    });
    out
}

/// Whether a `/` here starts a regular expression: after anything but a
/// value (a name, number, string or closing bracket) it does.
fn regex_allowed(previous: Option<&Token>) -> bool {
    match previous.map(|t| &t.tok) {
        None => true,
        Some(Tok::Ident(name)) => KEYWORDS.contains(&name.as_str()) && name != "this",
        Some(Tok::Num(_) | Tok::Str(_) | Tok::Regex(..)) => false,
        Some(Tok::Punct(p)) => !matches!(*p, ")" | "]" | "}"),
        Some(Tok::Eof) => true,
    }
}

fn lex_number(chars: &[char], i: &mut usize) -> f64 {
    let start = *i;
    if chars[*i] == '0' && matches!(chars.get(*i + 1), Some('x' | 'X')) {
        *i += 2;
        let digits = *i;
        while *i < chars.len() && chars[*i].is_ascii_hexdigit() {
            *i += 1;
        }
        let text: String = chars[digits..*i].iter().collect();
        return u64::from_str_radix(&text, 16).map_or(f64::NAN, |v| v as f64);
    }
    while *i < chars.len() && chars[*i].is_ascii_digit() {
        *i += 1;
    }
    if chars.get(*i) == Some(&'.') {
        *i += 1;
        while *i < chars.len() && chars[*i].is_ascii_digit() {
            *i += 1;
        }
    }
    if matches!(chars.get(*i), Some('e' | 'E')) {
        let mut j = *i + 1;
        if matches!(chars.get(j), Some('+' | '-')) {
            j += 1;
        }
        if chars.get(j).is_some_and(|d| d.is_ascii_digit()) {
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            *i = j;
        }
    }
    let text: String = chars[start..*i].iter().collect();
    text.parse().unwrap_or(f64::NAN)
}

fn lex_string(chars: &[char], i: &mut usize) -> String {
    let quote = chars[*i];
    *i += 1;
    let mut out = String::new();
    while *i < chars.len() {
        let c = chars[*i];
        *i += 1;
        if c == quote {
            break;
        }
        if c == '\n' {
            break;
        }
        if c != '\\' {
            out.push(c);
            continue;
        }
        let Some(&e) = chars.get(*i) else {
            break;
        };
        *i += 1;
        match e {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            'v' => out.push('\u{b}'),
            '0' => out.push('\0'),
            '\n' => {}
            'x' | 'u' => {
                let len = if e == 'x' { 2 } else { 4 };
                let hex: String = chars[*i..(*i + len).min(chars.len())].iter().collect();
                match u32::from_str_radix(&hex, 16)
                    .ok()
                    .filter(|_| hex.len() == len)
                    .and_then(char::from_u32)
                {
                    Some(c) => {
                        out.push(c);
                        *i += len;
                    }
                    None => out.push(e),
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// A regular expression literal at `i` (its opening `/`): the pattern
/// between the slashes and the flags after.
fn lex_regex(chars: &[char], i: &mut usize) -> (String, String) {
    *i += 1;
    let start = *i;
    let mut end = None;
    let mut in_class = false;
    while *i < chars.len() {
        let c = chars[*i];
        *i += 1;
        match c {
            '\\' => *i += 1,
            '[' => in_class = true,
            ']' => in_class = false,
            '/' if !in_class => {
                end = Some(*i - 1);
                break;
            }
            '\n' => break,
            _ => {}
        }
    }
    let pattern: String = chars[start..end.unwrap_or((*i).min(chars.len()))]
        .iter()
        .collect();
    let flags_start = *i;
    while *i < chars.len() && is_ident_char(chars[*i]) {
        *i += 1;
    }
    let flags = if end.is_some() {
        chars[flags_start..*i].iter().collect()
    } else {
        // Unterminated: a flag no pattern has, so it never runs.
        "!".to_string()
    };
    (pattern, flags)
}

// ----------------------------------------------------------------- syntax

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    Lt,
    Gt,
    Le,
    Ge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
    Plus,
    TypeOf,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
    Undefined,
    This,
    Ident(String),
    Member(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    New(Box<Expr>, Vec<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `&&` (true) or `||` (false).
    Logical(bool, Box<Expr>, Box<Expr>),
    /// `target = value`, or `target op= value` with the operator.
    Assign(Option<BinOp>, Box<Expr>, Box<Expr>),
    /// `++`/`--`: prefix?, increment?, target.
    Update(bool, bool, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Function(Rc<Function>),
    /// A regular expression literal: pattern and flags.
    Regex(String, String),
    /// A construct that is read but not run (an array or object literal,
    /// a bit operator, ...): `undefined`.
    Unsupported,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Expr(Expr),
    Var(Vec<(String, Option<Expr>)>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    Block(Vec<Stmt>),
    /// The cases in order: `None` is `default`.
    Switch(Expr, Vec<(Option<Expr>, Vec<Stmt>)>),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    For(Option<Box<Stmt>>, Option<Expr>, Option<Expr>, Box<Stmt>),
    Return(Option<Expr>),
    Break,
    Continue,
    Function(Rc<Function>),
    Empty,
    /// A statement the reader could not follow.
    Skipped,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}

/// Reads a script. Never fails: see the module notes.
pub fn parse(src: &str) -> Vec<Stmt> {
    let mut parser = Parser {
        tokens: lex(src),
        at: 0,
        depth: 0,
    };
    let mut out = Vec::new();
    while !parser.at_eof() {
        out.push(parser.statement_or_skip());
    }
    out
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
    depth: usize,
}

struct Unsupported;

type Parsed<T> = Result<T, Unsupported>;

impl Parser {
    fn peek(&self) -> &Tok {
        &self.tokens[self.at.min(self.tokens.len() - 1)].tok
    }
    fn peek_at(&self, ahead: usize) -> &Tok {
        &self.tokens[(self.at + ahead).min(self.tokens.len() - 1)].tok
    }
    fn newline_before(&self) -> bool {
        self.tokens[self.at.min(self.tokens.len() - 1)].newline_before
    }
    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }
    fn advance(&mut self) -> Tok {
        let tok = self.peek().clone();
        if self.at < self.tokens.len() - 1 {
            self.at += 1;
        }
        tok
    }
    fn is_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }
    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Tok::Ident(q) if q == w)
    }
    fn eat_punct(&mut self, p: &str) -> bool {
        if self.is_punct(p) {
            self.advance();
            true
        } else {
            false
        }
    }
    fn eat_word(&mut self, w: &str) -> bool {
        if self.is_word(w) {
            self.advance();
            true
        } else {
            false
        }
    }
    fn expect_punct(&mut self, p: &str) -> Parsed<()> {
        if self.eat_punct(p) {
            Ok(())
        } else {
            Err(Unsupported)
        }
    }
    fn ident(&mut self) -> Parsed<String> {
        match self.peek() {
            Tok::Ident(name) if !KEYWORDS.contains(&name.as_str()) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => Err(Unsupported),
        }
    }
    fn enter(&mut self) -> Parsed<()> {
        if self.depth >= MAX_DEPTH {
            return Err(Unsupported);
        }
        self.depth += 1;
        Ok(())
    }
    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// The end of a statement: `;`, or (inserted) a line end, `}` or the
    /// end of the script.
    fn end_statement(&mut self) -> Parsed<()> {
        if self.eat_punct(";") || self.is_punct("}") || self.at_eof() || self.newline_before() {
            Ok(())
        } else {
            Err(Unsupported)
        }
    }

    fn statement_or_skip(&mut self) -> Stmt {
        let start = self.at;
        let depth = self.depth;
        match self.statement() {
            Ok(stmt) => stmt,
            Err(Unsupported) => {
                self.depth = depth;
                self.at = start;
                self.skip_statement();
                Stmt::Skipped
            }
        }
    }

    /// Past the statement at the cursor: to the next `;` (taken), a line
    /// end or a `}` outside any bracket opened here (left), or the end.
    fn skip_statement(&mut self) {
        let mut depth = 0usize;
        let mut first = true;
        loop {
            if self.at_eof() {
                return;
            }
            if !first && depth == 0 && self.newline_before() {
                return;
            }
            first = false;
            match self.peek() {
                Tok::Punct("(" | "[" | "{") => depth += 1,
                Tok::Punct(")" | "]") => depth = depth.saturating_sub(1),
                Tok::Punct("}") => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    if depth == 0 {
                        self.advance();
                        return;
                    }
                }
                Tok::Punct(";") if depth == 0 => {
                    self.advance();
                    return;
                }
                _ => {}
            }
            self.advance();
        }
    }

    fn statement(&mut self) -> Parsed<Stmt> {
        self.enter()?;
        let result = self.statement_inner();
        self.leave();
        result
    }

    fn statement_inner(&mut self) -> Parsed<Stmt> {
        if self.eat_punct(";") {
            return Ok(Stmt::Empty);
        }
        if self.is_punct("{") {
            return Ok(Stmt::Block(self.block()?));
        }
        if let Tok::Ident(word) = self.peek() {
            match word.as_str() {
                "var" => {
                    self.advance();
                    let decls = self.var_declarations()?;
                    self.end_statement()?;
                    return Ok(Stmt::Var(decls));
                }
                "if" => {
                    self.advance();
                    self.expect_punct("(")?;
                    let cond = self.expression()?;
                    self.expect_punct(")")?;
                    let then = self.statement_or_skip();
                    let otherwise = if self.eat_word("else") {
                        Some(Box::new(self.statement_or_skip()))
                    } else {
                        None
                    };
                    return Ok(Stmt::If(cond, Box::new(then), otherwise));
                }
                "switch" => {
                    self.advance();
                    self.expect_punct("(")?;
                    let value = self.expression()?;
                    self.expect_punct(")")?;
                    self.expect_punct("{")?;
                    let mut cases = Vec::new();
                    while !self.eat_punct("}") {
                        if self.at_eof() {
                            return Err(Unsupported);
                        }
                        let label = if self.eat_word("default") {
                            None
                        } else if self.eat_word("case") {
                            Some(self.expression()?)
                        } else {
                            return Err(Unsupported);
                        };
                        self.expect_punct(":")?;
                        let mut body = Vec::new();
                        while !(self.is_punct("}")
                            || self.is_word("case")
                            || self.is_word("default")
                            || self.at_eof())
                        {
                            body.push(self.statement_or_skip());
                        }
                        cases.push((label, body));
                    }
                    return Ok(Stmt::Switch(value, cases));
                }
                "while" => {
                    self.advance();
                    self.expect_punct("(")?;
                    let cond = self.expression()?;
                    self.expect_punct(")")?;
                    let body = self.statement_or_skip();
                    return Ok(Stmt::While(cond, Box::new(body)));
                }
                "do" => {
                    self.advance();
                    let body = self.statement_or_skip();
                    if !self.eat_word("while") {
                        return Err(Unsupported);
                    }
                    self.expect_punct("(")?;
                    let cond = self.expression()?;
                    self.expect_punct(")")?;
                    self.eat_punct(";");
                    return Ok(Stmt::DoWhile(Box::new(body), cond));
                }
                "for" => {
                    self.advance();
                    self.expect_punct("(")?;
                    let init = if self.is_punct(";") {
                        None
                    } else if self.eat_word("var") {
                        Some(Box::new(Stmt::Var(self.var_declarations()?)))
                    } else {
                        Some(Box::new(Stmt::Expr(self.expression()?)))
                    };
                    self.expect_punct(";")?;
                    let cond = if self.is_punct(";") {
                        None
                    } else {
                        Some(self.expression()?)
                    };
                    self.expect_punct(";")?;
                    let update = if self.is_punct(")") {
                        None
                    } else {
                        Some(self.expression()?)
                    };
                    self.expect_punct(")")?;
                    let body = self.statement_or_skip();
                    return Ok(Stmt::For(init, cond, update, Box::new(body)));
                }
                "return" => {
                    self.advance();
                    let value = if self.is_punct(";")
                        || self.is_punct("}")
                        || self.at_eof()
                        || self.newline_before()
                    {
                        None
                    } else {
                        Some(self.expression()?)
                    };
                    self.end_statement()?;
                    return Ok(Stmt::Return(value));
                }
                "break" | "continue" => {
                    let stmt = if word == "break" {
                        Stmt::Break
                    } else {
                        Stmt::Continue
                    };
                    self.advance();
                    // A label is ignored.
                    if !self.newline_before() {
                        if let Tok::Ident(_) = self.peek() {
                            self.advance();
                        }
                    }
                    self.end_statement()?;
                    return Ok(stmt);
                }
                "function" if matches!(self.peek_at(1), Tok::Ident(_)) => {
                    self.advance();
                    let function = self.function_rest()?;
                    return Ok(Stmt::Function(Rc::new(function)));
                }
                "try" => {
                    self.advance();
                    let body = self.block()?;
                    if self.eat_word("catch") {
                        self.expect_punct("(")?;
                        self.ident()?;
                        self.expect_punct(")")?;
                        self.block()?;
                    }
                    if self.eat_word("finally") {
                        self.block()?;
                    }
                    return Ok(Stmt::Block(body));
                }
                "throw" => {
                    self.advance();
                    self.expression()?;
                    self.end_statement()?;
                    return Ok(Stmt::Skipped);
                }
                _ => {}
            }
        }
        let expr = self.expression()?;
        self.end_statement()?;
        Ok(Stmt::Expr(expr))
    }

    fn block(&mut self) -> Parsed<Vec<Stmt>> {
        self.expect_punct("{")?;
        self.enter()?;
        let mut out = Vec::new();
        while !self.eat_punct("}") {
            if self.at_eof() {
                self.leave();
                return Err(Unsupported);
            }
            out.push(self.statement_or_skip());
        }
        self.leave();
        Ok(out)
    }

    fn var_declarations(&mut self) -> Parsed<Vec<(String, Option<Expr>)>> {
        let mut out = Vec::new();
        loop {
            let name = self.ident()?;
            let value = if self.eat_punct("=") {
                Some(self.assignment()?)
            } else {
                None
            };
            out.push((name, value));
            if !self.eat_punct(",") {
                return Ok(out);
            }
        }
    }

    /// After `function`: an optional name, the parameters and the body.
    fn function_rest(&mut self) -> Parsed<Function> {
        let name = self.ident().unwrap_or_default();
        self.expect_punct("(")?;
        let mut params = Vec::new();
        if !self.is_punct(")") {
            loop {
                params.push(self.ident()?);
                if !self.eat_punct(",") {
                    break;
                }
            }
        }
        self.expect_punct(")")?;
        let body = self.block()?;
        Ok(Function { name, params, body })
    }

    fn expression(&mut self) -> Parsed<Expr> {
        self.enter()?;
        let result = self.assignment();
        self.leave();
        result
    }

    fn assignment(&mut self) -> Parsed<Expr> {
        let left = self.conditional()?;
        let op = match self.peek() {
            Tok::Punct("=") => None,
            Tok::Punct("+=") => Some(BinOp::Add),
            Tok::Punct("-=") => Some(BinOp::Sub),
            Tok::Punct("*=") => Some(BinOp::Mul),
            Tok::Punct("/=") => Some(BinOp::Div),
            Tok::Punct("%=") => Some(BinOp::Rem),
            Tok::Punct("&=" | "|=" | "^=" | "<<=" | ">>=" | ">>>=") => return Err(Unsupported),
            _ => return Ok(left),
        };
        if !matches!(left, Expr::Ident(_) | Expr::Member(..) | Expr::Index(..)) {
            return Err(Unsupported);
        }
        self.advance();
        self.enter()?;
        let right = self.assignment();
        self.leave();
        Ok(Expr::Assign(op, Box::new(left), Box::new(right?)))
    }

    fn conditional(&mut self) -> Parsed<Expr> {
        let cond = self.binary(0)?;
        if !self.eat_punct("?") {
            return Ok(cond);
        }
        self.enter()?;
        let result = (|| {
            let then = self.assignment()?;
            self.expect_punct(":")?;
            let otherwise = self.assignment()?;
            Ok(Expr::Cond(
                Box::new(cond),
                Box::new(then),
                Box::new(otherwise),
            ))
        })();
        self.leave();
        result
    }

    /// Precedence climbing over the binary operators (`in` and
    /// `instanceof` read as unsupported).
    fn binary(&mut self, min: u8) -> Parsed<Expr> {
        let mut left = self.unary()?;
        loop {
            let (prec, kind) = match self.peek() {
                Tok::Punct("||") => (1, Kind::Or),
                Tok::Punct("&&") => (2, Kind::And),
                Tok::Punct("|" | "^" | "&" | "<<" | ">>" | ">>>") => (3, Kind::Bits),
                Tok::Punct("==") => (6, Kind::Bin(BinOp::Eq)),
                Tok::Punct("!=") => (6, Kind::Bin(BinOp::Ne)),
                Tok::Punct("===") => (6, Kind::Bin(BinOp::StrictEq)),
                Tok::Punct("!==") => (6, Kind::Bin(BinOp::StrictNe)),
                Tok::Punct("<") => (7, Kind::Bin(BinOp::Lt)),
                Tok::Punct(">") => (7, Kind::Bin(BinOp::Gt)),
                Tok::Punct("<=") => (7, Kind::Bin(BinOp::Le)),
                Tok::Punct(">=") => (7, Kind::Bin(BinOp::Ge)),
                Tok::Ident(w) if w == "in" || w == "instanceof" => (7, Kind::Bits),
                Tok::Punct("+") => (9, Kind::Bin(BinOp::Add)),
                Tok::Punct("-") => (9, Kind::Bin(BinOp::Sub)),
                Tok::Punct("*") => (10, Kind::Bin(BinOp::Mul)),
                Tok::Punct("/") => (10, Kind::Bin(BinOp::Div)),
                Tok::Punct("%") => (10, Kind::Bin(BinOp::Rem)),
                _ => return Ok(left),
            };
            if prec < min {
                return Ok(left);
            }
            self.advance();
            self.enter()?;
            let right = self.binary(prec + 1);
            self.leave();
            let right = right?;
            left = match kind {
                Kind::Or => Expr::Logical(false, Box::new(left), Box::new(right)),
                Kind::And => Expr::Logical(true, Box::new(left), Box::new(right)),
                Kind::Bin(op) => Expr::Binary(op, Box::new(left), Box::new(right)),
                Kind::Bits => Expr::Unsupported,
            };
        }
    }

    fn unary(&mut self) -> Parsed<Expr> {
        let op = match self.peek() {
            Tok::Punct("!") => Some(UnOp::Not),
            Tok::Punct("-") => Some(UnOp::Neg),
            Tok::Punct("+") => Some(UnOp::Plus),
            Tok::Ident(w) if w == "typeof" => Some(UnOp::TypeOf),
            Tok::Ident(w) if w == "void" || w == "delete" => {
                self.advance();
                self.enter()?;
                let operand = self.unary();
                self.leave();
                operand?;
                return Ok(Expr::Unsupported);
            }
            Tok::Punct("~") => {
                self.advance();
                self.enter()?;
                let operand = self.unary();
                self.leave();
                operand?;
                return Ok(Expr::Unsupported);
            }
            Tok::Punct(p @ ("++" | "--")) => {
                let inc = *p == "++";
                self.advance();
                self.enter()?;
                let target = self.unary();
                self.leave();
                return Ok(Expr::Update(true, inc, Box::new(target?)));
            }
            _ => None,
        };
        if let Some(op) = op {
            self.advance();
            self.enter()?;
            let operand = self.unary();
            self.leave();
            return Ok(Expr::Unary(op, Box::new(operand?)));
        }
        let expr = self.postfix()?;
        if !self.newline_before() {
            if let Tok::Punct(p @ ("++" | "--")) = self.peek() {
                let inc = *p == "++";
                self.advance();
                return Ok(Expr::Update(false, inc, Box::new(expr)));
            }
        }
        Ok(expr)
    }

    fn postfix(&mut self) -> Parsed<Expr> {
        let mut expr = self.primary()?;
        loop {
            if self.eat_punct(".") {
                let name = match self.advance() {
                    Tok::Ident(name) => name,
                    _ => return Err(Unsupported),
                };
                expr = Expr::Member(Box::new(expr), name);
            } else if self.eat_punct("[") {
                self.enter()?;
                let index = self.expression();
                self.leave();
                let index = index?;
                self.expect_punct("]")?;
                expr = Expr::Index(Box::new(expr), Box::new(index));
            } else if self.is_punct("(") {
                let args = self.arguments()?;
                expr = Expr::Call(Box::new(expr), args);
            } else {
                return Ok(expr);
            }
        }
    }

    fn arguments(&mut self) -> Parsed<Vec<Expr>> {
        self.expect_punct("(")?;
        self.enter()?;
        let result = (|| {
            let mut args = Vec::new();
            if self.eat_punct(")") {
                return Ok(args);
            }
            loop {
                args.push(self.assignment()?);
                if self.eat_punct(")") {
                    return Ok(args);
                }
                self.expect_punct(",")?;
            }
        })();
        self.leave();
        result
    }

    fn primary(&mut self) -> Parsed<Expr> {
        match self.advance() {
            Tok::Num(n) => Ok(Expr::Num(n)),
            Tok::Str(s) => Ok(Expr::Str(s)),
            Tok::Regex(pattern, flags) => Ok(Expr::Regex(pattern, flags)),
            Tok::Punct("(") => {
                self.enter()?;
                let inner = self.expression();
                self.leave();
                let inner = inner?;
                self.expect_punct(")")?;
                Ok(inner)
            }
            Tok::Punct("[") => {
                self.enter()?;
                let result = (|| {
                    while !self.eat_punct("]") {
                        if self.at_eof() {
                            return Err(Unsupported);
                        }
                        if !self.eat_punct(",") {
                            self.assignment()?;
                        }
                    }
                    Ok(Expr::Unsupported)
                })();
                self.leave();
                result
            }
            Tok::Punct("{") => {
                self.enter()?;
                let result = (|| {
                    while !self.eat_punct("}") {
                        if self.at_eof() {
                            return Err(Unsupported);
                        }
                        match self.advance() {
                            Tok::Ident(_) | Tok::Str(_) | Tok::Num(_) => {}
                            _ => return Err(Unsupported),
                        }
                        self.expect_punct(":")?;
                        self.assignment()?;
                        if !self.eat_punct(",") && !self.is_punct("}") {
                            return Err(Unsupported);
                        }
                    }
                    Ok(Expr::Unsupported)
                })();
                self.leave();
                result
            }
            Tok::Ident(word) => match word.as_str() {
                "this" => Ok(Expr::This),
                "true" => Ok(Expr::Bool(true)),
                "false" => Ok(Expr::Bool(false)),
                "null" => Ok(Expr::Null),
                "undefined" => Ok(Expr::Undefined),
                "function" => {
                    self.enter()?;
                    let function = self.function_rest();
                    self.leave();
                    Ok(Expr::Function(Rc::new(function?)))
                }
                "new" => {
                    self.enter()?;
                    let result = (|| {
                        let mut target = self.primary()?;
                        while self.eat_punct(".") {
                            match self.advance() {
                                Tok::Ident(name) => target = Expr::Member(Box::new(target), name),
                                _ => return Err(Unsupported),
                            }
                        }
                        let args = if self.is_punct("(") {
                            self.arguments()?
                        } else {
                            Vec::new()
                        };
                        Ok(Expr::New(Box::new(target), args))
                    })();
                    self.leave();
                    result
                }
                w if KEYWORDS.contains(&w) => Err(Unsupported),
                _ => Ok(Expr::Ident(word)),
            },
            _ => Err(Unsupported),
        }
    }
}

enum Kind {
    Or,
    And,
    Bin(BinOp),
    Bits,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<Tok> {
        lex(src).into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn lexes_names_numbers_strings_and_punctuation() {
        assert_eq!(
            toks("a.b_1 == 'x\\n' // c\n/* d */ 0x1F 1.5e2 !== \"q\\\"\""),
            [
                Tok::Ident("a".into()),
                Tok::Punct("."),
                Tok::Ident("b_1".into()),
                Tok::Punct("=="),
                Tok::Str("x\n".into()),
                Tok::Num(31.0),
                Tok::Num(150.0),
                Tok::Punct("!=="),
                Tok::Str("q\"".into()),
                Tok::Eof,
            ]
        );
        assert_eq!(
            toks("x = /\\b\\S+\\b/g; y = a / b / c"),
            [
                Tok::Ident("x".into()),
                Tok::Punct("="),
                Tok::Regex("\\b\\S+\\b".into(), "g".into()),
                Tok::Punct(";"),
                Tok::Ident("y".into()),
                Tok::Punct("="),
                Tok::Ident("a".into()),
                Tok::Punct("/"),
                Tok::Ident("b".into()),
                Tok::Punct("/"),
                Tok::Ident("c".into()),
                Tok::Eof,
            ]
        );
        let lexed = lex("a\nb");
        assert!(!lexed[0].newline_before && lexed[1].newline_before);
        assert_eq!(toks("\u{1F600} @ #"), [Tok::Eof]);
        assert_eq!(
            toks("'unterminated"),
            [Tok::Str("unterminated".into()), Tok::Eof]
        );
    }

    #[test]
    fn reads_statements_with_inserted_semicolons() {
        let program = parse(
            "var a = 1, b\nif (a == 1) { x.presence = \"hidden\" } else y.rawValue = 2;\n\
             switch (this.rawValue) { case \"0\": f(); break; default: }\n\
             function g(p, q) { return p + q }\nreturn\nz.v = a ? 1 : 2",
        );
        assert_eq!(program.len(), 6, "{program:?}");
        assert!(matches!(&program[0], Stmt::Var(d) if d.len() == 2));
        assert!(matches!(&program[1], Stmt::If(_, _, Some(_))));
        assert!(matches!(&program[2], Stmt::Switch(_, cases) if cases.len() == 2));
        assert!(matches!(&program[3], Stmt::Function(f) if f.params == ["p", "q"]));
        assert_eq!(program[4], Stmt::Return(None));
        assert!(
            matches!(&program[5], Stmt::Expr(Expr::Assign(None, _, c)) if matches!(**c, Expr::Cond(..)))
        );
    }

    #[test]
    fn precedence_and_calls() {
        let program = parse("a + b * c == d && !e || f.g(h, i)[j]");
        let Stmt::Expr(Expr::Logical(false, left, right)) = &program[0] else {
            panic!("{program:?}");
        };
        assert!(matches!(**left, Expr::Logical(true, _, _)));
        assert!(matches!(**right, Expr::Index(..)));
        let Expr::Logical(true, eq, not) = &**left else {
            panic!();
        };
        assert!(matches!(**eq, Expr::Binary(BinOp::Eq, _, _)));
        assert!(matches!(**not, Expr::Unary(UnOp::Not, _)));
        let Expr::Binary(BinOp::Eq, sum, _) = &**eq else {
            panic!();
        };
        assert!(matches!(**sum, Expr::Binary(BinOp::Add, _, _)));
    }

    #[test]
    fn unknown_statements_are_skipped_one_at_a_time() {
        let program = parse(
            "a.b = 1;\nvar p = new RegExp(/x/);\nfor (k in o) { q = 1 }\nlabel: x = 2\n\
             c.d = 2\nx |= 3; e = 3; f = [1, 2]; g = {a: 1}; h = typeof i;\n{ j = 1; @ k = 2; } l = 4",
        );
        let kinds: Vec<&str> = program
            .iter()
            .map(|s| match s {
                Stmt::Expr(Expr::Assign(..)) => "assign",
                Stmt::Expr(_) => "expr",
                Stmt::Var(_) => "var",
                Stmt::Skipped => "skipped",
                Stmt::Block(_) => "block",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "assign", "var", "skipped", "skipped", "assign", "skipped", "assign", "assign",
                "assign", "assign", "block", "assign"
            ],
            "{program:?}"
        );
        // `for (k in o)` is skipped as a whole, including its block.
        assert!(matches!(&program[2], Stmt::Skipped));
        // Depth is bounded.
        let deep = format!("{}1{}", "(".repeat(100), ")".repeat(100));
        assert_eq!(parse(&deep), [Stmt::Skipped]);
        let nested = format!("{}{}", "{".repeat(100), "}".repeat(100));
        assert!(
            parse(&nested)
                .iter()
                .all(|s| matches!(s, Stmt::Skipped | Stmt::Block(_)))
        );
        assert_eq!(parse(""), []);
        assert_eq!(parse("/* only a comment */"), []);
    }

    #[test]
    fn try_throw_and_loops_read() {
        let program = parse(
            "try { a = 1 } catch (e) { b = 2 } finally { c = 3 }\nthrow 'x';\n\
             while (i < 3) i++;\ndo { j-- } while (j > 0)\nfor (var k = 0; k < 2; k++) {}",
        );
        assert!(matches!(&program[0], Stmt::Block(b) if b.len() == 1));
        assert_eq!(program[1], Stmt::Skipped);
        assert!(matches!(&program[2], Stmt::While(..)));
        assert!(matches!(&program[3], Stmt::DoWhile(..)));
        assert!(matches!(
            &program[4],
            Stmt::For(Some(_), Some(_), Some(_), _)
        ));
        assert_eq!(program.len(), 5, "{program:?}");
    }
}
