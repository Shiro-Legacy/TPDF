// SPDX-License-Identifier: MIT
//! FormCalc, XFA's default script language (a `script` without a
//! JavaScript `contentType`): tokens, a small syntax tree and an
//! evaluator over the same `Runner` the JavaScript uses, so names, SOM
//! steps, values and presence go through one implementation.
//!
//! Rules:
//! - Comments run from `;` or `//` to the line end. Keywords and built-in
//!   function names are case-insensitive; variables are not.
//! - Statements: expressions, assignments (`Ref = e` sets the node's raw
//!   value, `Ref.presence = e` / `.access` / `.rawValue` the property,
//!   `x = e` a variable), `var x [= e]`, `if (c) then ... [elseif (c)
//!   then ...]* [else ...] endif`, `while (c) do ... endwhile`, `for [var]
//!   i = a upto|downto b [step s] do ... endfor`, `func f(a, b) do ...
//!   endfunc`, `break`, `continue`, `return [e]`, `foreach v in (e, ...)
//!   do ... endfor` (the body once per item, a list argument giving each
//!   of its items: `v` holds the node, so `v` is its value and
//!   `v.rawValue = x` sets it; one step per item).
//! - References: `Name`, `Name[n]`, `A.B`, `#class[n]`, `$`, `$form`,
//!   `$event`, `$host` (its methods do nothing), `xfa`, method calls such
//!   as `Ref.resolveNode("...")`, through the JavaScript runner's members;
//!   `Name[*]`, `A..Name`, `A.*`, relative indexes (`Row[-1]`), `$record`,
//!   `$data` and `!name` through `som` (lists of form or data nodes). A
//!   reference used as a value is the node's raw value (null when empty);
//!   a list used as a value is its first item's, and the aggregates (Sum,
//!   Avg, Count, Max, Min, Concat, NPV) read every item. A list as an
//!   assignment's target is its first node; data nodes are not written.
//! - Script objects: a FormCalc `variables/script` object's `func`s are
//!   called as `Object.Func(args)`, their names resolving from the
//!   subform holding the object; a JavaScript object's functions run as
//!   JavaScript.
//! - Values are numbers, strings and null. `+ - * /` are numeric (null
//!   and a string that is no number are 0; a division by zero is null);
//!   `and`/`or`/`not` and the comparisons give 1 or 0. `==`/`<>`: null
//!   equals null and the empty string, nothing else (so `null == 0` is
//!   0, as in FormCalc); two strings compare as text; anything else as
//!   numbers. A node's value whose text is a number compares as a number
//!   (`Qty > 9` with "10" in Qty is 1), a string literal as text
//!   (`"10" > "9"` is 0). `<` and friends with one null operand read it
//!   as 0 or "" to match the other. Numbers are written with at most 15
//!   significant digits, so `0.1 + 0.2` is "0.3".
//! - A script's value (a `calculate` script's result) is the value of
//!   the last expression statement evaluated, inside `if` and loop
//!   bodies too; assignments and declarations give none, so a script
//!   that only assigns keeps what it set. A function's value is its
//!   `return` value or its last expression's.
//! - Built-ins: Abs, Avg, Ceil, Count, Floor, Max, Min, Mod, Round, Sum
//!   (the aggregates take any number of values, references or lists and
//!   ignore null and unsupported ones), At, Concat, Left, Len, Lower,
//!   Ltrim, Replace, Right, Rtrim, Space, Str, Stuff, Substr, Upper,
//!   Choose, Exists, HasValue, Oneof, Ref, Within, Null; through `fclib`
//!   and `picture`: Date, Date2Num, DateFmt, IsoDate2Num, IsoTime2Num,
//!   LocalDateFmt, LocalTimeFmt, Num2Date, Num2GMTime, Num2Time, Time,
//!   Time2Num, TimeFmt (dates count days from 1 = 1900-01-01, times
//!   milliseconds from 1 = midnight GMT; local time is GMT; a picture
//!   argument is read by `picture`, the default being the locale's medium
//!   pattern, and without one Date2Num/Time2Num read leniently), Format,
//!   Parse (the script node's locale, else the form's), WordNum
//!   (English), UnitType, UnitValue, Apr, CTerm, FV, IPmt, NPV, Pmt,
//!   PPmt, PV, Rate, Term, Encode, Decode. Eval, Get, Post, Put and
//!   anything unknown are null.
//! - Reading never fails and running never throws: a statement the reader
//!   cannot follow costs that statement (to the line end, or the whole
//!   `if`/`while`/`for`/`func` when its head is the broken part); a bad
//!   reference or a call of something that is no function is null.
//! - Bounded as the JavaScript: the runner's step budgets
//!   (`MAX_STEPS`, `MAX_SCRIPT_STEPS`, long string results cost more, a
//!   list one step per item and per node its search visited),
//!   `MAX_DEPTH` levels of nesting, `MAX_CALL_DEPTH` calls,
//!   `MAX_STRING_CHARS` per string (a longer result is null),
//!   `MAX_LOCALS` variables per function.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::Rc;

use super::eval::{
    EventInfo, MAX_CALL_DEPTH, MAX_LOCALS, MAX_STRING_CHARS, NodeRef, NodeView, Runner, Stopped,
    Value, view,
};
use super::fclib;
use super::som::{MAX_LIST, Pick, Sel, collapse};
use crate::xfa::picture;
use crate::xfa::types::ValueKind;

/// Statement and expression nesting the reader follows, at most.
pub const MAX_DEPTH: usize = 48;

// ------------------------------------------------------------------ tokens

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Num(f64),
    Str(String),
    Punct(&'static str),
    Eof,
}

#[derive(Clone, Debug, PartialEq)]
struct Token {
    tok: Tok,
    /// A line end lies between this token and the one before it.
    line_start: bool,
}

/// Punctuators, longest first. `?` stands for a character no rule reads.
const PUNCTS: &[&str] = &[
    "==", "<>", "<=", ">=", "..", "(", ")", "[", "]", ",", ".", "=", "+", "-", "*", "/", "<", ">",
    "|", "&",
];

const KEYWORDS: &[&str] = &[
    "and", "break", "continue", "do", "downto", "else", "elseif", "endfor", "endfunc", "endif",
    "endwhile", "eq", "false", "for", "foreach", "func", "ge", "gt", "if", "in", "le", "lt", "ne",
    "not", "null", "or", "return", "step", "then", "true", "upto", "var", "while",
];

/// Openers and closers of the block statements, for skipping one whole.
const OPENERS: &[&str] = &["if", "while", "for", "foreach", "func"];
const CLOSERS: &[&str] = &["endif", "endwhile", "endfor", "endfunc"];

fn is_keyword(name: &str) -> bool {
    KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(name))
}

fn lex(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0;
    let mut line_start = true;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' || c == '\r' {
            line_start = true;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == ';' || (c == '/' && chars.get(i + 1) == Some(&'/')) {
            while i < chars.len() && chars[i] != '\n' && chars[i] != '\r' {
                i += 1;
            }
            continue;
        }
        let after_operand = matches!(
            out.last().map(|t| &t.tok),
            Some(Tok::Ident(_) | Tok::Num(_) | Tok::Str(_) | Tok::Punct(")" | "]"))
        );
        let next_digit = chars.get(i + 1).is_some_and(char::is_ascii_digit);
        let tok = if c == '"' {
            i += 1;
            let mut s = String::new();
            while let Some(&ch) = chars.get(i) {
                if ch == '"' {
                    if chars.get(i + 1) == Some(&'"') {
                        s.push('"');
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                if ch == '\\' && chars.get(i + 1) == Some(&'u') {
                    let hex: String = chars.iter().skip(i + 2).take(4).collect();
                    if let Some(decoded) = (hex.len() == 4)
                        .then(|| u32::from_str_radix(&hex, 16).ok())
                        .flatten()
                        .and_then(char::from_u32)
                    {
                        s.push(decoded);
                        i += 6;
                        continue;
                    }
                }
                s.push(ch);
                i += 1;
            }
            Tok::Str(s)
        } else if c.is_ascii_digit() || (c == '.' && next_digit && !after_operand) {
            let start = i;
            while chars.get(i).is_some_and(char::is_ascii_digit) {
                i += 1;
            }
            if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
                i += 1;
                while chars.get(i).is_some_and(char::is_ascii_digit) {
                    i += 1;
                }
            }
            if matches!(chars.get(i), Some('e' | 'E')) {
                let sign = usize::from(matches!(chars.get(i + 1), Some('+' | '-')));
                if chars.get(i + 1 + sign).is_some_and(char::is_ascii_digit) {
                    i += 1 + sign;
                    while chars.get(i).is_some_and(char::is_ascii_digit) {
                        i += 1;
                    }
                }
            }
            let text: String = chars[start..i].iter().collect();
            Tok::Num(text.parse().unwrap_or(0.0))
        } else if c.is_alphabetic() || matches!(c, '_' | '$' | '!' | '#') {
            let start = i;
            i += 1;
            while chars
                .get(i)
                .is_some_and(|&ch| ch.is_alphanumeric() || ch == '_')
            {
                i += 1;
            }
            Tok::Ident(chars[start..i].iter().collect())
        } else {
            let rest = &chars[i..];
            let punct = PUNCTS
                .iter()
                .find(|p| p.chars().zip(rest).filter(|(a, b)| a == *b).count() == p.len())
                .copied()
                .unwrap_or("?");
            i += if punct == "?" { 1 } else { punct.len() };
            Tok::Punct(punct)
        };
        out.push(Token { tok, line_start });
        line_start = false;
    }
    out.push(Token {
        tok: Tok::Eof,
        line_start: true,
    });
    out
}

// ------------------------------------------------------------- syntax tree

#[derive(Clone, Copy, Debug, PartialEq)]
enum BinOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum UnOp {
    Neg,
    Plus,
    Not,
}

#[derive(Clone, Debug, PartialEq)]
enum Index {
    At(Expr),
    /// `[*]`: every node of the name.
    All,
    /// `[-1]`, `[+1]`: relative to the node of the name the script is in.
    Rel(Expr),
}

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Num(f64),
    Str(Rc<str>),
    Null,
    /// A first step: a variable, a name in scope, `$...`, `#class`.
    Name(String, Option<Box<Index>>),
    /// `object.name[index]`.
    Member(Box<Expr>, String, Option<Box<Index>>),
    Call(Box<Expr>, Vec<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `object..name[index]`: nodes of the name anywhere under the object.
    Descendant(Box<Expr>, String, Option<Box<Index>>),
    /// `object.*`: the object's children.
    Children(Box<Expr>),
    /// Read but not evaluated (an index on a call's result): null.
    Unsupported,
}

#[derive(Debug, PartialEq)]
pub(super) struct Func {
    params: Vec<String>,
    body: Vec<Stmt>,
}

/// A FormCalc script object (`variables/script`): its functions by
/// lower-case name.
#[derive(Debug, Default)]
pub(super) struct FcObject {
    funcs: HashMap<String, Rc<Func>>,
}

#[derive(Debug, PartialEq)]
enum Stmt {
    Expr(Expr),
    Assign(Expr, Expr),
    Var(String, Option<Expr>),
    /// `(condition, body)` arms in order, then the `else` body.
    If(Vec<(Expr, Vec<Stmt>)>, Vec<Stmt>),
    While(Expr, Vec<Stmt>),
    For {
        var: String,
        start: Expr,
        end: Expr,
        down: bool,
        step: Option<Expr>,
        body: Vec<Stmt>,
    },
    /// `foreach var in (lists) do body endfor`.
    Foreach {
        var: String,
        lists: Vec<Expr>,
        body: Vec<Stmt>,
    },
    Func(String, Rc<Func>),
    Return(Option<Expr>),
    Break,
    Continue,
    /// A statement the reader could not follow, or one not run.
    Skipped,
}

// ------------------------------------------------------------------ reader

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    depth: usize,
}

fn parse(source: &str) -> Vec<Stmt> {
    let mut p = Parser {
        toks: lex(source),
        pos: 0,
        depth: 0,
    };
    p.stmts(&[])
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos.min(self.toks.len() - 1)].tok
    }
    fn line_start(&self) -> bool {
        self.toks[self.pos.min(self.toks.len() - 1)].line_start
    }
    fn bump(&mut self) {
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
    }
    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }
    fn is_kw(&self, kw: &str) -> bool {
        matches!(self.peek(), Tok::Ident(n) if n.eq_ignore_ascii_case(kw))
    }
    fn is_any_kw(&self, kws: &[&str]) -> bool {
        kws.iter().any(|k| self.is_kw(k))
    }
    fn eat_kw(&mut self, kw: &str) -> bool {
        let found = self.is_kw(kw);
        if found {
            self.bump();
        }
        found
    }
    fn expect_kw(&mut self, kw: &str) -> Option<()> {
        self.eat_kw(kw).then_some(())
    }
    fn is_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }
    fn eat_punct(&mut self, p: &str) -> bool {
        let found = self.is_punct(p);
        if found {
            self.bump();
        }
        found
    }
    fn expect_punct(&mut self, p: &str) -> Option<()> {
        self.eat_punct(p).then_some(())
    }
    /// A name that is not a keyword.
    fn name(&mut self) -> Option<String> {
        match self.peek() {
            Tok::Ident(n) if !is_keyword(n) => {
                let n = n.clone();
                self.bump();
                Some(n)
            }
            _ => None,
        }
    }

    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        if self.depth >= MAX_DEPTH {
            return None;
        }
        self.depth += 1;
        let result = f(self);
        self.depth -= 1;
        result
    }

    /// Statements up to one of `terminators` (left unread) or the end.
    fn stmts(&mut self, terminators: &[&str]) -> Vec<Stmt> {
        let mut out = Vec::new();
        while !self.at_eof() && !self.is_any_kw(terminators) {
            let start = self.pos;
            match self.nested(Self::stmt) {
                Some(stmt) => out.push(stmt),
                None => {
                    self.recover(start);
                    out.push(Stmt::Skipped);
                }
            }
        }
        out
    }

    /// After a statement the reader could not follow from `start`: a
    /// block statement is skipped to its matching closer, anything else
    /// to the next line. Always moves past `start`.
    fn recover(&mut self, start: usize) {
        self.pos = start;
        if self.is_any_kw(OPENERS) {
            self.skip_block();
            return;
        }
        self.bump();
        while !self.at_eof() && !self.line_start() {
            self.bump();
        }
    }

    /// From a block opener past its matching closer (or to the end).
    fn skip_block(&mut self) {
        let mut open = 0usize;
        while !self.at_eof() {
            if self.is_any_kw(OPENERS) {
                open += 1;
            } else if self.is_any_kw(CLOSERS) {
                open = open.saturating_sub(1);
                if open == 0 {
                    self.bump();
                    return;
                }
            }
            self.bump();
        }
    }

    fn stmt(&mut self) -> Option<Stmt> {
        if self.eat_kw("var") {
            let name = self.name()?;
            let init = if self.eat_punct("=") {
                Some(self.expr()?)
            } else {
                None
            };
            return Some(Stmt::Var(name, init));
        }
        if self.eat_kw("func") {
            let name = self.name()?;
            self.expect_punct("(")?;
            let mut params = Vec::new();
            if !self.eat_punct(")") {
                loop {
                    params.push(self.name()?);
                    if self.eat_punct(")") {
                        break;
                    }
                    self.expect_punct(",")?;
                }
            }
            self.expect_kw("do")?;
            let body = self.stmts(&["endfunc"]);
            self.expect_kw("endfunc")?;
            return Some(Stmt::Func(name, Rc::new(Func { params, body })));
        }
        if self.eat_kw("if") {
            let mut arms = Vec::new();
            let cond = self.paren_expr()?;
            self.expect_kw("then")?;
            arms.push((cond, self.stmts(&["elseif", "else", "endif"])));
            let mut otherwise = Vec::new();
            loop {
                if self.eat_kw("elseif") {
                    let cond = self.paren_expr()?;
                    self.expect_kw("then")?;
                    arms.push((cond, self.stmts(&["elseif", "else", "endif"])));
                } else if self.eat_kw("else") {
                    otherwise = self.stmts(&["endif"]);
                    self.expect_kw("endif")?;
                    break;
                } else {
                    self.expect_kw("endif")?;
                    break;
                }
            }
            return Some(Stmt::If(arms, otherwise));
        }
        if self.eat_kw("while") {
            let cond = self.paren_expr()?;
            self.expect_kw("do")?;
            let body = self.stmts(&["endwhile"]);
            self.expect_kw("endwhile")?;
            return Some(Stmt::While(cond, body));
        }
        if self.eat_kw("for") {
            self.eat_kw("var");
            let var = self.name()?;
            self.expect_punct("=")?;
            let start = self.expr()?;
            let down = if self.eat_kw("downto") {
                true
            } else {
                self.expect_kw("upto")?;
                false
            };
            let end = self.expr()?;
            let step = if self.eat_kw("step") {
                Some(self.expr()?)
            } else {
                None
            };
            self.expect_kw("do")?;
            let body = self.stmts(&["endfor"]);
            self.expect_kw("endfor")?;
            return Some(Stmt::For {
                var,
                start,
                end,
                down,
                step,
                body,
            });
        }
        if self.eat_kw("foreach") {
            let var = self.name()?;
            self.expect_kw("in")?;
            self.expect_punct("(")?;
            let mut lists = Vec::new();
            if !self.eat_punct(")") {
                loop {
                    lists.push(self.expr()?);
                    if self.eat_punct(")") {
                        break;
                    }
                    self.expect_punct(",")?;
                }
            }
            self.expect_kw("do")?;
            let body = self.stmts(&["endfor"]);
            self.expect_kw("endfor")?;
            return Some(Stmt::Foreach { var, lists, body });
        }
        if self.eat_kw("break") {
            return Some(Stmt::Break);
        }
        if self.eat_kw("continue") {
            return Some(Stmt::Continue);
        }
        if self.eat_kw("return") {
            let value = if !self.line_start() && self.starts_expr() {
                Some(self.expr()?)
            } else {
                None
            };
            return Some(Stmt::Return(value));
        }
        let target = self.expr()?;
        if self.eat_punct("=") {
            if !matches!(target, Expr::Name(..) | Expr::Member(..)) {
                return None;
            }
            let value = self.expr()?;
            return Some(Stmt::Assign(target, value));
        }
        Some(Stmt::Expr(target))
    }

    fn starts_expr(&self) -> bool {
        match self.peek() {
            Tok::Num(_) | Tok::Str(_) => true,
            Tok::Ident(n) => {
                !is_keyword(n)
                    || matches!(n.to_lowercase().as_str(), "null" | "not" | "true" | "false")
            }
            Tok::Punct(p) => matches!(*p, "(" | "-" | "+"),
            Tok::Eof => false,
        }
    }

    fn paren_expr(&mut self) -> Option<Expr> {
        self.expect_punct("(")?;
        let e = self.expr()?;
        self.expect_punct(")")?;
        Some(e)
    }

    fn expr(&mut self) -> Option<Expr> {
        self.nested(|p| p.binary(0))
    }

    /// The binary operator at the reader for precedence `level` (0 binds
    /// loosest).
    fn op_at(&self, level: usize) -> Option<BinOp> {
        let table: &[(&str, &str, BinOp)] = match level {
            0 => &[("|", "or", BinOp::Or)],
            1 => &[("&", "and", BinOp::And)],
            2 => &[("==", "eq", BinOp::Eq), ("<>", "ne", BinOp::Ne)],
            3 => &[
                ("<", "lt", BinOp::Lt),
                ("<=", "le", BinOp::Le),
                (">", "gt", BinOp::Gt),
                (">=", "ge", BinOp::Ge),
            ],
            4 => &[("+", "", BinOp::Add), ("-", "", BinOp::Sub)],
            _ => &[("*", "", BinOp::Mul), ("/", "", BinOp::Div)],
        };
        table
            .iter()
            .find(|(p, kw, _)| self.is_punct(p) || (!kw.is_empty() && self.is_kw(kw)))
            .map(|(_, _, op)| *op)
    }

    fn binary(&mut self, level: usize) -> Option<Expr> {
        if level > 5 {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(op) = self.op_at(level) {
            self.bump();
            let right = self.binary(level + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Some(left)
    }

    fn unary(&mut self) -> Option<Expr> {
        self.nested(|p| {
            let op = if p.eat_punct("-") {
                UnOp::Neg
            } else if p.eat_punct("+") {
                UnOp::Plus
            } else if p.eat_kw("not") {
                UnOp::Not
            } else {
                return p.postfix();
            };
            Some(Expr::Unary(op, Box::new(p.unary()?)))
        })
    }

    fn postfix(&mut self) -> Option<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.eat_punct(".") {
                match self.peek().clone() {
                    Tok::Ident(n) => {
                        self.bump();
                        e = Expr::Member(Box::new(e), n, None);
                    }
                    Tok::Punct("*") => {
                        self.bump();
                        e = Expr::Children(Box::new(e));
                    }
                    _ => return None,
                }
            } else if self.eat_punct("..") {
                let Tok::Ident(n) = self.peek().clone() else {
                    return None;
                };
                self.bump();
                e = Expr::Descendant(Box::new(e), n, None);
            } else if self.eat_punct("[") {
                let index = if self.eat_punct("*") {
                    Index::All
                } else if self.is_punct("+") || self.is_punct("-") {
                    Index::Rel(self.expr()?)
                } else {
                    Index::At(self.expr()?)
                };
                self.expect_punct("]")?;
                let index = Some(Box::new(index));
                e = match e {
                    Expr::Name(n, None) => Expr::Name(n, index),
                    Expr::Member(o, n, None) => Expr::Member(o, n, index),
                    Expr::Descendant(o, n, None) => Expr::Descendant(o, n, index),
                    _ => Expr::Unsupported,
                };
            } else if self.is_punct("(")
                && !self.line_start()
                && matches!(e, Expr::Name(_, None) | Expr::Member(_, _, None))
            {
                self.bump();
                let mut args = Vec::new();
                if !self.eat_punct(")") {
                    loop {
                        args.push(self.expr()?);
                        if self.eat_punct(")") {
                            break;
                        }
                        self.expect_punct(",")?;
                    }
                }
                e = Expr::Call(Box::new(e), args);
            } else {
                return Some(e);
            }
        }
    }

    fn primary(&mut self) -> Option<Expr> {
        let e = match self.peek().clone() {
            Tok::Num(n) => Expr::Num(n),
            Tok::Str(s) => Expr::Str(Rc::from(s)),
            Tok::Punct("(") => {
                self.bump();
                let e = self.expr()?;
                self.expect_punct(")")?;
                return Some(e);
            }
            Tok::Ident(n) if n.eq_ignore_ascii_case("null") => Expr::Null,
            Tok::Ident(n) if n.eq_ignore_ascii_case("true") => Expr::Num(1.0),
            Tok::Ident(n) if n.eq_ignore_ascii_case("false") => Expr::Num(0.0),
            Tok::Ident(n) if !is_keyword(&n) => Expr::Name(n, None),
            _ => return None,
        };
        self.bump();
        Some(e)
    }
}

// --------------------------------------------------------------- evaluator

enum Flow {
    Next,
    Break,
    Continue,
    Return(Option<Value>),
}

struct Scope {
    locals: HashMap<String, Value>,
    depth: usize,
}

impl Scope {
    fn set(&mut self, name: &str, value: Value) {
        if self.locals.len() >= MAX_LOCALS && !self.locals.contains_key(name) {
            return;
        }
        self.locals.insert(name.to_string(), value);
    }
}

struct Fc<'r, 'f> {
    runner: &'r mut Runner<'f>,
    this: NodeRef,
    /// User functions by lower-case name.
    funcs: HashMap<String, Rc<Func>>,
    /// The value of the last expression statement evaluated.
    last: Value,
}

/// Properties an assignment sets on the node itself (not a child).
const PROPERTIES: &[&str] = &["rawValue", "value", "presence", "access"];

impl Runner<'_> {
    /// Runs the FormCalc `source` with `$` = `ctx` for `event`, returning
    /// its value (the last expression statement's; `Undefined` when the
    /// script evaluated none). `Err(Stopped)` when a bound stopped it.
    pub fn run_formcalc(
        &mut self,
        source: &str,
        ctx: &NodeRef,
        event: EventInfo,
    ) -> Result<Value, Stopped> {
        self.begin(event)?;
        let program = parse(source);
        let mut fc = Fc {
            runner: self,
            this: ctx.clone(),
            funcs: HashMap::new(),
            last: Value::Undefined,
        };
        fc.hoist(&program);
        let mut scope = Scope {
            locals: HashMap::new(),
            depth: 0,
        };
        Ok(match fc.exec_block(&program, &mut scope)? {
            Flow::Return(Some(v)) => v,
            _ => fc.last,
        })
    }
}

impl Fc<'_, '_> {
    fn hoist(&mut self, body: &[Stmt]) {
        for stmt in body {
            if let Stmt::Func(name, f) = stmt {
                self.funcs.insert(name.to_lowercase(), f.clone());
            }
        }
    }

    fn exec_block(&mut self, body: &[Stmt], scope: &mut Scope) -> Result<Flow, Stopped> {
        for stmt in body {
            match self.exec(stmt, scope)? {
                Flow::Next => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Next)
    }

    fn exec(&mut self, stmt: &Stmt, scope: &mut Scope) -> Result<Flow, Stopped> {
        self.runner.tick()?;
        match stmt {
            Stmt::Expr(e) => {
                let v = self.eval(e, scope)?;
                self.last = self.scalar(&v);
            }
            Stmt::Assign(target, value) => {
                let v = self.eval(value, scope)?;
                let v = self.stored(v, value);
                self.assign(target, v, scope)?;
            }
            Stmt::Var(name, init) => {
                let v = match init {
                    Some(e) => {
                        let v = self.eval(e, scope)?;
                        self.stored(v, e)
                    }
                    None => Value::Str(Rc::from("")),
                };
                scope.set(name, v);
            }
            Stmt::If(arms, otherwise) => {
                for (cond, body) in arms {
                    let c = self.eval(cond, scope)?;
                    if self.truthy(&c) {
                        return self.exec_block(body, scope);
                    }
                }
                return self.exec_block(otherwise, scope);
            }
            Stmt::While(cond, body) => loop {
                self.runner.tick()?;
                let c = self.eval(cond, scope)?;
                if !self.truthy(&c) {
                    break;
                }
                match self.exec_block(body, scope)? {
                    Flow::Break => break,
                    Flow::Return(v) => return Ok(Flow::Return(v)),
                    Flow::Next | Flow::Continue => {}
                }
            },
            Stmt::For {
                var,
                start,
                end,
                down,
                step,
                body,
            } => {
                let v = self.eval(start, scope)?;
                let mut i = self.number(&v);
                let v = self.eval(end, scope)?;
                let end = self.number(&v);
                let step = match step {
                    Some(e) => {
                        let v = self.eval(e, scope)?;
                        self.number(&v)
                    }
                    None => 1.0,
                };
                // A step that does not move toward the end runs nothing.
                if step > 0.0 {
                    loop {
                        self.runner.tick()?;
                        if (*down && i < end) || (!*down && i > end) {
                            break;
                        }
                        scope.set(var, Value::Num(i));
                        match self.exec_block(body, scope)? {
                            Flow::Break => break,
                            Flow::Return(v) => return Ok(Flow::Return(v)),
                            Flow::Next | Flow::Continue => {}
                        }
                        let current = scope.locals.get(var).cloned().unwrap_or(Value::Null);
                        let current = self.number(&current);
                        i = if *down {
                            current - step
                        } else {
                            current + step
                        };
                    }
                }
            }
            Stmt::Foreach { var, lists, body } => {
                let mut items = Vec::new();
                for e in lists {
                    let v = self.eval(e, scope)?;
                    match v {
                        Value::List(list) => items.extend(list.iter().cloned()),
                        Value::Undefined => items.push(Value::Null),
                        Value::Node(_) | Value::Data(_) => items.push(v),
                        other => items.push(self.scalar(&other)),
                    }
                    items.truncate(MAX_LIST);
                }
                for item in items {
                    self.runner.tick()?;
                    scope.set(var, item);
                    match self.exec_block(body, scope)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Next | Flow::Continue => {}
                    }
                }
            }
            Stmt::Func(name, f) => {
                self.funcs.insert(name.to_lowercase(), f.clone());
            }
            Stmt::Return(value) => {
                let v = match value {
                    Some(e) => {
                        let v = self.eval(e, scope)?;
                        Some(self.scalar(&v))
                    }
                    None => None,
                };
                return Ok(Flow::Return(v));
            }
            Stmt::Break => return Ok(Flow::Break),
            Stmt::Continue => return Ok(Flow::Continue),
            Stmt::Skipped => {}
        }
        Ok(Flow::Next)
    }

    /// What a variable or a node takes from `v`: the value, or the
    /// reference itself when `from` is a `Ref(...)` call.
    fn stored(&self, v: Value, from: &Expr) -> Value {
        let is_ref = matches!(from, Expr::Call(callee, _)
            if matches!(&**callee, Expr::Name(n, None) if n.eq_ignore_ascii_case("ref")));
        if is_ref && matches!(v, Value::Node(_) | Value::Data(_)) {
            v
        } else {
            self.scalar(&v)
        }
    }

    fn assign(&mut self, target: &Expr, value: Value, scope: &mut Scope) -> Result<(), Stopped> {
        if let Expr::Name(n, None) = target {
            if scope.locals.contains_key(n) {
                scope.set(n, value);
                return Ok(());
            }
        }
        if let Expr::Member(object, prop, None) = target {
            if PROPERTIES.contains(&prop.as_str()) {
                let object = self.eval(object, scope)?;
                self.runner.set_member(&object, prop, store_number(value));
                return Ok(());
            }
        }
        let place = match self.eval(target, scope)? {
            // A list as a place is its first node.
            Value::List(items) => items.first().cloned().unwrap_or(Value::Undefined),
            other => other,
        };
        match place {
            Value::Node(at) => self.runner.set_raw_value(&at, &store_number(value)),
            Value::TextVar(..) => self.runner.set_member(&place, "value", store_number(value)),
            // Data nodes are read, not written.
            Value::Data(_) => {}
            _ => {
                if let Expr::Name(n, None) = target {
                    if !n.starts_with(['$', '#', '!']) {
                        scope.set(n, value);
                    }
                }
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------- expressions

    fn eval(&mut self, expr: &Expr, scope: &mut Scope) -> Result<Value, Stopped> {
        self.runner.tick()?;
        Ok(match expr {
            Expr::Num(n) => Value::Num(*n),
            Expr::Str(s) => Value::Str(s.clone()),
            Expr::Null => Value::Null,
            Expr::Unsupported => Value::Undefined,
            Expr::Name(name, index) => self.name(name, index.as_deref(), scope)?,
            Expr::Member(object, name, index) => {
                let object = self.eval(object, scope)?;
                self.member(&object, name, index.as_deref(), scope)?
            }
            Expr::Descendant(object, name, index) => {
                let object = self.eval(object, scope)?;
                let Some(how) = self.pick(index.as_deref(), scope)? else {
                    return Ok(Value::Undefined);
                };
                let this = self.this.clone();
                let mut found = Vec::new();
                for item in items_of(object) {
                    found.extend(self.runner.step(&item, Sel::Name(name), how, true, &this));
                    found.truncate(MAX_LIST);
                }
                self.runner.charge(found.len())?;
                collapse(found, how)
            }
            Expr::Children(object) => {
                let object = self.eval(object, scope)?;
                let this = self.this.clone();
                let mut found = Vec::new();
                for item in items_of(object) {
                    found.extend(self.runner.step(&item, Sel::Any, Pick::All, false, &this));
                    found.truncate(MAX_LIST);
                }
                self.runner.charge(found.len())?;
                Value::List(found.into())
            }
            Expr::Call(callee, args) => self.call(callee, args, scope)?,
            Expr::Unary(op, operand) => {
                let v = self.eval(operand, scope)?;
                let n = self.number(&v);
                match op {
                    UnOp::Neg => Value::Num(if n == 0.0 { 0.0 } else { -n }),
                    UnOp::Plus => Value::Num(n),
                    UnOp::Not => flag(n == 0.0),
                }
            }
            Expr::Binary(op, left, right) => {
                let l = self.eval(left, scope)?;
                match op {
                    BinOp::Or | BinOp::And => {
                        let l = self.truthy(&l);
                        if l == (*op == BinOp::Or) {
                            flag(l)
                        } else {
                            let r = self.eval(right, scope)?;
                            flag(self.truthy(&r))
                        }
                    }
                    _ => {
                        let r = self.eval(right, scope)?;
                        self.binary(*op, &l, &r)
                    }
                }
            }
        })
    }

    /// An index as a pick: `[n]` a whole number at or above 0, `[*]`,
    /// `[-n]`/`[+n]`; none is `[0]`. `None` for an index that is no whole
    /// number.
    fn pick(&mut self, index: Option<&Index>, scope: &mut Scope) -> Result<Option<Pick>, Stopped> {
        Ok(match index {
            None => Some(Pick::At(0)),
            Some(Index::All) => Some(Pick::All),
            Some(Index::At(e)) => {
                let v = self.eval(e, scope)?;
                let n = self.number(&v);
                (n >= 0.0 && n.fract() == 0.0 && n < 1e9).then_some(Pick::At(n as usize))
            }
            Some(Index::Rel(e)) => {
                let v = self.eval(e, scope)?;
                let n = self.number(&v);
                (n.fract() == 0.0 && n.abs() < 1e9).then_some(Pick::Rel(n as i64))
            }
        })
    }

    fn name(
        &mut self,
        name: &str,
        index: Option<&Index>,
        scope: &mut Scope,
    ) -> Result<Value, Stopped> {
        if index.is_none() {
            if let Some(v) = scope.locals.get(name) {
                return Ok(v.clone());
            }
        }
        let Some(how) = self.pick(index, scope)? else {
            return Ok(Value::Undefined);
        };
        let this = self.this.clone();
        let found: Vec<Value> = match name {
            "$" => return Ok(Value::Node(this)),
            "$form" => return Ok(Value::FormModel),
            "$event" => return Ok(Value::Event),
            "$host" | "$layout" | "$template" | "$config" | "$connectionSet" | "$signature" => {
                return Ok(Value::Host);
            }
            "xfa" => return Ok(Value::Xfa),
            "$record" => self
                .runner
                .data_record()
                .into_iter()
                .map(Value::Data)
                .collect(),
            "$data" => self
                .runner
                .data_root()
                .into_iter()
                .map(Value::Data)
                .collect(),
            _ if name.starts_with('!') => self.runner.datasets_child(&name[1..]),
            _ if name.starts_with('$') => return Ok(Value::Undefined),
            // A class step searches the scope chain as a name does.
            _ if name.starts_with('#') => {
                self.runner.scope_step(&this, Sel::Class(&name[1..]), how)
            }
            _ if index.is_none() => return Ok(self.runner.lookup_scope(&this, name)),
            _ => self.runner.scope_step(&this, Sel::Name(name), how),
        };
        self.runner.charge(found.len())?;
        Ok(collapse(found, how))
    }

    fn member(
        &mut self,
        object: &Value,
        name: &str,
        index: Option<&Index>,
        scope: &mut Scope,
    ) -> Result<Value, Stopped> {
        // A list's member is every item's, in order.
        if let Value::List(items) = object {
            let mut out = Vec::new();
            for item in items.iter() {
                self.runner.tick()?;
                match self.member(item, name, index, scope)? {
                    Value::List(found) => out.extend(found.iter().cloned()),
                    Value::Undefined => {}
                    other => out.push(other),
                }
                if out.len() >= MAX_LIST {
                    out.truncate(MAX_LIST);
                    break;
                }
            }
            return Ok(Value::List(out.into()));
        }
        let Some(how) = self.pick(index, scope)? else {
            return Ok(Value::Undefined);
        };
        let this = self.this.clone();
        Ok(match object {
            Value::Node(_) | Value::Data(_) if index.is_some() || name.starts_with('#') => {
                let found = self.runner.step(object, Sel::of(name), how, false, &this);
                self.runner.charge(found.len())?;
                collapse(found, how)
            }
            _ => self.runner.member_of(object, name, &this),
        })
    }

    /// The FormCalc script object `name` of the subform at `owner`,
    /// read on first use; `None` when it is not a FormCalc one.
    fn fc_object(&mut self, owner: &NodeRef, name: &str) -> Option<Rc<FcObject>> {
        let key = (owner.clone(), name.to_string());
        if let Some(found) = self.runner.fc_objects.get(&key) {
            return Some(found.clone());
        }
        let source = match view(self.runner.form, owner) {
            Some(NodeView::Subform(s)) => s
                .variables
                .iter()
                .find(|v| v.name == name && v.is_script && v.formcalc)
                .map(|v| v.text.clone())?,
            _ => return None,
        };
        let mut object = FcObject::default();
        for stmt in parse(&source) {
            if let Stmt::Func(fname, f) = stmt {
                object.funcs.insert(fname.to_lowercase(), f);
            }
        }
        let object = Rc::new(object);
        self.runner.fc_objects.insert(key, object.clone());
        Some(object)
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], scope: &mut Scope) -> Result<Value, Stopped> {
        match callee {
            Expr::Name(name, None) => {
                let lower = name.to_lowercase();
                if let Some(f) = self.funcs.get(&lower).cloned() {
                    let mut values = Vec::with_capacity(args.len());
                    for arg in args {
                        let v = self.eval(arg, scope)?;
                        values.push(self.stored(v, arg));
                    }
                    return self.call_user(&f, values, scope.depth);
                }
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(self.eval(arg, scope)?);
                }
                let result = self.builtin(&lower, &values)?;
                // What a SOM text argument's search visited.
                self.runner.charge(0)?;
                Ok(result)
            }
            Expr::Member(object, method, None) => {
                let object = self.eval(object, scope)?;
                if let Value::ScriptObject(owner, name) = &object {
                    if let Some(found) = self.fc_object(owner, name) {
                        let Some(f) = found.funcs.get(&method.to_lowercase()).cloned() else {
                            return Ok(Value::Null);
                        };
                        let mut values = Vec::with_capacity(args.len());
                        for arg in args {
                            let v = self.eval(arg, scope)?;
                            values.push(self.stored(v, arg));
                        }
                        // Its names resolve from its subform; its functions
                        // call each other.
                        let this = std::mem::replace(&mut self.this, owner.clone());
                        let funcs = std::mem::replace(&mut self.funcs, found.funcs.clone());
                        let result = self.call_user(&f, values, scope.depth);
                        self.this = this;
                        self.funcs = funcs;
                        return result;
                    }
                }
                let this = self.this.clone();
                let function = self.runner.member_of(&object, method, &this);
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    let v = self.eval(arg, scope)?;
                    values.push(match v {
                        Value::TextVar(..) | Value::Undefined => self.scalar(&v),
                        other => other,
                    });
                }
                let result = self
                    .runner
                    .call_value(function, values, &this, scope.depth)?;
                Ok(match result {
                    Value::Undefined => Value::Null,
                    other => other,
                })
            }
            _ => Ok(Value::Null),
        }
    }

    fn call_user(&mut self, f: &Func, args: Vec<Value>, depth: usize) -> Result<Value, Stopped> {
        if depth >= MAX_CALL_DEPTH {
            return Ok(Value::Null);
        }
        let mut inner = Scope {
            locals: HashMap::new(),
            depth: depth + 1,
        };
        let mut args = args.into_iter();
        for param in &f.params {
            inner.set(param, args.next().unwrap_or(Value::Null));
        }
        let saved = std::mem::replace(&mut self.last, Value::Null);
        let flow = self.exec_block(&f.body, &mut inner);
        let last = std::mem::replace(&mut self.last, saved);
        Ok(match flow? {
            Flow::Return(Some(v)) => v,
            _ => last,
        })
    }

    // ---------------------------------------------------------- conversions

    /// A value as FormCalc holds it: a number, a string or null (a node is
    /// its raw value; a boolean is 1 or 0; anything else is null).
    fn scalar(&self, v: &Value) -> Value {
        match v {
            Value::Num(_) | Value::Str(_) | Value::Null => v.clone(),
            Value::Bool(b) => flag(*b),
            Value::Node(at) => match self.runner.raw_value(at) {
                Value::Str(s) => Value::Str(s),
                _ => Value::Null,
            },
            Value::TextVar(owner, name) => Value::Str(Rc::from(self.runner.text_var(owner, name))),
            Value::Data(r) => match self.runner.data_value(r) {
                Value::Str(s) => Value::Str(s),
                _ => Value::Null,
            },
            // A list as a value is its first item's.
            Value::List(items) => items.first().map_or(Value::Null, |v| self.scalar(v)),
            _ => Value::Null,
        }
    }

    /// A comparison operand: `scalar`, with a node's numeric text read as
    /// a number.
    fn operand(&self, v: &Value) -> Value {
        if let Value::List(items) = v {
            return items.first().map_or(Value::Null, |v| self.operand(v));
        }
        let s = self.scalar(v);
        match (&s, v) {
            (Value::Str(text), Value::Node(_) | Value::TextVar(..) | Value::Data(_)) => {
                numeric(text).map_or(s.clone(), Value::Num)
            }
            _ => s,
        }
    }

    fn number(&self, v: &Value) -> f64 {
        match self.scalar(v) {
            Value::Num(n) => n,
            Value::Str(s) => numeric(&s).unwrap_or(0.0),
            _ => 0.0,
        }
    }

    fn string(&self, v: &Value) -> String {
        match self.scalar(v) {
            Value::Num(n) => num_text(n),
            Value::Str(s) => s.to_string(),
            _ => String::new(),
        }
    }

    fn truthy(&self, v: &Value) -> bool {
        self.number(v) != 0.0
    }

    fn equal(&self, a: &Value, b: &Value) -> bool {
        match (self.operand(a), self.operand(b)) {
            (Value::Null, Value::Null) => true,
            (Value::Null, Value::Str(s)) | (Value::Str(s), Value::Null) => s.is_empty(),
            (Value::Null, _) | (_, Value::Null) => false,
            (Value::Str(x), Value::Str(y)) => x == y,
            (x, y) => self.number(&x) == self.number(&y),
        }
    }

    fn order(&self, a: &Value, b: &Value) -> Option<Ordering> {
        match (self.operand(a), self.operand(b)) {
            (Value::Null, Value::Null) => Some(Ordering::Equal),
            (Value::Str(x), Value::Str(y)) => Some(x.cmp(&y)),
            (Value::Str(x), Value::Null) => Some((*x).cmp("")),
            (Value::Null, Value::Str(y)) => Some("".cmp(&*y)),
            (x, y) => self.number(&x).partial_cmp(&self.number(&y)),
        }
    }

    fn binary(&self, op: BinOp, l: &Value, r: &Value) -> Value {
        let arithmetic = |result: f64| {
            if result.is_finite() {
                Value::Num(result)
            } else {
                Value::Null
            }
        };
        let (x, y) = (|| self.number(l), || self.number(r));
        match op {
            BinOp::Add => arithmetic(x() + y()),
            BinOp::Sub => arithmetic(x() - y()),
            BinOp::Mul => arithmetic(x() * y()),
            BinOp::Div => {
                let d = y();
                if d == 0.0 {
                    Value::Null
                } else {
                    arithmetic(x() / d)
                }
            }
            BinOp::Eq => flag(self.equal(l, r)),
            BinOp::Ne => flag(!self.equal(l, r)),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                flag(self.order(l, r).is_some_and(|o| match op {
                    BinOp::Lt => o.is_lt(),
                    BinOp::Le => o.is_le(),
                    BinOp::Gt => o.is_gt(),
                    _ => o.is_ge(),
                }))
            }
            // Short-circuited in `eval`.
            BinOp::Or | BinOp::And => Value::Null,
        }
    }

    // ------------------------------------------------------------ built-ins

    /// A string result, charged to the budgets by its length (one step a
    /// KiB) and null past `MAX_STRING_CHARS`.
    fn text(&mut self, s: String) -> Result<Value, Stopped> {
        if s.len() > MAX_STRING_CHARS {
            return Ok(Value::Null);
        }
        for _ in 0..s.len() / 1024 {
            self.runner.tick()?;
        }
        Ok(Value::Str(Rc::from(s)))
    }

    fn builtin(&mut self, name: &str, args: &[Value]) -> Result<Value, Stopped> {
        // The aggregates read every item of a list argument.
        let expanded: Vec<Value>;
        let args = if matches!(
            name,
            "sum" | "avg" | "count" | "max" | "min" | "concat" | "npv"
        ) {
            expanded = items_in(args);
            self.runner.charge(expanded.len())?;
            &expanded[..]
        } else {
            args
        };
        if let Some(v) = self.library(name, args) {
            return match v {
                Value::Str(s) => self.text(s.to_string()),
                other => Ok(other),
            };
        }
        let arg = |i: usize| args.get(i).map_or(Value::Null, |v| self.scalar(v));
        let num = |i: usize| self.number(&arg(i));
        let str_arg = |i: usize| self.string(&arg(i));
        let is_null = |i: usize| matches!(arg(i), Value::Null);
        // A count argument: a whole number, never below 0 (a count past
        // the string bound stays past it).
        let count = |i: usize| num(i).trunc().clamp(0.0, (MAX_STRING_CHARS + 1) as f64) as usize;
        Ok(match name {
            "abs" | "ceil" | "floor" if is_null(0) => Value::Null,
            "abs" => Value::Num(num(0).abs()),
            "ceil" => Value::Num(num(0).ceil()),
            "floor" => Value::Num(num(0).floor()),
            "round" => {
                if is_null(0) {
                    return Ok(Value::Null);
                }
                let places = num(1).trunc().clamp(0.0, 12.0) as i32;
                let scale = 10f64.powi(places);
                Value::Num(tidy(tidy(num(0) * scale).round() / scale))
            }
            "mod" => {
                if is_null(0) || is_null(1) || num(1) == 0.0 {
                    return Ok(Value::Null);
                }
                Value::Num(num(0) % num(1))
            }
            "sum" | "avg" | "count" | "max" | "min" => {
                let values: Vec<f64> = (0..args.len()).filter(|&i| !is_null(i)).map(num).collect();
                match name {
                    "count" => Value::Num(values.len() as f64),
                    _ if values.is_empty() => Value::Null,
                    "sum" => Value::Num(values.iter().sum()),
                    "avg" => Value::Num(values.iter().sum::<f64>() / values.len() as f64),
                    "max" => Value::Num(values.iter().copied().fold(f64::MIN, f64::max)),
                    _ => Value::Num(values.iter().copied().fold(f64::MAX, f64::min)),
                }
            }
            "concat" => {
                let mut out = String::new();
                for i in 0..args.len() {
                    out.push_str(&str_arg(i));
                    if out.len() > MAX_STRING_CHARS {
                        return Ok(Value::Null);
                    }
                }
                return self.text(out);
            }
            "len" => Value::Num(str_arg(0).chars().count() as f64),
            "lower" | "upper" | "ltrim" | "rtrim" | "left" | "right" | "substr" | "stuff"
            | "replace"
                if is_null(0) =>
            {
                Value::Null
            }
            "lower" => return self.text(str_arg(0).to_lowercase()),
            "upper" => return self.text(str_arg(0).to_uppercase()),
            "ltrim" => return self.text(str_arg(0).trim_start().to_string()),
            "rtrim" => return self.text(str_arg(0).trim_end().to_string()),
            "left" => return self.text(str_arg(0).chars().take(count(1)).collect()),
            "right" => {
                let chars: Vec<char> = str_arg(0).chars().collect();
                let n = count(1).min(chars.len());
                return self.text(chars[chars.len() - n..].iter().collect());
            }
            "substr" => {
                let chars: Vec<char> = str_arg(0).chars().collect();
                let start = (num(1).trunc().max(1.0) as usize - 1).min(chars.len());
                let end = start.saturating_add(count(2)).min(chars.len());
                return self.text(chars[start..end].iter().collect());
            }
            "stuff" => {
                let chars: Vec<char> = str_arg(0).chars().collect();
                let start = (num(1).trunc().max(1.0) as usize - 1).min(chars.len());
                let end = start.saturating_add(count(2)).min(chars.len());
                let mut out: String = chars[..start].iter().collect();
                out.push_str(&str_arg(3));
                out.extend(&chars[end..]);
                return self.text(out);
            }
            "replace" => {
                let (s, find, with) = (str_arg(0), str_arg(1), str_arg(2));
                if find.is_empty() {
                    return self.text(s);
                }
                let grows = with.len().saturating_sub(find.len());
                if s.len() + s.matches(&*find).count() * grows > MAX_STRING_CHARS {
                    return Ok(Value::Null);
                }
                return self.text(s.replace(&*find, &with));
            }
            "space" => {
                let n = count(0);
                if n > MAX_STRING_CHARS {
                    return Ok(Value::Null);
                }
                return self.text(" ".repeat(n));
            }
            "str" => {
                if is_null(0) {
                    return Ok(Value::Null);
                }
                let width = if args.len() > 1 { count(1) } else { 10 };
                let precision = num(2).trunc().clamp(0.0, 15.0) as usize;
                let digits = format!("{:.precision$}", num(0));
                let out = if digits.len() > width {
                    "*".repeat(width)
                } else {
                    format!("{digits:>width$}")
                };
                return self.text(out);
            }
            "at" => {
                let (s, needle) = (str_arg(0), str_arg(1));
                Value::Num(match s.find(&*needle) {
                    Some(byte) => (s[..byte].chars().count() + 1) as f64,
                    None => 0.0,
                })
            }
            "hasvalue" => flag(!str_arg(0).trim().is_empty()),
            "exists" => flag(match args.first() {
                Some(Value::Str(som)) => !self.runner.resolve_all(&self.this, som).is_empty(),
                Some(Value::List(items)) => !items.is_empty(),
                Some(
                    Value::Node(_) | Value::Data(_) | Value::TextVar(..) | Value::ScriptObject(..),
                ) => true,
                _ => false,
            }),
            "ref" => match args.first() {
                Some(v @ (Value::Node(_) | Value::Data(_))) => v.clone(),
                Some(Value::List(items)) => items.first().cloned().unwrap_or(Value::Null),
                Some(Value::Str(som)) => (self.runner.resolve_all(&self.this, som))
                    .into_iter()
                    .next()
                    .unwrap_or(Value::Null),
                _ => Value::Null,
            },
            "choose" => {
                let n = num(0).trunc();
                if n >= 1.0 && (n as usize) < args.len() {
                    arg(n as usize)
                } else {
                    Value::Str(Rc::from(""))
                }
            }
            "oneof" => flag(
                args.iter()
                    .skip(1)
                    .any(|v| args.first().is_some_and(|first| self.equal(first, v))),
            ),
            "within" => {
                let Some(first) = args.first() else {
                    return Ok(Value::Null);
                };
                if is_null(0) {
                    return Ok(Value::Null);
                }
                let (lo, hi) = (args.get(1), args.get(2));
                let null = Value::Null;
                let at_least = self
                    .order(first, lo.unwrap_or(&null))
                    .is_some_and(Ordering::is_ge);
                let at_most = self
                    .order(first, hi.unwrap_or(&null))
                    .is_some_and(Ordering::is_le);
                flag(at_least && at_most)
            }
            // `Null()`, `Eval`, the URL functions, and anything unknown.
            _ => Value::Null,
        })
    }

    /// The locale a date, time or picture function uses: the one named
    /// by argument `i` when given, else the script node's, else the
    /// form's.
    fn locale_arg(&self, args: &[Value], i: usize) -> String {
        let given = args.get(i).map(|v| self.string(v)).unwrap_or_default();
        if !given.trim().is_empty() {
            return given.trim().to_string();
        }
        let form = &*self.runner.form;
        match view(form, &self.this) {
            Some(NodeView::Field(f)) if !f.locale.is_empty() => f.locale.clone(),
            _ if !form.locale.is_empty() => form.locale.clone(),
            _ => "en_US".into(),
        }
    }

    /// The date, time, picture, word, measurement, financial and encoding
    /// functions (`fclib`, `picture`); `None` for any other name.
    fn library(&self, name: &str, args: &[Value]) -> Option<Value> {
        let arg = |i: usize| args.get(i).map_or(Value::Null, |v| self.scalar(v));
        let given = |i: usize| !matches!(arg(i), Value::Null);
        let num = |i: usize| self.number(&arg(i));
        let text = |i: usize| self.string(&arg(i));
        let str_value = |s: String| Value::Str(Rc::from(s));
        let number = |n: Option<f64>| n.filter(|n| n.is_finite()).map_or(Value::Null, Value::Num);
        let locales = &self.runner.form.locales;
        let picture_arg = |i: usize| given(i).then(|| text(i));
        let financial = matches!(
            name,
            "apr" | "cterm" | "fv" | "ipmt" | "npv" | "pmt" | "ppmt" | "pv" | "rate" | "term"
        );
        if financial && (args.is_empty() || (0..args.len()).any(|i| !given(i))) {
            return Some(Value::Null);
        }
        Some(match name {
            "date" => Value::Num(fclib::today() as f64),
            "time" => Value::Num(fclib::now_time() as f64),
            "date2num" | "time2num" => {
                if !given(0) {
                    return Some(Value::Null);
                }
                let date = name == "date2num";
                let kind = if date {
                    ValueKind::Date
                } else {
                    ValueKind::Time
                };
                let locale = self.locale_arg(args, 2);
                let canonical = match picture_arg(1) {
                    Some(p) => picture::parse(&p).and_then(|pattern| {
                        picture::parse_value(&pattern, kind, &text(0), locales, &locale)
                    }),
                    None => picture::lenient(kind, &text(0), locales.get(&locale)),
                };
                let n = canonical.and_then(|c| {
                    if date {
                        fclib::iso_date_number(&c)
                    } else {
                        fclib::iso_time_number(&c)
                    }
                });
                Value::Num(n.unwrap_or(0) as f64)
            }
            "num2date" | "num2time" | "num2gmtime" => {
                if !given(0) {
                    return Some(Value::Null);
                }
                let date = name == "num2date";
                let n = num(0).trunc();
                let canonical = if n.abs() > 1e12 {
                    None
                } else if date {
                    fclib::date_text(n as i64)
                } else {
                    // A GMT time says so, for a picture's zone symbol.
                    let gmt = if name == "num2gmtime" { "Z" } else { "" };
                    fclib::time_text(n as i64).map(|t| format!("{t}{gmt}"))
                };
                let Some(canonical) = canonical else {
                    return Some(str_value(String::new()));
                };
                let locale = self.locale_arg(args, 2);
                let loc = locales.get(&locale);
                let (kind, default) = if date {
                    (ValueKind::Date, &loc.date_patterns[2])
                } else {
                    (ValueKind::Time, &loc.time_patterns[2])
                };
                let pic = picture_arg(1).unwrap_or_else(|| default.clone());
                let shown = picture::parse(&pic).and_then(|pattern| {
                    picture::format(&pattern, kind, &canonical, locales, &locale)
                });
                str_value(shown.unwrap_or_default())
            }
            "datefmt" | "localdatefmt" | "timefmt" | "localtimefmt" => {
                let style = if given(0) { num(0).trunc() } else { 0.0 };
                // 1 short, 2 medium (and the default), 3 long, 4 full.
                let slot = match style as i64 {
                    1 => 3,
                    3 => 1,
                    4 => 0,
                    _ => 2,
                };
                let loc = locales.get(&self.locale_arg(args, 1));
                let patterns = if name.ends_with("datefmt") {
                    &loc.date_patterns
                } else {
                    &loc.time_patterns
                };
                str_value(patterns[slot].clone())
            }
            "isodate2num" | "isotime2num" => {
                if !given(0) {
                    return Some(Value::Null);
                }
                let n = if name == "isodate2num" {
                    fclib::iso_date_number(&text(0))
                } else {
                    fclib::iso_time_number(&text(0))
                };
                Value::Num(n.unwrap_or(0) as f64)
            }
            "format" => {
                let pic = text(0);
                let value = text(1);
                let locale = self.locale_arg(args, usize::MAX);
                let kinds: &[ValueKind] = if numeric(&value).is_some() {
                    &[ValueKind::Decimal, ValueKind::Text]
                } else {
                    &[
                        ValueKind::Date,
                        ValueKind::Time,
                        ValueKind::DateTime,
                        ValueKind::Text,
                    ]
                };
                let shown = picture::parse(&pic).and_then(|pattern| {
                    (kinds.iter())
                        .find_map(|k| picture::format(&pattern, *k, &value, locales, &locale))
                });
                str_value(shown.unwrap_or_default())
            }
            "parse" => {
                let pic = text(0);
                let typed = text(1);
                let locale = self.locale_arg(args, usize::MAX);
                let read = picture::parse(&pic).and_then(|pattern| {
                    [
                        ValueKind::Decimal,
                        ValueKind::Date,
                        ValueKind::Time,
                        ValueKind::DateTime,
                        ValueKind::Text,
                    ]
                    .into_iter()
                    .find_map(|k| {
                        let v = picture::parse_value(&pattern, k, &typed, locales, &locale)?;
                        Some((k, v))
                    })
                });
                match read {
                    Some((ValueKind::Decimal, v)) => numeric(&v).map_or(str_value(v), Value::Num),
                    Some((_, v)) => str_value(v),
                    None => str_value(String::new()),
                }
            }
            "wordnum" => {
                if !given(0) {
                    return Some(Value::Null);
                }
                let id = if given(1) { num(1).trunc() as i64 } else { 0 };
                str_value(fclib::word_num(num(0), id))
            }
            "unittype" => match given(0).then(|| fclib::unit_type(&text(0))).flatten() {
                Some(unit) => str_value(unit.to_string()),
                None => Value::Null,
            },
            "unitvalue" => {
                if !given(0) {
                    return Some(Value::Null);
                }
                let to = picture_arg(1);
                number(fclib::unit_value(&text(0), to.as_deref()).map(tidy))
            }
            "apr" => number(fclib::apr(num(0), num(1), num(2))),
            "cterm" => number(fclib::cterm(num(0), num(1), num(2))),
            "fv" => number(fclib::fv(num(0), num(1), num(2))),
            "pmt" => number(fclib::pmt(num(0), num(1), num(2))),
            "pv" => number(fclib::pv(num(0), num(1), num(2))),
            "rate" => number(fclib::rate(num(0), num(1), num(2))),
            "term" => number(fclib::term(num(0), num(1), num(2))),
            "ipmt" => number(fclib::ipmt(num(0), num(1), num(2), num(3), num(4))),
            "ppmt" => number(fclib::ppmt(num(0), num(1), num(2), num(3), num(4))),
            "npv" => {
                let values: Vec<f64> = (1..args.len()).map(num).collect();
                number(fclib::npv(num(0), &values))
            }
            "encode" | "decode" => {
                if !given(0) {
                    return Some(Value::Null);
                }
                let kind = picture_arg(1).unwrap_or_else(|| "url".into());
                let out = if name == "encode" {
                    fclib::encode(&text(0), &kind)
                } else {
                    fclib::decode(&text(0), &kind)
                };
                out.map_or(Value::Null, str_value)
            }
            _ => return None,
        })
    }
}

/// A value's items: a list's, else the value itself.
fn items_of(v: Value) -> Vec<Value> {
    match v {
        Value::List(items) => items.to_vec(),
        other => vec![other],
    }
}

/// Arguments with every list argument's items in its place.
fn items_in(args: &[Value]) -> Vec<Value> {
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        match arg {
            Value::List(items) => out.extend(items.iter().cloned()),
            other => out.push(other.clone()),
        }
        if out.len() >= MAX_LIST {
            out.truncate(MAX_LIST);
            break;
        }
    }
    out
}

fn flag(b: bool) -> Value {
    Value::Num(if b { 1.0 } else { 0.0 })
}

/// Text that reads as a number (`"12"`, `" 1.5 "`, `"-3e2"`); not
/// `"inf"`, `"NaN"` or `""`.
fn numeric(text: &str) -> Option<f64> {
    let t = text.trim();
    let first = t.chars().next()?;
    if !(first.is_ascii_digit() || matches!(first, '.' | '+' | '-')) {
        return None;
    }
    t.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// `n` to 15 significant digits (so `0.1 + 0.2` is 0.3), and never -0.
fn tidy(n: f64) -> f64 {
    if !n.is_finite() {
        return n;
    }
    let t = format!("{n:.14e}").parse().unwrap_or(n);
    if t == 0.0 { 0.0 } else { t }
}

fn num_text(n: f64) -> String {
    let n = tidy(n);
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// A value about to be written into the form: numbers tidied.
fn store_number(v: Value) -> Value {
    match v {
        Value::Num(n) => Value::Num(tidy(n)),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xfa::script::eval::{NodeView, view};
    use crate::xfa::types::{Form, Presence};
    use crate::xfa::{bind, template, xml};

    const TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.6/">
<subform name="form1" layout="tb">
  <subform name="S" layout="tb">
    <field name="A"><ui><textEdit/></ui></field>
    <field name="B"><ui><textEdit/></ui></field>
    <field name="Out"><ui><textEdit/></ui></field>
    <field name="Name"><ui><textEdit/></ui></field>
    <subform name="Section" layout="tb"><draw name="D"/></subform>
    <subform name="Row" layout="tb"><field name="R"><ui><textEdit/></ui></field></subform>
    <subform name="Row" layout="tb"><field name="R"><ui><textEdit/></ui></field></subform>
  </subform>
</subform>
</template>"#;

    const DATASETS: &str = r#"<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data>
<form1><S><A>3</A><B>4</B><Name> ada </Name><Row><R>r0</R></Row><Row><R>r1</R></Row></S></form1>
</xfa:data></xfa:datasets>"#;

    const A: usize = 0;
    const B: usize = 1;
    const OUT: usize = 2;
    const NAME: usize = 3;
    const SECTION: usize = 4;

    fn form() -> Form {
        let mut form = template::read(TEMPLATE).unwrap();
        let datasets = xml::parse(DATASETS).unwrap();
        bind::bind(&mut form, Some(&datasets)).unwrap();
        form
    }

    fn at(child: usize) -> NodeRef {
        NodeRef(vec![0, child])
    }

    fn value(form: &Form, at: &NodeRef) -> String {
        match view(form, at).unwrap() {
            NodeView::Field(f) => f.value.plain(),
            _ => panic!("not a field"),
        }
    }

    fn run_at(form: &mut Form, ctx: usize, script: &str) -> Result<Value, Stopped> {
        let ctx = at(ctx);
        let mut runner = Runner::new(form);
        runner.run_formcalc(script, &ctx, EventInfo::default())
    }

    /// The script's value from `Out`, as text (`null` for null).
    fn result(form: &mut Form, script: &str) -> String {
        match run_at(form, OUT, script).expect("within budget") {
            Value::Num(n) => num_text(n),
            Value::Str(s) => s.to_string(),
            Value::Null => "null".into(),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn lexing_reads_doubled_quotes_comments_and_numbers() {
        let toks: Vec<Tok> =
            lex("a = \"say \"\"hi\"\"\" ; a comment\n// another\nb <> 1.5e3 + .5 x.y")
                .into_iter()
                .map(|t| t.tok)
                .collect();
        assert_eq!(
            toks,
            [
                Tok::Ident("a".into()),
                Tok::Punct("="),
                Tok::Str("say \"hi\"".into()),
                Tok::Ident("b".into()),
                Tok::Punct("<>"),
                Tok::Num(1500.0),
                Tok::Punct("+"),
                Tok::Num(0.5),
                Tok::Ident("x".into()),
                Tok::Punct("."),
                Tok::Ident("y".into()),
                Tok::Eof,
            ]
        );
        let lines: Vec<bool> = lex("a\nb c").iter().map(|t| t.line_start).collect();
        assert_eq!(lines, [true, true, false, true]);
        assert_eq!(lex("\"\\u0041\" @")[0].tok, Tok::Str("A".into()));
        assert_eq!(lex("\"\\u0041\" @")[1].tok, Tok::Punct("?"));
    }

    #[test]
    fn arithmetic_comparison_and_conversions() {
        let mut form = form();
        for (script, expected) in [
            ("1 + 2 * 3", "7"),
            ("(1 + 2) * 3", "9"),
            ("7 / 2", "3.5"),
            ("A * B", "12"),
            ("Out + 1", "1"),
            ("\"abc\" + 1", "1"),
            ("0.1 + 0.2", "0.3"),
            ("1 / 0", "null"),
            ("-A", "-3"),
            ("- - 2", "2"),
            ("\"10\" < \"9\"", "1"),
            ("A < 10", "1"),
            ("A == \"3\"", "1"),
            ("A eq 3.0", "1"),
            ("Out == null", "1"),
            ("Out == \"\"", "1"),
            ("null == 0", "0"),
            ("Out <> null", "0"),
            ("1 and 0 or 1", "1"),
            ("not 0", "1"),
            ("NOT A", "0"),
            ("3 eq 3 & 2 lt 1", "0"),
            ("1 | 0 & 0", "1"),
            ("A >= 3 and B le 4", "1"),
            ("true + true", "2"),
            ("Concat(\"a\", null, 1.5, A)", "a1.53"),
            ("Name", " ada "),
            ("Section", "null"),
            ("Nope", "null"),
            ("Section.presence", "visible"),
            ("A.rawValue", "3"),
            ("Out.isNull", "1"),
            ("A.name", "A"),
            ("$.name", "Out"),
            ("A.parent.name", "S"),
            ("Row[1].R", "r1"),
            ("S.Row[0].R", "r0"),
            ("Row[*].R", "r0"),
            ("Row[-1].R", "null"),
            ("Row[+1].R", "r1"),
            ("S..A", "3"),
            ("form1..R[1]", "r1"),
            ("$record.A", "null"),
            ("$record.S.A", "3"),
            ("$data.form1.S.B", "4"),
            ("!data.form1.S.Row[1].R", "r1"),
            ("Concat(S.*)", "34 ada "),
            ("#field[1]", "4"),
            ("$form.form1.S.A", "3"),
            ("xfa.resolveNode(\"A\").rawValue", "3"),
            ("S.resolveNode(\"Row[1].R\")", "r1"),
            ("$host.messageBox(\"hi\")", "1"),
            ("Row.instanceManager.addInstance(1)", "null"),
            ("Row.instanceManager.count", "2"),
            ("$event.name", ""),
        ] {
            assert_eq!(result(&mut form, script), expected, "{script}");
        }
    }

    #[test]
    fn built_in_functions() {
        let mut form = form();
        for (script, expected) in [
            ("Sum(A, B, 3)", "10"),
            ("sum(A, Out, Row[*].R, 1)", "4"),
            ("Sum(Out)", "null"),
            ("Avg(A, B)", "3.5"),
            ("Count(A, Out, B)", "2"),
            ("Max(A, B, 1)", "4"),
            ("MIN(A, B, 1)", "1"),
            ("Abs(-2)", "2"),
            ("Ceil(1.2)", "2"),
            ("Floor(-1.5)", "-2"),
            ("Round(2.675, 2)", "2.68"),
            ("Round(-2.5)", "-3"),
            ("Mod(-7, 3)", "-1"),
            ("Mod(7, 0)", "null"),
            ("Len(Name)", "5"),
            ("Upper(Ltrim(Name))", "ADA "),
            ("Rtrim(Name)", " ada"),
            ("Lower(\"AbC\")", "abc"),
            ("Upper(Out)", "null"),
            ("Left(\"abcdef\", 2)", "ab"),
            ("Right(\"abcdef\", 2)", "ef"),
            ("Substr(\"abcdef\", 2, 3)", "bcd"),
            ("Substr(\"abcdef\", 5, 9)", "ef"),
            ("Stuff(\"abcdef\", 2, 3, \"XY\")", "aXYef"),
            ("Stuff(\"abc\", 9, 1)", "abc"),
            ("Replace(\"a-b-c\", \"-\", \"+\")", "a+b+c"),
            ("Replace(\"a-b-c\", \"-\")", "abc"),
            ("At(\"abcdef\", \"cd\")", "3"),
            ("At(\"abcdef\", \"x\")", "0"),
            ("Space(3)", "   "),
            ("Str(3.14159, 6, 2)", "  3.14"),
            ("Str(12345, 3)", "***"),
            ("Str(42)", "        42"),
            ("Choose(2, \"a\", \"b\", \"c\")", "b"),
            ("Choose(9, \"a\")", ""),
            ("Oneof(A, 1, 3)", "1"),
            ("Oneof(\"x\", \"y\")", "0"),
            ("Within(A, 1, 5)", "1"),
            ("Within(\"c\", \"a\", \"b\")", "0"),
            ("HasValue(A)", "1"),
            ("HasValue(Out)", "0"),
            ("HasValue(\"  \")", "0"),
            ("Exists(A)", "1"),
            ("Exists(Nope)", "0"),
            ("Exists(\"Row[1].R\")", "1"),
            ("Ref(\"A\")", "3"),
            ("Null()", "null"),
            ("Format(\"z\", 1)", "1"),
            ("Date() >= 46000", "1"),
            ("NoSuchFunction(1, 2)", "null"),
            ("A.noSuchMethod(1)", "null"),
            ("Eval(\"1 + 1\")", "null"),
        ] {
            assert_eq!(result(&mut form, script), expected, "{script}");
        }
    }

    #[test]
    fn control_flow() {
        let mut form = form();
        let pick = "if (A > 5) then \"big\" elseif (A > 2) then \"mid\" else \"small\" endif";
        assert_eq!(result(&mut form, pick), "mid");
        assert_eq!(result(&mut form, &pick.replace("A >", "B >")), "mid");
        assert_eq!(
            result(&mut form, &pick.replace("A >", "Out >")),
            "small",
            "null reads as 0"
        );
        assert_eq!(
            result(&mut form, "IF (1) THEN \"x\" ENDIF"),
            "x",
            "keywords ignore case"
        );
        let looped = "var i = 0\nvar s = 0\nwhile (i < 5) do\n  i = i + 1\n  if (i == 3) then continue endif\n  s = s + i\nendwhile\ns";
        assert_eq!(result(&mut form, looped), "12");
        assert_eq!(
            result(
                &mut form,
                "var t = 0\nfor i = 1 upto 10 step 3 do t = t + i endfor\nt"
            ),
            "22"
        );
        assert_eq!(
            result(
                &mut form,
                "var t = \"\"\nfor var j = 3 downto 1 do t = Concat(t, j) endfor\nt"
            ),
            "321"
        );
        assert_eq!(
            result(
                &mut form,
                "var t = 0\nfor i = 1 upto 100 do if (i > 4) then break endif t = i endfor\nt"
            ),
            "4"
        );
        assert_eq!(
            result(
                &mut form,
                "var t = 0\nfor i = 1 upto 3 step 0 do t = 1 endfor\nt"
            ),
            "0",
            "a step of 0 runs nothing"
        );
        assert_eq!(
            result(&mut form, "var x\nx"),
            "",
            "a bare var is the empty string"
        );
        assert_eq!(
            result(&mut form, "A = 5"),
            "Undefined",
            "an assignment gives no value"
        );
        assert_eq!(value(&form, &at(A)), "5");
    }

    #[test]
    fn assignments_set_values_properties_and_variables() {
        let mut form = form();
        run_at(
            &mut form,
            OUT,
            "$.rawValue = A * 2\nSection.presence = \"hidden\"\nName = Upper(Ltrim(Name))\n\
             Row[1].R = \"second\"\nB.access = \"readOnly\"\nvar x = A\nx = 9\nA = A + 0.1 + 0.2",
        )
        .unwrap();
        assert_eq!(value(&form, &at(OUT)), "6");
        assert_eq!(value(&form, &at(NAME)), "ADA ");
        assert_eq!(value(&form, &at(A)), "3.3", "a local, then a tidy number");
        assert_eq!(value(&form, &NodeRef(vec![0, 6, 0])), "second");
        let Some(NodeView::Subform(s)) = view(&form, &at(SECTION)) else {
            panic!();
        };
        assert_eq!(s.common.presence, Presence::Hidden);
        let Some(NodeView::Field(b)) = view(&form, &at(B)) else {
            panic!();
        };
        assert!(b.common.read_only);
        run_at(&mut form, OUT, "$ = \"x\"\nNope = 1\nNope").unwrap();
        assert_eq!(value(&form, &at(OUT)), "x");
        // A variable holding Ref(B) is still a variable; its members are B's.
        run_at(&mut form, OUT, "var r = Ref(B)\nr = 7").unwrap();
        assert_eq!(value(&form, &at(B)), "4");
        run_at(&mut form, OUT, "var r = Ref(B)\nr.rawValue = 8").unwrap();
        assert_eq!(value(&form, &at(B)), "8");
        run_at(&mut form, OUT, "Out = Null()").unwrap();
        assert_eq!(value(&form, &at(OUT)), "", "null empties");
    }

    #[test]
    fn user_functions() {
        let mut form = form();
        assert_eq!(
            result(
                &mut form,
                "func Twice(x) do x * 2 endfunc\nTwice(A) + twice(1)"
            ),
            "8"
        );
        assert_eq!(
            result(
                &mut form,
                "Fact(5)\nfunc Fact(n) do\n  if (n <= 1) then return 1 endif\n  n * Fact(n - 1)\nendfunc"
            ),
            "120",
            "hoisted, recursive, with return"
        );
        assert_eq!(
            result(
                &mut form,
                "func Hide(s) do s.presence = \"hidden\" endfunc\nHide(Ref(Section))\n1"
            ),
            "1"
        );
        let Some(NodeView::Subform(s)) = view(&form, &at(SECTION)) else {
            panic!();
        };
        assert_eq!(s.common.presence, Presence::Hidden);
        assert_eq!(
            result(&mut form, "func F() do 2 endfunc\n\"before\"\nvar v = F()"),
            "before",
            "a call keeps the script's own value apart"
        );
    }

    #[test]
    fn unknown_constructs_cost_their_statement() {
        let mut form = form();
        run_at(
            &mut form,
            OUT,
            "A = 1\nB = ) junk (\nif (A == 1) then Out = \"yes\" endif\n\
             foreach x (1, 2) do Out = \"no\" endfor\nif (A ==) then\n  Out = \"bad\"\nendif\n\
             endif\nName = \"after\" ; a comment\n1 = 2\nName.x.y.z = 3",
        )
        .unwrap();
        assert_eq!(value(&form, &at(A)), "1");
        assert_eq!(value(&form, &at(B)), "4");
        assert_eq!(value(&form, &at(OUT)), "yes");
        assert_eq!(value(&form, &at(NAME)), "after");
        assert_eq!(
            parse("a b")[..],
            [
                Stmt::Expr(Expr::Name("a".into(), None)),
                Stmt::Expr(Expr::Name("b".into(), None))
            ]
        );
    }

    #[test]
    fn bounds_stop_runaway_scripts() {
        let mut form = form();
        let deep = format!("A = {}1{}\nB = 2", "(".repeat(200), ")".repeat(200));
        run_at(&mut form, OUT, &deep).unwrap();
        assert_eq!(value(&form, &at(A)), "3", "too deep: skipped");
        assert_eq!(value(&form, &at(B)), "2");
        let nested = format!(
            "{}Out = 1{}\nName = 2",
            "if (1) then ".repeat(100),
            " endif".repeat(100)
        );
        run_at(&mut form, OUT, &nested).unwrap();
        assert_eq!(value(&form, &at(NAME)), "2");
        assert!(matches!(
            run_at(&mut form, OUT, "while (1) do A = 1 endwhile"),
            Err(Stopped)
        ));
        assert!(matches!(
            run_at(&mut form, OUT, "for i = 1 upto 1e12 do endfor"),
            Err(Stopped)
        ));
        run_at(&mut form, OUT, "func F(n) do F(n + 1) endfunc\nF(0)\nB = 5").unwrap();
        assert_eq!(
            value(&form, &at(B)),
            "5",
            "recursion stops at the call depth"
        );
        run_at(
            &mut form,
            OUT,
            "var s = \"x\"\nfor i = 1 upto 40 do s = Concat(s, s) endfor\nOut = Len(s)",
        )
        .unwrap();
        assert_eq!(
            value(&form, &at(OUT)),
            "0",
            "past the bound the string is null"
        );
        run_at(
            &mut form,
            OUT,
            "Out = Space(1e12)\nName = Replace(Space(100000), \" \", Space(100))",
        )
        .unwrap();
        assert_eq!(value(&form, &at(OUT)), "");
        assert_eq!(value(&form, &at(NAME)), "");
        // One runner's scripts share its budget; each has its own bound.
        let ctx = at(OUT);
        let mut runner = Runner::new(&mut form);
        assert!(matches!(
            runner.run_formcalc("while (1) do endwhile", &ctx, EventInfo::default()),
            Err(Stopped)
        ));
        assert!(!runner.exhausted());
        assert!(
            runner
                .run_formcalc("B = 6", &ctx, EventInfo::default())
                .is_ok()
        );
    }

    const ROWS: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.6/">
<subform name="form1" layout="tb">
  <variables>
    <script name="Calc">
      func Twice(x) do x * 2 endfunc
      func Total() do Twice(Sum(T.Row[*].Amount)) / 2 endfunc
    </script>
  </variables>
  <subform name="T" layout="tb">
    <subform name="Row" layout="tb">
      <occur min="0" max="-1"/>
      <field name="Amount"><ui><numericEdit/></ui></field>
      <field name="Running"><ui><textEdit/></ui></field>
    </subform>
    <field name="Total"><ui><textEdit/></ui>
      <calculate><script>Calc.Total()</script></calculate>
    </field>
  </subform>
</subform>
</template>"#;

    fn rows_form(amounts: &[&str]) -> Form {
        let rows: String = amounts
            .iter()
            .map(|a| format!("<Row><Amount>{a}</Amount></Row>"))
            .collect();
        let datasets = format!(
            "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><xfa:data>\
             <form1><T>{rows}</T></form1></xfa:data></xfa:datasets>"
        );
        let mut form = template::read(ROWS).unwrap();
        bind::bind(&mut form, Some(&xml::parse(&datasets).unwrap())).unwrap();
        form
    }

    fn result_at(form: &mut Form, ctx: &NodeRef, script: &str) -> String {
        let mut runner = Runner::new(form);
        match runner
            .run_formcalc(script, ctx, EventInfo::default())
            .expect("within budget")
        {
            Value::Num(n) => num_text(n),
            Value::Str(s) => s.to_string(),
            Value::Null => "null".into(),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn lists_aggregate_skip_empty_rows_and_resolve_data() {
        let mut form = rows_form(&["10", "", "32.5"]);
        let total = NodeRef(vec![0, 3]);
        for (script, expected) in [
            ("Sum(Row[*].Amount)", "42.5"),
            ("Count(Row[*].Amount)", "2"),
            ("Avg(Row[*].Amount)", "21.25"),
            ("Max(Row[*].Amount)", "32.5"),
            ("Min(Row[*].Amount, 50)", "10"),
            ("Row[*].Amount", "10"),
            ("Row[*].Amount + 1", "11"),
            ("Concat(Row[*].Amount)", "1032.5"),
            ("Sum(T..Amount[*])", "42.5"),
            ("form1..Amount", "10"),
            ("form1..Amount[2]", "32.5"),
            ("Count(Row[0].*)", "1"),
            ("Sum($record.T.Row[*].Amount)", "42.5"),
            ("Count($record.T.Row[*].Amount)", "2"),
            ("$data.form1.T.Row[2].Amount", "32.5"),
            ("Exists(Row[*])", "1"),
            ("Exists(Row[5])", "0"),
            ("Exists(\"Row[*].Amount\")", "1"),
            ("Exists(\"$record.T.Row[1]\")", "1"),
            ("HasValue(Row[*].Amount)", "1"),
            ("Ref(\"Row[2].Amount\")", "32.5"),
            ("Row[0].Amount = 5\n$record.T.Row[0].Amount", "5"),
            ("Calc.Twice(4)", "8"),
            ("Calc.Total()", "37.5"),
            ("Calc.Missing(1)", "null"),
        ] {
            assert_eq!(result_at(&mut form, &total, script), expected, "{script}");
        }
        // Relative indexes count from the row the script runs in.
        let running = |row: usize| NodeRef(vec![0, row, 1]);
        assert_eq!(result_at(&mut form, &running(1), "Row[-1].Amount"), "5");
        assert_eq!(result_at(&mut form, &running(1), "Row[+1].Amount"), "32.5");
        assert_eq!(result_at(&mut form, &running(0), "Row[-1].Amount"), "null");
    }

    #[test]
    fn foreach_runs_over_lists() {
        let mut form = rows_form(&["10", "", "32.5"]);
        let total = NodeRef(vec![0, 3]);
        for (script, expected) in [
            (
                "var t = 0\nforeach a in (Row[*].Amount, 100) do t = t + a endfor\nt",
                "142.5",
            ),
            (
                "var n = 0\nforeach r in (Row[*]) do\n  if (r.Amount == 32.5) then break endif\n  n = n + 1\nendfor\nn",
                "2",
            ),
            (
                "var c = \"\"\nforeach r in (Row[*]) do\n  if (r.Amount.isNull) then continue endif\n  c = Concat(c, r.Amount, \";\")\nendfor\nc",
                "10;32.5;",
            ),
            (
                "foreach a in (Row[*].Amount) do a.rawValue = 7 endfor\nSum(Row[*].Amount)",
                "21",
            ),
            ("var n = 0\nforeach x in () do n = 1 endfor\nn", "0"),
        ] {
            assert_eq!(result_at(&mut form, &total, script), expected, "{script}");
        }
    }

    #[test]
    fn script_objects_calculate_and_hostile_lists_stop() {
        let mut form = rows_form(&["1", "2", "3"]);
        let mut runner = Runner::new(&mut form);
        assert_eq!(runner.calculate().ran, 2, "it set Total: a second pass ran");
        drop(runner);
        assert_eq!(value(&form, &NodeRef(vec![0, 3])), "6");
        let many = vec!["1"; 2000];
        let mut form = rows_form(&many);
        let total = NodeRef(vec![0, 2000]);
        let mut runner = Runner::new(&mut form);
        assert!(matches!(
            runner.run_formcalc(
                "foreach a in (Row[*]) do foreach b in (Row[*].Amount) do endfor endfor",
                &total,
                EventInfo::default()
            ),
            Err(Stopped)
        ));
        assert!(
            runner
                .run_formcalc("Sum(Row[*].Amount)", &total, EventInfo::default())
                .is_ok(),
            "one list of 2,000 is within the budget"
        );
    }

    #[test]
    fn dates_times_pictures_words_and_money() {
        let mut form = form();
        for (script, expected) in [
            ("Date2Num(\"2026-11-05\")", "46330"),
            ("Date2Num(\"1/1/1900\", \"D/M/YYYY\")", "1"),
            (
                "Num2Date(Date2Num(\"2026-11-05\"), \"MMM D, YYYY\")",
                "Nov 5, 2026",
            ),
            ("Num2Date(1)", "Jan 1, 1900"),
            (
                "Num2Date(Date2Num(\"Mar 15, 1996\"), \"YYYY-MM-DD\")",
                "1996-03-15",
            ),
            ("Date2Num(\"2000-03-01\") - Date2Num(\"2000-02-28\")", "2"),
            ("Date2Num(\"2100-03-01\") - Date2Num(\"2100-02-28\")", "1"),
            ("Date2Num(\"1900-02-29\")", "0"),
            ("Date2Num(\"not a date\")", "0"),
            ("Num2Date(0)", ""),
            (
                "Num2Date(Date2Num(\"2024-02-28\") + 1, \"YYYY-MM-DD\")",
                "2024-02-29",
            ),
            ("IsoDate2Num(\"1900-03-01\")", "60"),
            ("DateFmt(1)", "M/D/YY"),
            ("DateFmt()", "MMM D, YYYY"),
            ("LocalDateFmt(4, \"en_US\")", "EEEE, MMMM D, YYYY"),
            ("TimeFmt(1)", "h:MM A"),
            ("Time2Num(\"13:13:13\", \"HH:MM:SS\")", "47593001"),
            ("Time2Num(\"1:13:13 PM\")", "47593001"),
            ("Num2Time(47593001, \"HH:MM:SS\")", "13:13:13"),
            ("Num2GMTime(1, \"HH:MM:SS\")", "00:00:00"),
            ("IsoTime2Num(\"13:13:13Z\")", "47593001"),
            ("Time() > 0", "1"),
            ("Format(\"$z,zz9.99\", 1234.5)", "$1,234.50"),
            ("Format(\"MMM D, YYYY\", \"2026-11-05\")", "Nov 5, 2026"),
            ("Parse(\"$z,zz9.99\", \"$1,234.50\") + 0.5", "1235"),
            ("Parse(\"MMM D, YYYY\", \"Nov 5, 2026\")", "2026-11-05"),
            ("Parse(\"9999\", \"abc\")", ""),
            ("WordNum(1234)", "One Thousand Two Hundred and Thirty-four"),
            ("WordNum(12.5, 2)", "Twelve Dollars And Fifty Cents"),
            ("Round(Pmt(150000, 0.0475 / 12, 300), 2)", "855.18"),
            ("Round(PV(400, 0.1 / 12, 360), 2)", "45580.33"),
            ("Round(FV(400, 0.1 / 12, 360), 2)", "904195.17"),
            ("Round(IPmt(30000, 0.085, 295.50, 7, 3), 2)", "624.88"),
            ("Round(NPV(0.065, 5000), 2)", "4694.84"),
            ("Rate(110, 100, 1)", "0.1"),
            ("Pmt(1000, 0, 12)", "null"),
            ("Pmt(1000, null, 12)", "null"),
            ("UnitType(\"2.54cm\")", "cm"),
            ("UnitValue(\"2in\", \"cm\")", "5.08"),
            ("UnitValue(\"72pt\")", "1"),
            ("Encode(\"a b&c\")", "a%20b%26c"),
            ("Decode(\"&lt;b&gt;\", \"html\")", "<b>"),
            ("Get(\"http://example.com\")", "null"),
        ] {
            assert_eq!(result(&mut form, script), expected, "{script}");
        }
    }
}
