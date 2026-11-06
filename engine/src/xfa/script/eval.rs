// SPDX-License-Identifier: MIT
//! Runs a parsed script against the bound form: the subset of XFA's
//! scripting object model that Designer forms use to show, hide and set
//! values. Names resolve as XFA scopes them (the node's children, then
//! each ancestor's, unnamed subforms transparent); `this` is the node the
//! event belongs to, or a script object's subform inside its functions.
//!
//! Rules:
//! - Nodes carry `rawValue` (`value` reads the same), `presence`,
//!   `access`, `isNull`, `name`, `parent`, `index`, `instanceManager`,
//!   `resolveNode`, their children by name
//!   and a subform's variables. Any other property reads `undefined` and
//!   ignores a write.
//! - A repeating subform's `instanceManager` (or `_Name`) has `count`,
//!   `min`, `max`, and `addInstance`, `removeInstance(i)`, `setInstances(n)`,
//!   which change nothing in the form: they are recorded
//!   (`take_instance_ops`) for the caller to apply to the data and bind
//!   again; `count` counts the recorded ones. One that would pass `occur`
//!   (or `limits::MAX_INSTANCES`), or whose instances are not bound to
//!   data groups of their name under one parent, is refused silently.
//!   `moveInstance` and `insertInstance` do nothing.
//! - `xfa.resolveNode` reads a SOM expression relative to `this`
//!   (`som`): `name`, `name[n]`, `name[*]`, relative indexes, `..name`,
//!   `*`, `#class[n]`, `parent`, `xfa.form`, `$form`, `$`, `$record`,
//!   `$data`, `xfa.datasets`, `!name`; it gives the first node named.
//!   `resolveNodes` gives them all, a list with `length`, `item(i)` and
//!   `[i]`. Predicates (`[a > 1]`, `.(...)`) name nothing: `null`, an
//!   empty list. Data nodes have `value`, `name`, `isNull`, `parent`,
//!   `index` and their children by name, and are not written.
//! - `xfa.host` and `xfa.layout` members are functions that return
//!   `undefined`; `xfa.event` gives `target`, `newText`, `name` and
//!   empty strings for the rest.
//! - Conversion and comparison follow JavaScript for the values that
//!   exist here (numbers, strings, booleans, null, undefined, nodes).
//!   A node converts to its raw value's text.
//! - Calling anything that is not a function, reading a member of a
//!   primitive it does not have, or an unsupported construct is
//!   `undefined`, never an exception: the rest of the script still runs.
//! - Regular expressions (`regex`): literals, `RegExp(...)` and
//!   `new RegExp(...)`; `test`, `exec`, `lastIndex` with `g`; strings'
//!   `match`, `search`, `replace` (a string, `$&`/`$1`.., or a function as
//!   the replacement) and `split` with a regex; a list's `join`. A match
//!   spends the script's steps; one past them stops the script.
//! - Bounded: `MAX_STEPS` evaluations per runner, `MAX_SCRIPT_STEPS` per
//!   script, `MAX_CALL_DEPTH` calls deep, `MAX_STRING_CHARS` per string,
//!   `MAX_LOCALS` variables per frame. Past a bound the script stops
//!   where it is (`Stopped`); what it did stands.

use std::cell::{Cell, OnceCell};
use std::collections::HashMap;
use std::rc::Rc;

use super::formcalc::FcObject;
use super::parse::{self, BinOp, Expr, Function, Stmt, UnOp};
use super::regex::{Captures, Regex};
use super::som::DataRef;
use crate::xfa::limits::{MAX_INSTANCE_OPS, MAX_INSTANCES};
use crate::xfa::types::{
    DataPath, Draw, ExclGroup, Field, Form, InstanceOp, Node, Presence, Subform, Value as XValue,
};

/// Expression and statement evaluations one runner performs, at most.
pub const MAX_STEPS: usize = 4_000_000;
/// Evaluations one script run performs, at most.
pub const MAX_SCRIPT_STEPS: usize = 400_000;
/// Function calls nested, at most.
pub const MAX_CALL_DEPTH: usize = 20;
/// Characters a string value holds, at most (a longer result is `undefined`).
pub const MAX_STRING_CHARS: usize = 1 << 20;
/// Variables one frame holds, at most.
pub const MAX_LOCALS: usize = 4096;
/// Items a `split` or a global `match` gives, at most.
pub const MAX_SPLIT: usize = 100_000;
/// Unnamed subforms a name lookup descends through, at most.
const MAX_LOOKUP_DEPTH: usize = 32;

/// A node of the bound form, as the indexes down from the root subform:
/// into `Subform::children`, or into `ExclGroup::fields` for a radio
/// button. The root is the empty path.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct NodeRef(pub Vec<usize>);

impl NodeRef {
    pub fn child(&self, index: usize) -> NodeRef {
        let mut path = self.0.clone();
        path.push(index);
        NodeRef(path)
    }
    pub fn parent(&self) -> Option<NodeRef> {
        let mut path = self.0.clone();
        path.pop()?;
        Some(NodeRef(path))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum NodeView<'a> {
    Subform(&'a Subform),
    Field(&'a Field),
    Draw(&'a Draw),
    Group(&'a ExclGroup),
}

impl NodeView<'_> {
    pub fn name(&self) -> &str {
        match self {
            NodeView::Subform(s) => &s.common.name,
            NodeView::Field(f) => &f.common.name,
            NodeView::Draw(d) => &d.common.name,
            NodeView::Group(g) => &g.common.name,
        }
    }
    pub fn class(&self) -> &'static str {
        match self {
            NodeView::Subform(_) => "subform",
            NodeView::Field(_) => "field",
            NodeView::Draw(_) => "draw",
            NodeView::Group(_) => "exclGroup",
        }
    }
    fn presence(&self) -> Presence {
        match self {
            NodeView::Subform(s) => s.common.presence,
            NodeView::Field(f) => f.common.presence,
            NodeView::Draw(d) => d.common.presence,
            NodeView::Group(g) => g.common.presence,
        }
    }
    fn read_only(&self) -> bool {
        match self {
            NodeView::Subform(s) => s.common.read_only,
            NodeView::Field(f) => f.common.read_only,
            NodeView::Draw(d) => d.common.read_only,
            NodeView::Group(g) => g.common.read_only,
        }
    }
    pub fn child_count(&self) -> usize {
        match self {
            NodeView::Subform(s) => s.children.len(),
            NodeView::Group(g) => g.fields.len(),
            _ => 0,
        }
    }
}

enum NodeMut<'a> {
    Subform(&'a mut Subform),
    Field(&'a mut Field),
    Draw(&'a mut Draw),
    Group(&'a mut ExclGroup),
}

/// The node at `at`, when the path exists.
pub fn view<'a>(form: &'a Form, at: &NodeRef) -> Option<NodeView<'a>> {
    let mut current = NodeView::Subform(&form.root);
    for &index in &at.0 {
        current = match current {
            NodeView::Subform(s) => match s.children.get(index)? {
                Node::Subform(x) => NodeView::Subform(x),
                Node::Field(x) => NodeView::Field(x),
                Node::Draw(x) => NodeView::Draw(x),
                Node::ExclGroup(x) => NodeView::Group(x),
            },
            NodeView::Group(g) => NodeView::Field(g.fields.get(index)?),
            _ => return None,
        };
    }
    Some(current)
}

fn view_mut<'a>(form: &'a mut Form, at: &NodeRef) -> Option<NodeMut<'a>> {
    let mut current = NodeMut::Subform(&mut form.root);
    for &index in &at.0 {
        current = match current {
            NodeMut::Subform(s) => match s.children.get_mut(index)? {
                Node::Subform(x) => NodeMut::Subform(x),
                Node::Field(x) => NodeMut::Field(x),
                Node::Draw(x) => NodeMut::Draw(x),
                Node::ExclGroup(x) => NodeMut::Group(x),
            },
            NodeMut::Group(g) => NodeMut::Field(g.fields.get_mut(index)?),
            _ => return None,
        };
    }
    Some(current)
}

/// A script value.
#[derive(Clone, Debug)]
pub enum Value {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Str(Rc<str>),
    Node(NodeRef),
    /// `xfa`.
    Xfa,
    /// `xfa.host`, `xfa.layout` and other objects whose members do nothing.
    Host,
    /// `xfa.event`.
    Event,
    /// `xfa.form`: the model holding the root subform.
    FormModel,
    /// A `variables/script` object: its owner subform and name.
    ScriptObject(NodeRef, Rc<str>),
    /// A `variables/text` variable: its owner subform and name.
    TextVar(NodeRef, Rc<str>),
    /// A function and the node its names resolve from (`this`).
    Function(Rc<Function>, NodeRef),
    Builtin(Builtin),
    /// A subform's `instanceManager`.
    InstanceManager(NodeRef),
    /// What a reference that names several nodes gives (`Row[*]`,
    /// `xfa.resolveNodes`): form or data nodes, at most `som::MAX_LIST`.
    List(Rc<[Value]>),
    /// A data node (`$record`, `$data`, `!name`, `xfa.datasets`).
    Data(DataRef),
    /// A regular expression and its `lastIndex`.
    RegExp(Rc<RegExpValue>),
}

/// A regular expression value: the pattern read, and where a global
/// `exec` or `test` goes on from.
#[derive(Debug)]
pub struct RegExpValue {
    pub re: Regex,
    pub last_index: Cell<usize>,
}

#[derive(Clone, Debug)]
pub enum Builtin {
    /// `resolveNode`, relative to this node.
    ResolveNode(NodeRef),
    /// `resolveNodes`, relative to this node.
    ResolveNodes(NodeRef),
    /// A list's `item(i)`.
    Item(Rc<[Value]>),
    /// A method that does nothing and returns `undefined`.
    Noop,
    /// `xfa.host.messageBox(text, ...)`: records `text`, returns 1 (OK).
    MessageBox,
    /// A string method on this string.
    StrMethod(Rc<str>, &'static str),
    /// A number method on this number.
    NumMethod(f64, &'static str),
    /// An `instanceManager` method (`addInstance`, `removeInstance`,
    /// `setInstances`) of the subform instance at this node.
    Instances(NodeRef, &'static str),
    /// `RegExp`, called or with `new`.
    RegExpCtor,
    /// `test` or `exec` of this regular expression.
    RegexMethod(Rc<RegExpValue>, &'static str),
    /// A list's `join`.
    Join(Rc<[Value]>),
}

/// The event a script runs for (`xfa.event`).
#[derive(Clone, Debug, Default)]
pub struct EventInfo {
    pub activity: String,
    pub target: NodeRef,
    /// `newText` of a `change` event: the value being set.
    pub new_text: Option<String>,
}

/// The run stopped at a bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stopped;

enum Flow {
    Next,
    Break,
    Continue,
    Return(Value),
}

struct Frame {
    locals: HashMap<String, Value>,
    this: NodeRef,
    depth: usize,
}

impl Frame {
    fn new(this: NodeRef, depth: usize) -> Self {
        Frame {
            locals: HashMap::new(),
            this,
            depth,
        }
    }
    fn set(&mut self, name: &str, value: Value) {
        if self.locals.len() >= MAX_LOCALS && !self.locals.contains_key(name) {
            return;
        }
        self.locals.insert(name.to_string(), value);
    }
}

/// A repeating subform's instances as bound.
struct Instances {
    count: usize,
    /// Their data paths, when each is bound to a data group of the
    /// subform's name under one parent.
    paths: Option<Rc<[DataPath]>>,
}

/// A repeating subform's instances, for an `instanceManager` method.
struct InstanceGroup {
    /// Its parent and name (`Runner::instance_delta`'s key).
    key: (NodeRef, String),
    /// Its instances' data paths as bound.
    instances: Rc<[DataPath]>,
    min: usize,
    /// The most instances it may have.
    cap: usize,
}

struct ScriptObject {
    functions: HashMap<String, Rc<Function>>,
    /// Its top-level `var`s whose values are constants (`const_value`):
    /// a form's message table (`scriptUtils.i18n.msgTelE`).
    vars: HashMap<String, Value>,
}

/// Runs scripts against one form.
pub struct Runner<'f> {
    pub form: &'f mut Form,
    /// Text variables' current values (the template's until set).
    text_vars: HashMap<(NodeRef, String), String>,
    /// Script objects, parsed on first use.
    objects: HashMap<(NodeRef, String), Rc<ScriptObject>>,
    /// Evaluations left for this runner.
    steps: usize,
    /// Evaluations left for the current script.
    script_steps: usize,
    event: EventInfo,
    /// `instanceManager` operations recorded, in order.
    instance_ops: Vec<InstanceOp>,
    /// Instances the recorded operations add (or remove) per repeating
    /// subform: its parent and name.
    instance_delta: HashMap<(NodeRef, String), isize>,
    /// Repeating subforms' instances, read on first use.
    instance_groups: HashMap<(NodeRef, String), Rc<Instances>>,
    /// Data paths to the fields bound to them (`som`), built on first use.
    pub(super) bound: OnceCell<HashMap<DataPath, NodeRef>>,
    /// Nodes references visited since the last `charge`.
    pub(super) visits: Cell<usize>,
    /// FormCalc script objects, read on first use (`formcalc`).
    pub(super) fc_objects: HashMap<(NodeRef, String), Rc<FcObject>>,
    /// Messages recorded since the last `take_messages` (`messageBox`,
    /// failed validations), `MAX_MESSAGES` at most.
    messages: Vec<String>,
}

/// Messages one run of handlers records, at most.
pub const MAX_MESSAGES: usize = 16;
/// Bytes of one message, at most (cut at a character boundary).
pub const MAX_MESSAGE_BYTES: usize = 1024;

impl<'f> Runner<'f> {
    pub fn new(form: &'f mut Form) -> Self {
        Runner {
            form,
            text_vars: HashMap::new(),
            objects: HashMap::new(),
            steps: MAX_STEPS,
            script_steps: MAX_SCRIPT_STEPS,
            event: EventInfo::default(),
            instance_ops: Vec::new(),
            instance_delta: HashMap::new(),
            instance_groups: HashMap::new(),
            bound: OnceCell::new(),
            visits: Cell::new(0),
            fc_objects: HashMap::new(),
            messages: Vec::new(),
        }
    }

    /// Records a message for the user (`messageBox`, a failed
    /// validation): trimmed, cut to `MAX_MESSAGE_BYTES`; dropped when
    /// empty or once `MAX_MESSAGES` are held.
    pub fn record_message(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() || self.messages.len() >= MAX_MESSAGES {
            return;
        }
        let mut end = text.len().min(MAX_MESSAGE_BYTES);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.messages.push(text[..end].to_string());
    }

    /// The messages recorded since the last call, in order.
    pub fn take_messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }

    /// The `instanceManager` operations recorded since the last call, in
    /// order; the counts they changed are forgotten.
    pub fn take_instance_ops(&mut self) -> Vec<InstanceOp> {
        self.instance_delta.clear();
        std::mem::take(&mut self.instance_ops)
    }

    /// Whether the runner's own budget is spent (no script runs further).
    pub fn exhausted(&self) -> bool {
        self.steps == 0
    }

    /// Runs `source` with `this` = `ctx` for `event`. `Err(Stopped)` when
    /// a bound stopped it part way.
    pub fn run(&mut self, source: &str, ctx: &NodeRef, event: EventInfo) -> Result<(), Stopped> {
        self.run_value(source, ctx, event).map(|_| ())
    }

    /// `run`, returning the script's completion value as a `calculate` or
    /// `validate` script gives it: the value of the last top-level
    /// statement when that is an expression statement, else `Undefined`
    /// (also when a `return` or `break` ends the script early).
    pub fn run_value(
        &mut self,
        source: &str,
        ctx: &NodeRef,
        event: EventInfo,
    ) -> Result<Value, Stopped> {
        self.begin(event)?;
        let program = parse::parse(source);
        let mut frame = Frame::new(ctx.clone(), 0);
        self.hoist(&program, &mut frame);
        let Some((Stmt::Expr(last), body)) = program.split_last() else {
            self.exec_block(&program, &mut frame)?;
            return Ok(Value::Undefined);
        };
        match self.exec_block(body, &mut frame)? {
            Flow::Next => {}
            _ => return Ok(Value::Undefined),
        }
        self.tick()?;
        self.eval(last, &mut frame)
    }

    /// Starts a script run (JavaScript or FormCalc): the event it runs
    /// for and a fresh per-script budget; `Err(Stopped)` when the runner's
    /// own budget is spent.
    pub(super) fn begin(&mut self, event: EventInfo) -> Result<(), Stopped> {
        if self.steps == 0 {
            return Err(Stopped);
        }
        self.event = event;
        self.script_steps = MAX_SCRIPT_STEPS;
        Ok(())
    }

    /// `n` evaluations off both budgets (a list's items), and one for
    /// every node a reference visited since the last charge (`som`);
    /// `Err(Stopped)` past either.
    pub(super) fn charge(&mut self, n: usize) -> Result<(), Stopped> {
        let n = n.saturating_add(self.visits.take());
        if self.steps < n || self.script_steps < n {
            self.steps -= self.steps.min(n);
            self.script_steps = 0;
            return Err(Stopped);
        }
        self.steps -= n;
        self.script_steps -= n;
        Ok(())
    }

    /// One evaluation off both budgets (with the nodes references
    /// visited since the last charge); `Err(Stopped)` past either.
    pub(super) fn tick(&mut self) -> Result<(), Stopped> {
        if self.visits.get() > 0 {
            return self.charge(1);
        }
        if self.steps == 0 || self.script_steps == 0 {
            return Err(Stopped);
        }
        self.steps -= 1;
        self.script_steps -= 1;
        Ok(())
    }

    fn hoist(&mut self, body: &[Stmt], frame: &mut Frame) {
        for stmt in body {
            if let Stmt::Function(f) = stmt {
                frame.set(&f.name, Value::Function(f.clone(), frame.this.clone()));
            }
        }
    }

    // ------------------------------------------------------------ statements

    fn exec_block(&mut self, body: &[Stmt], frame: &mut Frame) -> Result<Flow, Stopped> {
        for stmt in body {
            match self.exec(stmt, frame)? {
                Flow::Next => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Next)
    }

    fn exec(&mut self, stmt: &Stmt, frame: &mut Frame) -> Result<Flow, Stopped> {
        self.tick()?;
        match stmt {
            Stmt::Expr(e) => {
                self.eval(e, frame)?;
            }
            Stmt::Var(decls) => {
                for (name, init) in decls {
                    match init {
                        Some(e) => {
                            let v = self.eval(e, frame)?;
                            frame.set(name, v);
                        }
                        None => {
                            if !frame.locals.contains_key(name) {
                                frame.set(name, Value::Undefined);
                            }
                        }
                    }
                }
            }
            Stmt::If(cond, then, otherwise) => {
                let c = self.eval(cond, frame)?;
                if truthy(&c) {
                    return self.exec(then, frame);
                } else if let Some(otherwise) = otherwise {
                    return self.exec(otherwise, frame);
                }
            }
            Stmt::Block(body) => return self.exec_block(body, frame),
            Stmt::Switch(value, cases) => {
                let v = self.eval(value, frame)?;
                let mut start = None;
                for (index, (label, _)) in cases.iter().enumerate() {
                    let Some(label) = label else {
                        continue;
                    };
                    let l = self.eval(label, frame)?;
                    if strict_eq(&v, &l) {
                        start = Some(index);
                        break;
                    }
                }
                let start = start.or_else(|| cases.iter().position(|(label, _)| label.is_none()));
                if let Some(start) = start {
                    for (_, body) in &cases[start..] {
                        match self.exec_block(body, frame)? {
                            Flow::Next => {}
                            Flow::Break => break,
                            other => return Ok(other),
                        }
                    }
                }
            }
            Stmt::While(cond, body) => loop {
                self.tick()?;
                let c = self.eval(cond, frame)?;
                if !truthy(&c) {
                    break;
                }
                match self.exec(body, frame)? {
                    Flow::Break => break,
                    Flow::Return(v) => return Ok(Flow::Return(v)),
                    Flow::Next | Flow::Continue => {}
                }
            },
            Stmt::DoWhile(body, cond) => loop {
                self.tick()?;
                match self.exec(body, frame)? {
                    Flow::Break => break,
                    Flow::Return(v) => return Ok(Flow::Return(v)),
                    Flow::Next | Flow::Continue => {}
                }
                let c = self.eval(cond, frame)?;
                if !truthy(&c) {
                    break;
                }
            },
            Stmt::For(init, cond, update, body) => {
                if let Some(init) = init {
                    self.exec(init, frame)?;
                }
                loop {
                    self.tick()?;
                    if let Some(cond) = cond {
                        let c = self.eval(cond, frame)?;
                        if !truthy(&c) {
                            break;
                        }
                    }
                    match self.exec(body, frame)? {
                        Flow::Break => break,
                        Flow::Return(v) => return Ok(Flow::Return(v)),
                        Flow::Next | Flow::Continue => {}
                    }
                    if let Some(update) = update {
                        self.eval(update, frame)?;
                    }
                }
            }
            Stmt::Return(value) => {
                let v = match value {
                    Some(e) => self.eval(e, frame)?,
                    None => Value::Undefined,
                };
                return Ok(Flow::Return(v));
            }
            Stmt::Break => return Ok(Flow::Break),
            Stmt::Continue => return Ok(Flow::Continue),
            Stmt::Function(f) => {
                frame.set(&f.name, Value::Function(f.clone(), frame.this.clone()));
            }
            Stmt::Empty | Stmt::Skipped => {}
        }
        Ok(Flow::Next)
    }

    // ----------------------------------------------------------- expressions

    fn eval(&mut self, expr: &Expr, frame: &mut Frame) -> Result<Value, Stopped> {
        self.tick()?;
        Ok(match expr {
            Expr::Num(n) => Value::Num(*n),
            Expr::Str(s) => Value::Str(Rc::from(s.as_str())),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Null => Value::Null,
            Expr::Undefined | Expr::Unsupported => Value::Undefined,
            Expr::This => Value::Node(frame.this.clone()),
            Expr::Ident(name) => match frame.locals.get(name) {
                Some(v) => v.clone(),
                None => self.global(name, &frame.this),
            },
            Expr::Member(object, name) => {
                let object = self.eval(object, frame)?;
                self.member(&object, name, frame)
            }
            Expr::Index(object, index) => {
                let object = self.eval(object, frame)?;
                let index = self.eval(index, frame)?;
                match (&object, &index) {
                    (_, Value::Str(name)) => self.member(&object, name, frame),
                    (Value::Str(s), Value::Num(n)) => char_at(s, *n),
                    (Value::List(items), Value::Num(n)) => list_item(items, *n),
                    // `Row[1]`: the n-th same-named sibling of the node.
                    (Value::Node(at), Value::Num(n)) if *n >= 0.0 && n.fract() == 0.0 => {
                        let found = at.parent().and_then(|parent| {
                            let name = view(self.form, at)?.name().to_string();
                            self.find_named(&parent, &name, *n as usize)
                        });
                        found.map_or(Value::Undefined, Value::Node)
                    }
                    _ => Value::Undefined,
                }
            }
            Expr::Call(callee, args) => {
                let function = match &**callee {
                    Expr::Member(object, name) => {
                        let object = self.eval(object, frame)?;
                        self.member(&object, name, frame)
                    }
                    other => self.eval(other, frame)?,
                };
                let mut values = Vec::with_capacity(args.len());
                for arg in args {
                    values.push(self.eval(arg, frame)?);
                }
                self.call(function, values, frame)?
            }
            Expr::Regex(pattern, flags) => {
                self.charge(pattern.len() / 16)?;
                regexp(pattern, flags)
            }
            Expr::New(callee, args) => match &**callee {
                Expr::Ident(name) if name == "RegExp" && !frame.locals.contains_key(name) => {
                    let mut values = Vec::with_capacity(args.len());
                    for arg in args {
                        values.push(self.eval(arg, frame)?);
                    }
                    self.call(Value::Builtin(Builtin::RegExpCtor), values, frame)?
                }
                _ => Value::Undefined,
            },
            Expr::Unary(op, operand) => {
                let v = self.eval(operand, frame)?;
                match op {
                    UnOp::Not => Value::Bool(!truthy(&v)),
                    UnOp::Neg => Value::Num(-self.to_number(&v)),
                    UnOp::Plus => Value::Num(self.to_number(&v)),
                    UnOp::TypeOf => Value::Str(Rc::from(type_of(&v))),
                }
            }
            Expr::Binary(op, left, right) => {
                let l = self.eval(left, frame)?;
                let r = self.eval(right, frame)?;
                self.binary(*op, &l, &r)
            }
            Expr::Logical(and, left, right) => {
                let l = self.eval(left, frame)?;
                if truthy(&l) == *and {
                    self.eval(right, frame)?
                } else {
                    l
                }
            }
            Expr::Assign(op, target, value) => {
                let mut v = self.eval(value, frame)?;
                if let Some(op) = op {
                    let current = self.eval(target, frame)?;
                    v = self.binary(*op, &current, &v);
                }
                self.assign(target, v.clone(), frame)?;
                v
            }
            Expr::Update(prefix, increment, target) => {
                let current = self.eval(target, frame)?;
                let current = self.to_number(&current);
                let next = if *increment {
                    current + 1.0
                } else {
                    current - 1.0
                };
                self.assign(target, Value::Num(next), frame)?;
                Value::Num(if *prefix { next } else { current })
            }
            Expr::Cond(cond, then, otherwise) => {
                let c = self.eval(cond, frame)?;
                if truthy(&c) {
                    self.eval(then, frame)?
                } else {
                    self.eval(otherwise, frame)?
                }
            }
            Expr::Function(f) => Value::Function(f.clone(), frame.this.clone()),
        })
    }

    /// A name that is not a local: `xfa`, then the XFA scope chain.
    fn global(&self, name: &str, ctx: &NodeRef) -> Value {
        match name {
            "xfa" => Value::Xfa,
            "$" => Value::Node(ctx.clone()),
            "RegExp" => Value::Builtin(Builtin::RegExpCtor),
            _ => self.lookup_scope(ctx, name),
        }
    }

    fn assign(&mut self, target: &Expr, value: Value, frame: &mut Frame) -> Result<(), Stopped> {
        match target {
            Expr::Ident(name) => frame.set(name, value),
            Expr::Member(object, name) => {
                let object = self.eval(object, frame)?;
                self.set_member(&object, name, value);
            }
            Expr::Index(object, index) => {
                let object = self.eval(object, frame)?;
                if let Value::Str(name) = self.eval(index, frame)? {
                    self.set_member(&object, &name, value);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn binary(&self, op: BinOp, l: &Value, r: &Value) -> Value {
        match op {
            BinOp::Add => {
                if matches!(l, Value::Str(_)) || matches!(r, Value::Str(_)) {
                    let mut out = self.to_js_string(l);
                    let right = self.to_js_string(r);
                    if out.len() + right.len() > MAX_STRING_CHARS {
                        return Value::Undefined;
                    }
                    out.push_str(&right);
                    Value::Str(Rc::from(out))
                } else {
                    Value::Num(self.to_number(l) + self.to_number(r))
                }
            }
            BinOp::Sub => Value::Num(self.to_number(l) - self.to_number(r)),
            BinOp::Mul => Value::Num(self.to_number(l) * self.to_number(r)),
            BinOp::Div => Value::Num(self.to_number(l) / self.to_number(r)),
            BinOp::Rem => Value::Num(self.to_number(l) % self.to_number(r)),
            BinOp::Eq => Value::Bool(self.loose_eq(l, r)),
            BinOp::Ne => Value::Bool(!self.loose_eq(l, r)),
            BinOp::StrictEq => Value::Bool(strict_eq(l, r)),
            BinOp::StrictNe => Value::Bool(!strict_eq(l, r)),
            BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                let ordering = match (l, r) {
                    (Value::Str(a), Value::Str(b)) => Some(a.cmp(b)),
                    _ => self.to_number(l).partial_cmp(&self.to_number(r)),
                };
                Value::Bool(match ordering {
                    None => false,
                    Some(o) => match op {
                        BinOp::Lt => o.is_lt(),
                        BinOp::Gt => o.is_gt(),
                        BinOp::Le => o.is_le(),
                        _ => o.is_ge(),
                    },
                })
            }
        }
    }

    // ---------------------------------------------------------- conversions

    fn to_number(&self, v: &Value) -> f64 {
        match v {
            Value::Undefined => f64::NAN,
            Value::Null => 0.0,
            Value::Bool(b) => f64::from(u8::from(*b)),
            Value::Num(n) => *n,
            Value::Str(s) => str_to_number(s),
            Value::Node(r) => match self.raw_value(r) {
                Value::Str(s) => str_to_number(&s),
                _ => f64::NAN,
            },
            Value::Data(r) => match self.data_value(r) {
                Value::Str(s) => str_to_number(&s),
                _ => f64::NAN,
            },
            _ => f64::NAN,
        }
    }

    fn to_js_string(&self, v: &Value) -> String {
        match v {
            Value::Undefined => "undefined".into(),
            Value::Null => "null".into(),
            Value::Bool(b) => b.to_string(),
            Value::Num(n) => num_to_string(*n),
            Value::Str(s) => s.to_string(),
            Value::Node(r) => match self.raw_value(r) {
                Value::Str(s) => s.to_string(),
                _ => String::new(),
            },
            Value::TextVar(owner, name) => self.text_var(owner, name),
            Value::Data(r) => match self.data_value(r) {
                Value::Str(s) => s.to_string(),
                _ => String::new(),
            },
            Value::RegExp(r) => format!("/{}/{}", r.re.source, r.re.flags()),
            _ => "[object]".into(),
        }
    }

    /// JavaScript's `==`.
    fn loose_eq(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Undefined | Value::Null, Value::Undefined | Value::Null) => true,
            (Value::Undefined | Value::Null, _) | (_, Value::Undefined | Value::Null) => false,
            (Value::Str(x), Value::Str(y)) => x == y,
            (Value::Node(x), Value::Node(y)) => x == y,
            (Value::Node(_), Value::Str(_)) | (Value::TextVar(..), Value::Str(_)) => {
                self.to_js_string(a) == self.to_js_string(b)
            }
            (Value::Str(_), Value::Node(_)) | (Value::Str(_), Value::TextVar(..)) => {
                self.to_js_string(a) == self.to_js_string(b)
            }
            (Value::Num(_) | Value::Bool(_) | Value::Str(_) | Value::Node(_), _)
            | (_, Value::Num(_) | Value::Bool(_) | Value::Str(_) | Value::Node(_)) => {
                let (x, y) = (self.to_number(a), self.to_number(b));
                x == y
            }
            _ => false,
        }
    }

    // ---------------------------------------------------------------- scope

    /// XFA's scope resolution of a bare name from `ctx`: the node's own
    /// children, then each ancestor's children and variables, then the
    /// root's own name.
    pub(super) fn lookup_scope(&self, ctx: &NodeRef, name: &str) -> Value {
        let mut at = ctx.clone();
        loop {
            if let Some(found) = self.find_named(&at, name, 0) {
                return Value::Node(found);
            }
            // `_Row`: the instance manager of `Row`.
            if let Some(found) = (name.strip_prefix('_'))
                .filter(|rest| !rest.is_empty())
                .and_then(|rest| self.find_named(&at, rest, 0))
            {
                return Value::InstanceManager(found);
            }
            if let Some(found) = self.variable(&at, name) {
                return found;
            }
            match at.parent() {
                Some(parent) => at = parent,
                None => {
                    if self.form.root.common.name == name {
                        return Value::Node(at);
                    }
                    return Value::Undefined;
                }
            }
        }
    }

    /// The `index`-th child of `at` named `name`, unnamed subforms
    /// searched through (they are transparent to names).
    pub(super) fn find_named(&self, at: &NodeRef, name: &str, index: usize) -> Option<NodeRef> {
        let mut count = 0;
        self.find_named_in(at, name, index, &mut count, 0)
    }

    fn find_named_in(
        &self,
        at: &NodeRef,
        name: &str,
        index: usize,
        count: &mut usize,
        depth: usize,
    ) -> Option<NodeRef> {
        if depth > MAX_LOOKUP_DEPTH {
            return None;
        }
        let parent = view(self.form, at)?;
        for i in 0..parent.child_count() {
            let child_ref = at.child(i);
            let child = view(self.form, &child_ref)?;
            if child.name() == name {
                if *count == index {
                    return Some(child_ref);
                }
                *count += 1;
            } else if let NodeView::Subform(s) = child {
                if s.common.name.is_empty() {
                    if let Some(found) =
                        self.find_named_in(&child_ref, name, index, count, depth + 1)
                    {
                        return Some(found);
                    }
                }
            }
        }
        None
    }

    /// A subform's variable by name.
    fn variable(&self, at: &NodeRef, name: &str) -> Option<Value> {
        let NodeView::Subform(s) = view(self.form, at)? else {
            return None;
        };
        let v = s.variables.iter().find(|v| v.name == name)?;
        Some(if v.is_script {
            Value::ScriptObject(at.clone(), Rc::from(name))
        } else {
            Value::TextVar(at.clone(), Rc::from(name))
        })
    }

    pub(super) fn text_var(&self, owner: &NodeRef, name: &str) -> String {
        if let Some(text) = self.text_vars.get(&(owner.clone(), name.to_string())) {
            return text.clone();
        }
        match view(self.form, owner) {
            Some(NodeView::Subform(s)) => s
                .variables
                .iter()
                .find(|v| v.name == name)
                .map(|v| v.text.clone())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// A script object's function by name (the object parsed on first use).
    fn script_function(&mut self, owner: &NodeRef, object: &str, name: &str) -> Value {
        let key = (owner.clone(), object.to_string());
        if !self.objects.contains_key(&key) {
            let source = match view(self.form, owner) {
                Some(NodeView::Subform(s)) => s
                    .variables
                    .iter()
                    .find(|v| v.name == object && v.is_script && !v.formcalc)
                    .map(|v| v.text.clone()),
                _ => None,
            };
            let mut functions = HashMap::new();
            let mut vars = HashMap::new();
            if let Some(source) = source {
                for stmt in parse::parse(&source) {
                    match stmt {
                        Stmt::Function(f) => {
                            functions.insert(f.name.clone(), f);
                        }
                        Stmt::Var(decls) => {
                            for (var, init) in decls {
                                if let Some(v) = init.as_ref().and_then(|e| const_value(e, 0)) {
                                    vars.insert(var, v);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            self.objects
                .insert(key.clone(), Rc::new(ScriptObject { functions, vars }));
        }
        let Some(object) = self.objects.get(&key) else {
            return Value::Undefined;
        };
        match (object.functions.get(name), object.vars.get(name)) {
            (Some(f), _) => Value::Function(f.clone(), owner.clone()),
            (None, Some(v)) => v.clone(),
            (None, None) => Value::Undefined,
        }
    }

    // -------------------------------------------------------------- members

    /// `object.name` as JavaScript reads it, with `this` = `this` (for
    /// `xfa.resolveNode`): the FormCalc evaluator's way in.
    pub(super) fn member_of(&mut self, object: &Value, name: &str, this: &NodeRef) -> Value {
        let frame = Frame::new(this.clone(), 0);
        self.member(object, name, &frame)
    }

    fn member(&mut self, object: &Value, name: &str, frame: &Frame) -> Value {
        match object {
            Value::Node(at) => self.node_member(at, name),
            Value::Xfa => match name {
                "host" | "layout" | "template" | "config" | "connectionSet" | "sourceSet" => {
                    Value::Host
                }
                "event" => Value::Event,
                "form" => Value::FormModel,
                "resolveNode" => Value::Builtin(Builtin::ResolveNode(frame.this.clone())),
                "resolveNodes" => Value::Builtin(Builtin::ResolveNodes(frame.this.clone())),
                "datasets" => {
                    if self.form.datasets.is_some() {
                        Value::Data(DataRef::default())
                    } else {
                        Value::Undefined
                    }
                }
                "record" => self.data_record().map_or(Value::Undefined, Value::Data),
                _ => Value::Undefined,
            },
            Value::Host if name == "messageBox" => Value::Builtin(Builtin::MessageBox),
            Value::Host => Value::Builtin(Builtin::Noop),
            Value::Event => match name {
                "target" => Value::Node(self.event.target.clone()),
                "newText" | "fullText" => match &self.event.new_text {
                    Some(text) => Value::Str(Rc::from(text.as_str())),
                    None => Value::Undefined,
                },
                "prevText" | "change" | "prevContentType" | "newContentType" | "soapFaultCode"
                | "soapFaultString" => Value::Str(Rc::from("")),
                "name" => Value::Str(Rc::from(self.event.activity.as_str())),
                "selStart" | "selEnd" | "commitKey" | "keyDown" => Value::Num(0.0),
                "shift" | "modifier" | "cancelAction" | "reenter" => Value::Bool(false),
                _ => Value::Undefined,
            },
            Value::FormModel => {
                if self.form.root.common.name == name {
                    Value::Node(NodeRef::default())
                } else if name == "resolveNode" {
                    Value::Builtin(Builtin::ResolveNode(NodeRef::default()))
                } else if name == "resolveNodes" {
                    Value::Builtin(Builtin::ResolveNodes(NodeRef::default()))
                } else {
                    Value::Undefined
                }
            }
            Value::ScriptObject(owner, object) => {
                let owner = owner.clone();
                let object = object.clone();
                self.script_function(&owner, &object, name)
            }
            Value::TextVar(owner, var) => match name {
                "value" | "rawValue" => Value::Str(Rc::from(self.text_var(owner, var))),
                _ => Value::Undefined,
            },
            Value::Str(s) => match name {
                "length" => Value::Num(s.chars().count() as f64),
                "toString" | "valueOf" | "substring" | "substr" | "slice" | "indexOf"
                | "lastIndexOf" | "toUpperCase" | "toLowerCase" | "charAt" | "concat" | "trim"
                | "replace" | "split" | "match" | "search" => {
                    Value::Builtin(Builtin::StrMethod(s.clone(), str_method(name)))
                }
                "charCodeAt" | "localeCompare" => Value::Builtin(Builtin::Noop),
                _ => Value::Undefined,
            },
            Value::Num(n) => match name {
                "toString" | "toFixed" | "valueOf" => {
                    Value::Builtin(Builtin::NumMethod(*n, num_method(name)))
                }
                _ => Value::Undefined,
            },
            Value::InstanceManager(at) => match name {
                "count" => Value::Num(self.instance_count(at) as f64),
                "min" => Value::Num(self.occur(at).0 as f64),
                "max" => Value::Num(self.occur(at).1.map_or(f64::INFINITY, |m| m as f64)),
                "addInstance" => Value::Builtin(Builtin::Instances(at.clone(), "add")),
                "removeInstance" => Value::Builtin(Builtin::Instances(at.clone(), "remove")),
                "setInstances" => Value::Builtin(Builtin::Instances(at.clone(), "set")),
                "moveInstance" | "insertInstance" | "name" => Value::Builtin(Builtin::Noop),
                _ => Value::Undefined,
            },
            Value::List(items) => match name {
                "length" => Value::Num(items.len() as f64),
                "item" => Value::Builtin(Builtin::Item(items.clone())),
                "join" => Value::Builtin(Builtin::Join(items.clone())),
                _ => Value::Undefined,
            },
            Value::RegExp(r) => match name {
                "test" => Value::Builtin(Builtin::RegexMethod(r.clone(), "test")),
                "exec" => Value::Builtin(Builtin::RegexMethod(r.clone(), "exec")),
                "source" => Value::Str(Rc::from(r.re.source.as_str())),
                "flags" => Value::Str(Rc::from(r.re.flags())),
                "global" => Value::Bool(r.re.global),
                "ignoreCase" => Value::Bool(r.re.ignore_case),
                "multiline" => Value::Bool(r.re.multiline),
                "lastIndex" => Value::Num(r.last_index.get() as f64),
                _ => Value::Undefined,
            },
            Value::Data(r) => self.data_member(r, name),
            _ => Value::Undefined,
        }
    }

    fn node_member(&self, at: &NodeRef, name: &str) -> Value {
        let Some(node) = view(self.form, at) else {
            return Value::Undefined;
        };
        match name {
            "rawValue" | "value" => self.raw_value(at),
            "formattedValue" => match node {
                NodeView::Field(f) => {
                    Value::Str(Rc::from(crate::xfa::picture::shown(self.form, f)))
                }
                _ => self.raw_value(at),
            },
            "presence" => Value::Str(Rc::from(presence_name(node.presence()))),
            "isNull" => Value::Bool(matches!(self.raw_value(at), Value::Null)),
            "name" => Value::Str(Rc::from(node.name())),
            "parent" => match at.parent() {
                Some(parent) => Value::Node(parent),
                None => Value::Undefined,
            },
            "index" => Value::Num(match node {
                NodeView::Subform(s) => s.instance as f64,
                _ => 0.0,
            }),
            "access" => Value::Str(Rc::from(if node.read_only() { "readOnly" } else { "open" })),
            "instanceManager" => Value::InstanceManager(at.clone()),
            "className" => Value::Str(Rc::from(node.class())),
            "somExpression" => Value::Str(Rc::from(match node {
                NodeView::Subform(s) => s.som.as_str(),
                NodeView::Field(f) => f.som.as_str(),
                NodeView::Group(g) => g.som.as_str(),
                NodeView::Draw(_) => "",
            })),
            "resolveNode" => Value::Builtin(Builtin::ResolveNode(at.clone())),
            "resolveNodes" => Value::Builtin(Builtin::ResolveNodes(at.clone())),
            "getAttribute"
            | "setAttribute"
            | "execEvent"
            | "execInitialize"
            | "execCalculate"
            | "execValidate"
            | "isPropertySpecified"
            | "addItem"
            | "clearItems"
            | "deleteItem"
            | "getDisplayItem"
            | "getSaveItem"
            | "boundItem"
            | "setFocus"
            | "selectedIndex"
            | "getItemState"
            | "setItemState" => Value::Builtin(Builtin::Noop),
            _ => {
                if let Some(child) = name.strip_prefix('_') {
                    if let Some(found) = self.find_named(at, child, 0) {
                        return Value::InstanceManager(found);
                    }
                }
                if let Some(found) = self.find_named(at, name, 0) {
                    return Value::Node(found);
                }
                self.variable(at, name).unwrap_or(Value::Undefined)
            }
        }
    }

    pub(super) fn set_member(&mut self, object: &Value, name: &str, value: Value) {
        match object {
            Value::Node(at) => match name {
                "rawValue" | "value" => self.set_raw_value(at, &value),
                "formattedValue" => {
                    // Read back through the field's pictures to its
                    // canonical value; kept as given when nothing reads it.
                    let text = match value {
                        Value::Undefined | Value::Null => None,
                        ref other => Some(self.to_js_string(other)),
                    };
                    let canonical = match (&text, view(self.form, at)) {
                        (Some(t), Some(NodeView::Field(f))) => {
                            crate::xfa::picture::canonical(self.form, f, t)
                        }
                        _ => None,
                    };
                    match canonical {
                        Some(c) => self.set_raw_value(at, &Value::Str(Rc::from(c))),
                        None => self.set_raw_value(at, &value),
                    }
                }
                "presence" => {
                    let text = self.to_js_string(&value);
                    let presence = Presence::parse(&text);
                    if let Some(node) = view_mut(self.form, at) {
                        match node {
                            NodeMut::Subform(s) => s.common.presence = presence,
                            NodeMut::Field(f) => f.common.presence = presence,
                            NodeMut::Draw(d) => d.common.presence = presence,
                            NodeMut::Group(g) => g.common.presence = presence,
                        }
                    }
                }
                "access" => {
                    let text = self.to_js_string(&value);
                    let read_only =
                        matches!(text.as_str(), "readOnly" | "protected" | "nonInteractive");
                    if let Some(node) = view_mut(self.form, at) {
                        match node {
                            NodeMut::Subform(s) => s.common.read_only = read_only,
                            NodeMut::Field(f) => f.common.read_only = read_only,
                            NodeMut::Draw(d) => d.common.read_only = read_only,
                            NodeMut::Group(g) => g.common.read_only = read_only,
                        }
                    }
                }
                _ => {}
            },
            Value::TextVar(owner, var) => {
                if matches!(name, "value" | "rawValue") {
                    let text = self.to_js_string(&value);
                    if text.len() <= MAX_STRING_CHARS {
                        self.text_vars
                            .insert((owner.clone(), var.to_string()), text);
                    }
                }
            }
            Value::RegExp(r) if name == "lastIndex" => {
                let n = self.to_number(&value);
                r.last_index.set(if n.is_finite() && n > 0.0 {
                    n as usize
                } else {
                    0
                });
            }
            _ => {}
        }
    }

    /// A node's `rawValue`: its value's text, `null` when empty.
    pub fn raw_value(&self, at: &NodeRef) -> Value {
        let text = match view(self.form, at) {
            Some(NodeView::Field(f)) => f.value.plain(),
            Some(NodeView::Group(g)) => g.value.clone(),
            Some(NodeView::Draw(d)) => d.value.plain(),
            _ => return Value::Undefined,
        };
        if text.is_empty() {
            Value::Null
        } else {
            Value::Str(Rc::from(text))
        }
    }

    pub fn set_raw_value(&mut self, at: &NodeRef, value: &Value) {
        let text = match value {
            Value::Undefined | Value::Null => None,
            other => Some(self.to_js_string(other)),
        };
        if text.as_ref().is_some_and(|t| t.len() > MAX_STRING_CHARS) {
            return;
        }
        let as_value = |text: &Option<String>| match text {
            Some(t) => XValue::Text(t.clone()),
            None => XValue::Empty,
        };
        let group_update = match view_mut(self.form, at) {
            Some(NodeMut::Field(f)) => {
                f.value = as_value(&text);
                if f.is_check() {
                    let (on, _) = f.check_values();
                    f.checked = text.as_deref() == Some(on.as_str());
                }
                if f.group.is_some() {
                    let (on, _) = f.check_values();
                    Some((f.checked, on))
                } else {
                    None
                }
            }
            Some(NodeMut::Draw(d)) => {
                d.value = as_value(&text);
                None
            }
            Some(NodeMut::Group(_)) => {
                self.set_group_value(at, text.unwrap_or_default());
                None
            }
            _ => None,
        };
        // A radio button set on or off changes its group's value.
        if let (Some((checked, on)), Some(group)) = (group_update, at.parent()) {
            if let Some(NodeView::Group(g)) = view(self.form, &group) {
                let current = g.value.clone();
                let next = if checked {
                    on
                } else if current == on {
                    String::new()
                } else {
                    current
                };
                self.set_group_value(&group, next);
            }
        }
    }

    fn set_group_value(&mut self, at: &NodeRef, value: String) {
        if let Some(NodeMut::Group(g)) = view_mut(self.form, at) {
            g.value = value;
            for f in &mut g.fields {
                let (on, off) = f.check_values();
                f.checked = !g.value.is_empty() && on == g.value;
                f.value = XValue::Text(if f.checked { on } else { off });
            }
        }
    }

    /// The repeating subform the subform at `at` is an instance of: its
    /// parent and name. `None` for an unnamed subform, the root, or
    /// anything else.
    fn instance_key(&self, at: &NodeRef) -> Option<(NodeRef, String)> {
        let Some(NodeView::Subform(s)) = view(self.form, at) else {
            return None;
        };
        let parent = at.parent()?;
        if s.common.name.is_empty()
            || !matches!(view(self.form, &parent), Some(NodeView::Subform(_)))
        {
            return None;
        }
        Some((parent, s.common.name.clone()))
    }

    /// The instances of a repeating subform (its parent's subform children
    /// of its name), read once per runner: the scripts change no
    /// structure.
    fn instances(&mut self, key: &(NodeRef, String)) -> Rc<Instances> {
        if let Some(found) = self.instance_groups.get(key) {
            return found.clone();
        }
        let (parent, name) = key;
        let mut count = 0;
        let mut paths = Some(Vec::new());
        if let Some(NodeView::Subform(p)) = view(self.form, parent) {
            for child in &p.children {
                let Node::Subform(s) = child else {
                    continue;
                };
                if s.common.name != *name {
                    continue;
                }
                count += 1;
                paths = paths.and_then(|mut list: Vec<DataPath>| {
                    let path = s.data.clone()?;
                    let (last, head) = path.0.split_last()?;
                    let same_parent = list
                        .first()
                        .is_none_or(|first| first.0[..first.0.len() - 1] == *head);
                    (last.0 == *name && same_parent).then(|| {
                        list.push(path);
                        list
                    })
                });
            }
        }
        let found = Rc::new(Instances {
            count,
            paths: paths.filter(|p| !p.is_empty()).map(Rc::from),
        });
        self.instance_groups.insert(key.clone(), found.clone());
        found
    }

    /// Instances of the subform at `at`, the recorded operations counted.
    fn instance_count(&mut self, at: &NodeRef) -> usize {
        let Some(key) = self.instance_key(at) else {
            return 1;
        };
        let count = self.instances(&key).count;
        let delta = self.instance_delta.get(&key).copied().unwrap_or(0);
        (count as isize + delta).max(0) as usize
    }

    /// `instanceManager.addInstance`, `removeInstance(i)`, `setInstances(n)`
    /// of the subform at `at`, recorded when `occur` allows it.
    fn instance_method(&mut self, at: &NodeRef, method: &str, args: &[Value]) {
        if self.instance_ops.len() >= MAX_INSTANCE_OPS {
            return;
        }
        let Some(key) = self.instance_key(at) else {
            return;
        };
        let Some(instances) = self.instances(&key).paths.clone() else {
            return;
        };
        let (min, max) = self.occur(at);
        let group = InstanceGroup {
            key,
            instances,
            min,
            cap: max.unwrap_or(MAX_INSTANCES).min(MAX_INSTANCES),
        };
        let wanted = args
            .first()
            .map(|a| self.to_number(a))
            .filter(|n| n.is_finite() && *n >= 0.0 && n.fract() == 0.0)
            .map(|n| n.min(usize::MAX as f64) as usize);
        match method {
            "add" => {
                self.add_instance(&group);
            }
            "remove" => {
                if let Some(index) = wanted {
                    self.remove_instance(&group, index);
                }
            }
            _ => {
                let Some(target) = wanted else {
                    return;
                };
                let target = target.clamp(group.min, group.cap.max(group.min));
                while self.group_count(&group) < target {
                    if !self.add_instance(&group) {
                        break;
                    }
                }
                while self.group_count(&group) > target {
                    let last = self.group_count(&group) - 1;
                    if !self.remove_instance(&group, last) {
                        break;
                    }
                }
            }
        }
    }

    fn group_count(&self, group: &InstanceGroup) -> usize {
        let delta = self.instance_delta.get(&group.key).copied().unwrap_or(0);
        (group.instances.len() as isize + delta).max(0) as usize
    }

    fn record_instance_op(&mut self, group: &InstanceGroup, op: InstanceOp, step: isize) -> bool {
        if self.instance_ops.len() >= MAX_INSTANCE_OPS {
            return false;
        }
        self.instance_ops.push(op);
        *self.instance_delta.entry(group.key.clone()).or_insert(0) += step;
        true
    }

    fn add_instance(&mut self, group: &InstanceGroup) -> bool {
        if self.group_count(group) >= group.cap {
            return false;
        }
        let op = InstanceOp::Add {
            instances: group.instances.clone(),
        };
        self.record_instance_op(group, op, 1)
    }

    fn remove_instance(&mut self, group: &InstanceGroup, index: usize) -> bool {
        let count = self.group_count(group);
        if count <= group.min || index >= count {
            return false;
        }
        let op = InstanceOp::Remove {
            instances: group.instances.clone(),
            index,
        };
        self.record_instance_op(group, op, -1)
    }

    fn occur(&self, at: &NodeRef) -> (usize, Option<usize>) {
        match view(self.form, at) {
            Some(NodeView::Subform(s)) => (s.occur.min, s.occur.max),
            _ => (1, Some(1)),
        }
    }

    // ---------------------------------------------------------------- calls

    /// Calls `function` (a script object's function, `resolveNode`, a
    /// method) from `depth` calls deep with `this` = `this`: the FormCalc
    /// evaluator's way in.
    pub(super) fn call_value(
        &mut self,
        function: Value,
        args: Vec<Value>,
        this: &NodeRef,
        depth: usize,
    ) -> Result<Value, Stopped> {
        let frame = Frame::new(this.clone(), depth);
        self.call(function, args, &frame)
    }

    fn call(&mut self, function: Value, args: Vec<Value>, frame: &Frame) -> Result<Value, Stopped> {
        match function {
            Value::Function(f, scope) => {
                if frame.depth >= MAX_CALL_DEPTH {
                    return Ok(Value::Undefined);
                }
                let mut inner = Frame::new(scope, frame.depth + 1);
                let mut args = args.into_iter();
                for param in &f.params {
                    inner.set(param, args.next().unwrap_or(Value::Undefined));
                }
                self.hoist(&f.body, &mut inner);
                Ok(match self.exec_block(&f.body, &mut inner)? {
                    Flow::Return(v) => v,
                    _ => Value::Undefined,
                })
            }
            Value::Builtin(Builtin::ResolveNode(base)) => Ok(match args.first() {
                Some(Value::Str(expression)) => {
                    let found = self.resolve_all(&base, expression);
                    self.charge(0)?;
                    found.into_iter().next().unwrap_or(Value::Null)
                }
                _ => Value::Null,
            }),
            Value::Builtin(Builtin::ResolveNodes(base)) => Ok(match args.first() {
                Some(Value::Str(expression)) => {
                    let found = self.resolve_all(&base, expression);
                    self.charge(found.len())?;
                    Value::List(found.into())
                }
                _ => Value::List(Rc::from([])),
            }),
            Value::Builtin(Builtin::Item(items)) => Ok(match args.first() {
                Some(v) => match list_item(&items, self.to_number(v)) {
                    Value::Undefined => Value::Null,
                    found => found,
                },
                None => Value::Null,
            }),
            Value::Builtin(Builtin::StrMethod(s, method)) => {
                let regex_arg = matches!(args.first(), Some(Value::RegExp(_)));
                match method {
                    "match" | "search" | "split" => self.string_regex(&s, method, &args, frame),
                    "replace" if regex_arg || matches!(args.get(1), Some(Value::Function(..))) => {
                        self.string_regex(&s, method, &args, frame)
                    }
                    _ => Ok(self.string_method(&s, method, &args)),
                }
            }
            Value::Builtin(Builtin::RegExpCtor) => Ok(match args.first() {
                Some(Value::RegExp(r))
                    if args.get(1).is_none_or(|f| matches!(f, Value::Undefined)) =>
                {
                    regexp(&r.re.source, &r.re.flags())
                }
                Some(Value::RegExp(r)) => {
                    let flags = self.to_js_string(&args[1]);
                    regexp(&r.re.source, &flags)
                }
                first => {
                    let pattern = match first {
                        None | Some(Value::Undefined) => String::new(),
                        Some(v) => self.to_js_string(v),
                    };
                    let flags = match args.get(1) {
                        None | Some(Value::Undefined) => String::new(),
                        Some(v) => self.to_js_string(v),
                    };
                    self.charge(pattern.len() / 16)?;
                    regexp(&pattern, &flags)
                }
            }),
            Value::Builtin(Builtin::RegexMethod(r, method)) => {
                let text = match args.first() {
                    Some(v) => self.to_js_string(v),
                    None => "undefined".to_string(),
                };
                let chars: Vec<char> = text.chars().collect();
                let from = if r.re.global { r.last_index.get() } else { 0 };
                if from > chars.len() {
                    r.last_index.set(0);
                    return Ok(if method == "test" {
                        Value::Bool(false)
                    } else {
                        Value::Null
                    });
                }
                let found = self.run_regex(|steps| r.re.find(&chars, from, steps))?;
                if r.re.global {
                    r.last_index
                        .set(found.as_ref().map_or(0, |c| c[0].map_or(0, |m| m.1)));
                }
                Ok(match (method, found) {
                    ("test", found) => Value::Bool(found.is_some()),
                    (_, Some(caps)) => groups_list(&chars, &caps),
                    (_, None) => Value::Null,
                })
            }
            Value::Builtin(Builtin::Join(items)) => {
                let sep = match args.first() {
                    None | Some(Value::Undefined) => ",".to_string(),
                    Some(v) => self.to_js_string(v),
                };
                self.charge(items.len())?;
                let mut out = String::new();
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(&sep);
                    }
                    if !matches!(item, Value::Undefined | Value::Null) {
                        out.push_str(&self.to_js_string(item));
                    }
                    if out.len() > MAX_STRING_CHARS {
                        return Ok(Value::Undefined);
                    }
                }
                Ok(Value::Str(Rc::from(out)))
            }
            Value::Builtin(Builtin::MessageBox) => {
                let text = match args.first() {
                    Some(Value::Undefined) | None => String::new(),
                    Some(v) => self.to_js_string(v),
                };
                self.record_message(&text);
                Ok(Value::Num(1.0))
            }
            Value::Builtin(Builtin::Instances(at, method)) => {
                self.instance_method(&at, method, &args);
                Ok(Value::Undefined)
            }
            Value::Builtin(Builtin::NumMethod(n, method)) => Ok(match method {
                "toFixed" => {
                    let digits = args
                        .first()
                        .map_or(0.0, |d| self.to_number(d))
                        .clamp(0.0, 20.0) as usize;
                    Value::Str(Rc::from(format!("{n:.digits$}")))
                }
                _ => Value::Str(Rc::from(num_to_string(n))),
            }),
            _ => Ok(Value::Undefined),
        }
    }

    fn string_method(&self, s: &str, method: &str, args: &[Value]) -> Value {
        let chars: Vec<char> = s.chars().collect();
        let int = |index: usize| -> Option<usize> {
            let n = self.to_number(args.get(index)?);
            if n.is_nan() {
                Some(0)
            } else {
                Some(n.max(0.0).min(chars.len() as f64) as usize)
            }
        };
        let string = |text: String| Value::Str(Rc::from(text));
        match method {
            "toString" | "valueOf" | "trim" => string(if method == "trim" {
                s.trim().to_string()
            } else {
                s.to_string()
            }),
            "substring" | "slice" => {
                let a = int(0).unwrap_or(0);
                let b = int(1).unwrap_or(chars.len());
                let (a, b) = if a <= b { (a, b) } else { (b, a) };
                string(chars[a..b].iter().collect())
            }
            "substr" => {
                let a = int(0).unwrap_or(0);
                let len = int(1).unwrap_or(chars.len());
                string(chars[a..(a + len).min(chars.len())].iter().collect())
            }
            "indexOf" | "lastIndexOf" => {
                let needle = args
                    .first()
                    .map(|v| self.to_js_string(v))
                    .unwrap_or_default();
                let found = if method == "indexOf" {
                    s.find(&needle)
                } else {
                    s.rfind(&needle)
                };
                Value::Num(match found {
                    Some(byte) => s[..byte].chars().count() as f64,
                    None => -1.0,
                })
            }
            "toUpperCase" => string(s.to_uppercase()),
            "toLowerCase" => string(s.to_lowercase()),
            "charAt" => char_at(s, int(0).unwrap_or(0) as f64),
            "concat" => {
                let mut out = s.to_string();
                for arg in args {
                    out.push_str(&self.to_js_string(arg));
                    if out.len() > MAX_STRING_CHARS {
                        return Value::Undefined;
                    }
                }
                string(out)
            }
            "replace" => match args {
                [Value::Str(from), to, ..] => {
                    string(s.replacen(&**from, &self.to_js_string(to), 1))
                }
                _ => Value::Undefined,
            },
            _ => Value::Undefined,
        }
    }
}

impl Runner<'_> {
    /// Runs a match with what is left of the budgets (`regex`), charging
    /// the steps it took; one that runs out stops the script.
    fn run_regex<T>(
        &mut self,
        f: impl FnOnce(&mut usize) -> Result<T, super::regex::Exhausted>,
    ) -> Result<T, Stopped> {
        let allowed = self.steps.min(self.script_steps);
        let mut left = allowed;
        let out = f(&mut left);
        self.charge(allowed - left)?;
        out.map_err(|_| {
            self.script_steps = 0;
            Stopped
        })
    }

    /// The pattern a string method's first argument stands for: a
    /// regular expression as it is, anything else read as one.
    fn pattern_of(&mut self, arg: Option<&Value>) -> Result<Option<Rc<RegExpValue>>, Stopped> {
        Ok(match arg {
            Some(Value::RegExp(r)) => Some(r.clone()),
            None | Some(Value::Undefined) => match regexp("(?:)", "") {
                Value::RegExp(r) => Some(r),
                _ => None,
            },
            Some(v) => {
                let text = self.to_js_string(v);
                self.charge(text.len() / 16)?;
                match regexp(&text, "") {
                    Value::RegExp(r) => Some(r),
                    _ => None,
                }
            }
        })
    }

    /// Every match from the start (a global `match`, `replace` and
    /// `split` walk), empty matches stepping on one char.
    fn all_matches(
        &mut self,
        re: &Regex,
        chars: &[char],
        global: bool,
    ) -> Result<Vec<Captures>, Stopped> {
        let mut out = Vec::new();
        let mut from = 0;
        while from <= chars.len() && out.len() < MAX_SPLIT {
            let Some(caps) = self.run_regex(|steps| re.find(chars, from, steps))? else {
                break;
            };
            let (a, b) = caps[0].unwrap_or((from, from));
            out.push(caps);
            if !global {
                break;
            }
            from = if b == a { b + 1 } else { b };
        }
        self.charge(out.len())?;
        Ok(out)
    }

    /// `match`, `search`, `split`, and `replace` with a regular
    /// expression or a function.
    fn string_regex(
        &mut self,
        s: &str,
        method: &str,
        args: &[Value],
        frame: &Frame,
    ) -> Result<Value, Stopped> {
        let chars: Vec<char> = s.chars().collect();
        let text =
            |a: usize, b: usize| Value::Str(Rc::from(chars[a..b].iter().collect::<String>()));
        match method {
            "match" => {
                let Some(r) = self.pattern_of(args.first())? else {
                    return Ok(Value::Undefined);
                };
                if !r.re.global {
                    let found = self.run_regex(|steps| r.re.find(&chars, 0, steps))?;
                    return Ok(found.map_or(Value::Null, |caps| groups_list(&chars, &caps)));
                }
                r.last_index.set(0);
                let all = self.all_matches(&r.re, &chars, true)?;
                if all.is_empty() {
                    return Ok(Value::Null);
                }
                let items: Vec<Value> = all
                    .iter()
                    .filter_map(|c| c[0].map(|(a, b)| text(a, b)))
                    .collect();
                Ok(Value::List(items.into()))
            }
            "search" => {
                let Some(r) = self.pattern_of(args.first())? else {
                    return Ok(Value::Undefined);
                };
                let found = self.run_regex(|steps| r.re.find(&chars, 0, steps))?;
                Ok(Value::Num(
                    found.and_then(|c| c[0]).map_or(-1.0, |(a, _)| a as f64),
                ))
            }
            "split" => {
                let limit = match args.get(1) {
                    None | Some(Value::Undefined) => MAX_SPLIT,
                    Some(v) => (self.to_number(v).max(0.0) as usize).min(MAX_SPLIT),
                };
                let mut out: Vec<Value> = Vec::new();
                match args.first() {
                    None | Some(Value::Undefined) => out.push(Value::Str(Rc::from(s))),
                    Some(Value::RegExp(r)) => {
                        if chars.is_empty() {
                            let hit = self.run_regex(|steps| r.re.match_at(&chars, 0, steps))?;
                            if hit.is_none() {
                                out.push(text(0, 0));
                            }
                        } else {
                            // The ES split walk: a match at q that ends past
                            // the last cut makes a piece, then its groups.
                            let (mut p, mut q) = (0, 0);
                            while q < chars.len() && out.len() < limit {
                                let hit =
                                    self.run_regex(|steps| r.re.match_at(&chars, q, steps))?;
                                match hit.and_then(|c| Some((c[0]?.1, c))) {
                                    Some((e, caps)) if e != p && e <= chars.len() => {
                                        out.push(text(p, q));
                                        for g in caps.iter().skip(1) {
                                            if out.len() >= limit {
                                                break;
                                            }
                                            out.push(
                                                g.map_or(Value::Undefined, |(a, b)| text(a, b)),
                                            );
                                        }
                                        p = e;
                                        q = p;
                                    }
                                    _ => q += 1,
                                }
                            }
                            out.push(text(p, chars.len()));
                        }
                    }
                    Some(sep) => {
                        let sep = self.to_js_string(sep);
                        if sep.is_empty() {
                            out.extend((0..chars.len()).map(|i| text(i, i + 1)));
                        } else {
                            out.extend(s.split(sep.as_str()).map(|p| Value::Str(Rc::from(p))));
                        }
                    }
                }
                out.truncate(limit);
                self.charge(out.len())?;
                Ok(Value::List(out.into()))
            }
            _ => {
                // replace
                let all = match args.first() {
                    Some(Value::RegExp(r)) => {
                        let all = self.all_matches(&r.re, &chars, r.re.global)?;
                        if r.re.global {
                            r.last_index.set(0);
                        }
                        all
                    }
                    Some(v) => {
                        let needle: Vec<char> = self.to_js_string(v).chars().collect();
                        let at = (0..=chars.len().saturating_sub(needle.len()))
                            .find(|i| chars[*i..].starts_with(&needle));
                        match at {
                            Some(i) if needle.len() <= chars.len() => {
                                vec![vec![Some((i, i + needle.len()))]]
                            }
                            _ => Vec::new(),
                        }
                    }
                    None => Vec::new(),
                };
                let mut out = String::new();
                let mut last = 0;
                for caps in all {
                    let Some((a, b)) = caps[0] else { continue };
                    out.extend(&chars[last..a]);
                    let piece = match args.get(1) {
                        Some(f @ Value::Function(..)) => {
                            let mut call_args: Vec<Value> = caps
                                .iter()
                                .map(|c| c.map_or(Value::Undefined, |(x, y)| text(x, y)))
                                .collect();
                            call_args.push(Value::Num(a as f64));
                            call_args.push(Value::Str(Rc::from(s)));
                            let v = self.call(f.clone(), call_args, frame)?;
                            self.to_js_string(&v)
                        }
                        Some(v) => expand_replacement(&self.to_js_string(v), &chars, &caps),
                        None => "undefined".to_string(),
                    };
                    out.push_str(&piece);
                    last = b;
                    if out.len() > MAX_STRING_CHARS {
                        return Ok(Value::Undefined);
                    }
                }
                out.extend(&chars[last.min(chars.len())..]);
                Ok(Value::Str(Rc::from(out)))
            }
        }
    }
}

/// A replacement string with `$$`, `$&`, `` $` ``, `$'` and `$1`..`$99`
/// read for this match.
fn expand_replacement(with: &str, chars: &[char], caps: &Captures) -> String {
    let w: Vec<char> = with.chars().collect();
    let (a, b) = caps[0].unwrap_or((0, 0));
    let mut out = String::new();
    let mut i = 0;
    while i < w.len() {
        if w[i] != '$' || i + 1 >= w.len() {
            out.push(w[i]);
            i += 1;
            continue;
        }
        match w[i + 1] {
            '$' => {
                out.push('$');
                i += 2;
            }
            '&' => {
                out.extend(&chars[a..b]);
                i += 2;
            }
            '`' => {
                out.extend(&chars[..a]);
                i += 2;
            }
            '\'' => {
                out.extend(&chars[b..]);
                i += 2;
            }
            d if d.is_ascii_digit() => {
                let one = d as usize - '0' as usize;
                let two = w
                    .get(i + 2)
                    .filter(|c| c.is_ascii_digit())
                    .map(|c| one * 10 + (*c as usize - '0' as usize));
                let (n, len) = match two {
                    Some(n) if n >= 1 && n < caps.len() => (n, 3),
                    _ => (one, 2),
                };
                if n >= 1 && n < caps.len() {
                    if let Some((x, y)) = caps[n] {
                        out.extend(&chars[x..y]);
                    }
                    i += len;
                } else {
                    out.push('$');
                    i += 1;
                }
            }
            _ => {
                out.push('$');
                i += 1;
            }
        }
    }
    out
}

fn char_at(s: &str, index: f64) -> Value {
    if index < 0.0 || index.is_nan() {
        return Value::Str(Rc::from(""));
    }
    Value::Str(Rc::from(
        s.chars()
            .nth(index as usize)
            .map(String::from)
            .unwrap_or_default(),
    ))
}

fn str_method(name: &str) -> &'static str {
    match name {
        "toString" => "toString",
        "valueOf" => "valueOf",
        "substring" => "substring",
        "substr" => "substr",
        "slice" => "slice",
        "indexOf" => "indexOf",
        "lastIndexOf" => "lastIndexOf",
        "toUpperCase" => "toUpperCase",
        "toLowerCase" => "toLowerCase",
        "charAt" => "charAt",
        "concat" => "concat",
        "trim" => "trim",
        "split" => "split",
        "match" => "match",
        "search" => "search",
        _ => "replace",
    }
}

fn num_method(name: &str) -> &'static str {
    match name {
        "toFixed" => "toFixed",
        _ => "toString",
    }
}

fn presence_name(presence: Presence) -> &'static str {
    match presence {
        Presence::Visible => "visible",
        Presence::Hidden => "hidden",
        Presence::Invisible => "invisible",
        Presence::Inactive => "inactive",
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Undefined | Value::Null => false,
        Value::Bool(b) => *b,
        Value::Num(n) => *n != 0.0 && !n.is_nan(),
        Value::Str(s) => !s.is_empty(),
        _ => true,
    }
}

/// JavaScript's `===`.
fn strict_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Num(x), Value::Num(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Node(x), Value::Node(y)) => x == y,
        _ => false,
    }
}

fn type_of(v: &Value) -> &'static str {
    match v {
        Value::Undefined => "undefined",
        Value::Bool(_) => "boolean",
        Value::Num(_) => "number",
        Value::Str(_) => "string",
        Value::Function(..) | Value::Builtin(_) => "function",
        _ => "object",
    }
}

fn str_to_number(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    if t.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return f64::NAN;
    }
    t.parse().unwrap_or(f64::NAN)
}

fn num_to_string(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// The `n`-th item of a list (`list[n]`, `list.item(n)`).
/// A script object's `var` initializer when it is a constant: a string,
/// number, boolean or null literal, or `+` of such (folded as JavaScript
/// adds them), at most `parse::MAX_DEPTH` deep.
fn const_value(e: &Expr, depth: usize) -> Option<Value> {
    if depth > parse::MAX_DEPTH {
        return None;
    }
    Some(match e {
        Expr::Str(s) => Value::Str(Rc::from(s.as_str())),
        Expr::Num(n) => Value::Num(*n),
        Expr::Bool(b) => Value::Bool(*b),
        Expr::Null => Value::Null,
        Expr::Binary(BinOp::Add, l, r) => {
            match (const_value(l, depth + 1)?, const_value(r, depth + 1)?) {
                (Value::Num(a), Value::Num(b)) => Value::Num(a + b),
                (Value::Str(a), Value::Str(b)) if a.len() + b.len() <= MAX_STRING_CHARS => {
                    Value::Str(Rc::from(format!("{a}{b}")))
                }
                (Value::Str(a), Value::Num(b)) => {
                    Value::Str(Rc::from(format!("{a}{}", num_to_string(b))))
                }
                (Value::Num(a), Value::Str(b)) => {
                    Value::Str(Rc::from(format!("{}{b}", num_to_string(a))))
                }
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// A regular expression value, or `undefined` when the pattern or the
/// flags do not read (`regex`).
fn regexp(pattern: &str, flags: &str) -> Value {
    match Regex::new(pattern, flags) {
        Some(re) => Value::RegExp(Rc::new(RegExpValue {
            re,
            last_index: Cell::new(0),
        })),
        None => Value::Undefined,
    }
}

/// `exec`'s and a non-global `match`'s result: the match, then each
/// group (`undefined` for one that took no part).
fn groups_list(chars: &[char], caps: &Captures) -> Value {
    let items: Vec<Value> = caps
        .iter()
        .map(|c| match c {
            Some((a, b)) => Value::Str(Rc::from(chars[*a..*b].iter().collect::<String>())),
            None => Value::Undefined,
        })
        .collect();
    Value::List(items.into())
}

fn list_item(items: &[Value], n: f64) -> Value {
    if n >= 0.0 && n.fract() == 0.0 {
        items.get(n as usize).cloned().unwrap_or(Value::Undefined)
    } else {
        Value::Undefined
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xfa::types::{Node, Value as XValue};
    use crate::xfa::{bind, template, xml};

    const TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.6/">
<subform name="form1" layout="tb">
  <variables>
    <script name="sco" contentType="application/x-javascript">
      function hide(obj) { obj.presence = "hidden"; }
      function show(obj) { obj.presence = "visible"; }
      function Standard() { Main.Sections.B.presence = "visible"; Kind.value = "unit"; Main.Sections.Note.rawValue = Kind.value; }
      var unused = 1;
      var msg = "Invalid " + "number";
    </script>
    <text name="Kind">common</text>
  </variables>
  <subform name="Main" layout="tb">
    <subform name="Sections" layout="tb">
      <subform name="A" layout="tb"><draw name="T"><value><text>A</text></value></draw></subform>
      <subform name="B" layout="tb" presence="hidden"><draw name="T"><value><text>B</text></value></draw></subform>
      <subform layout="tb">
        <subform name="Inner" layout="tb"><field name="F"><ui><textEdit/></ui></field></subform>
        <subform layout="tb"><field name="Deep"><ui><textEdit/></ui></field></subform>
      </subform>
      <field name="Note"><ui><textEdit/></ui><value><text>note</text></value></field>
      <exclGroup name="Type">
        <field name="One"><ui><checkButton/></ui><items><text>1</text><text>0</text></items></field>
        <field name="Two"><ui><checkButton/></ui><items><text>2</text><text>0</text></items></field>
      </exclGroup>
      <field name="Check"><ui><checkButton/></ui><items><integer>1</integer><integer>0</integer></items></field>
    </subform>
    <subform name="Row" layout="tb"><field name="R"><ui><textEdit/></ui></field></subform>
    <subform name="Row" layout="tb"><field name="R"><ui><textEdit/></ui></field></subform>
  </subform>
</subform>
</template>"#;

    const DATASETS: &str = r#"<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data>
<form1><Main><Sections><Type>2</Type><Note>note</Note><Check>1</Check></Sections><Row><R>r0</R></Row><Row><R>r1</R></Row></Main></form1>
</xfa:data></xfa:datasets>"#;

    fn form() -> Form {
        let mut form = template::read(TEMPLATE).unwrap();
        let datasets = xml::parse(DATASETS).unwrap();
        bind::bind(&mut form, Some(&datasets)).unwrap();
        form
    }

    /// The path of the first node with this SOM name.
    fn find(form: &Form, som: &str) -> NodeRef {
        fn walk(nodes: &[Node], at: &NodeRef, som: &str) -> Option<NodeRef> {
            for (i, node) in nodes.iter().enumerate() {
                let here = at.child(i);
                match node {
                    Node::Subform(s) => {
                        if s.som == som {
                            return Some(here);
                        }
                        if let Some(found) = walk(&s.children, &here, som) {
                            return Some(found);
                        }
                    }
                    Node::Field(f) if f.som == som => return Some(here),
                    Node::Draw(d) if d.common.name == som => return Some(here),
                    Node::ExclGroup(g) => {
                        if g.som == som {
                            return Some(here);
                        }
                        for (j, f) in g.fields.iter().enumerate() {
                            if f.som == som {
                                return Some(here.child(j));
                            }
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        walk(&form.root.children, &NodeRef::default(), som).unwrap_or_else(|| panic!("{som}"))
    }

    fn presence(form: &Form, som: &str) -> Presence {
        view(form, &find(form, som)).unwrap().presence()
    }

    fn value(form: &Form, som: &str) -> String {
        match view(form, &find(form, som)).unwrap() {
            NodeView::Field(f) => f.value.plain(),
            NodeView::Group(g) => g.value.clone(),
            NodeView::Draw(d) => d.value.plain(),
            NodeView::Subform(_) => String::new(),
        }
    }

    fn run(form: &mut Form, ctx: &str, script: &str) {
        let at = find(form, ctx);
        let mut runner = Runner::new(form);
        let event = EventInfo {
            activity: "click".into(),
            target: at.clone(),
            new_text: None,
        };
        runner.run(script, &at, event).expect("within budget");
    }

    /// The status certificate's phone check (`scriptUtils.validation`),
    /// as Designer wrote it: a valid number is written 613-555-1234, any
    /// other is refused with a message and cleared.
    const PHONE_EXIT: &str = r#"
function validatePhoneExit(evnt,lang)
{
	var pattern = /^[0-9]{3}[\s-]?[0-9]{3}[\s-]?[0-9]{4}$/;
	var pattern2 = /^\({1}[0-9]{3}\){1}\s?[0-9]{3}[\s-]?[0-9]{4}$/;
	if(evnt.target.rawValue!=null)
	{
		if(!evnt.target.rawValue.match(pattern) && !evnt.target.rawValue.match(pattern2))
		{
			xfa.host.messageBox(sco.msg);
			xfa.host.setFocus(evnt.target.name);
			evnt.target.rawValue="";
		}
		else
		{
			var displayPat = /[\(\)\s-]/;
			var result = evnt.target.rawValue.split(displayPat);
			result = result.join("");
			var finalResult = result.substring(0,3) + "-" + result.substring(3,6) + "-" + result.substring(6,10);
			evnt.target.rawValue = finalResult;
		}
	}
}
validatePhoneExit(xfa.event);
"#;

    #[test]
    fn a_phone_check_with_regular_expressions_formats_or_refuses() {
        let note = "form1[0].Main[0].Sections[0].Note[0]";
        for (typed, shown) in [
            ("6135551234", "613-555-1234"),
            ("613 555-1234", "613-555-1234"),
            ("(613) 555-1234", "613-555-1234"),
            ("abc", ""),
            ("61355512345", ""),
        ] {
            let mut form = form();
            run(&mut form, note, &format!("this.rawValue = \"{typed}\";"));
            let at = find(&form, note);
            let mut runner = Runner::new(&mut form);
            let event = EventInfo {
                activity: "exit".into(),
                target: at.clone(),
                new_text: None,
            };
            runner.run(PHONE_EXIT, &at, event).expect("within budget");
            let messages = runner.take_messages();
            drop(runner);
            assert_eq!(value(&form, note), shown, "{typed}");
            // The message is a script object's `var`, as the form's is.
            let expected: &[&str] = if shown.is_empty() {
                &["Invalid number"]
            } else {
                &[]
            };
            assert_eq!(messages, expected, "{typed}");
        }
    }

    #[test]
    fn regular_expression_methods() {
        let mut form = form();
        let note = "form1[0].Main[0].Sections[0].Note[0]";
        run(
            &mut form,
            note,
            r#"var out = [];
            var parts = "a1b22c333".split(/\d+/);
            var s = parts.join("|") + ";" + parts.length;
            s += ";" + "2026-11-06".replace(/(\d+)-(\d+)-(\d+)/, "$3/$2/$1");
            s += ";" + "a.b.c".replace(/\./g, "-");
            s += ";" + "x1y22z".match(/\d+/g).length + ":" + "x1y22z".match(/\d+/g)[1];
            var m = "key=val".match(/(\w+)=(\w+)/);
            s += ";" + m[0] + "," + m[2];
            s += ";" + "abc".search(/c/) + "," + "abc".search(/z/);
            var re = new RegExp("o", "g");
            var n = 0;
            while (re.test("foo boo")) { n++; }
            s += ";" + n + "," + re.lastIndex;
            s += ";" + /^\s*$/.test("  ") + "," + RegExp("A", "i").test("cat");
            s += ";" + "AbC".replace(/[a-z]/gi, function (c) { return c.toLowerCase() + "."; });
            s += ";" + (/(/ == undefined);
            this.rawValue = s;"#,
        );
        assert_eq!(
            value(&form, note),
            "a|b|c|;4;06/11/2026;a-b-c;2:22;key=val,val;2,-1;4,0;true,true;a.b.c.;true"
        );
    }

    const ROWS: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.6/">
<subform name="form1" layout="tb">
  <subform name="T" layout="tb">
    <subform name="Row" layout="tb"><occur min="1" max="3"/>
      <field name="R"><ui><textEdit/></ui></field>
      <field name="Add"><ui><button/></ui><bind match="none"/></field>
    </subform>
    <subform name="Free" layout="tb"><bind match="none"/><occur max="-1"/></subform>
    <field name="Out"><ui><textEdit/></ui></field>
  </subform>
</subform>
</template>"#;

    fn rows() -> Form {
        let mut form = template::read(ROWS).unwrap();
        let datasets = xml::parse(
            r#"<xfa:datasets xmlns:xfa="http://www.xfa.org/schema/xfa-data/1.0/"><xfa:data>
<form1><T><Row><R>a</R></Row><Row><R>b</R></Row></T></form1></xfa:data></xfa:datasets>"#,
        )
        .unwrap();
        bind::bind(&mut form, Some(&datasets)).unwrap();
        form
    }

    fn row_paths(n: usize) -> Rc<[DataPath]> {
        (0..n)
            .map(|i| DataPath::parse(&format!("form1[0]/T[0]/Row[{i}]")).unwrap())
            .collect()
    }

    #[test]
    fn instance_managers_record_operations_within_occur() {
        let mut form = rows();
        let at = find(&form, "form1[0].T[0].Row[1].Add[0]");
        let mut runner = Runner::new(&mut form);
        let run = |runner: &mut Runner<'_>, script: &str| {
            let event = EventInfo {
                activity: "click".into(),
                target: at.clone(),
                new_text: None,
            };
            runner.run(script, &at, event).expect("within budget");
        };
        run(
            &mut runner,
            "Out.rawValue = this.parent.index + \":\" + Row.instanceManager.count + \":\" + _Row.max;",
        );
        // Adding stops at max (3); the count follows what was recorded.
        run(
            &mut runner,
            "this.parent.instanceManager.addInstance(0); Row.instanceManager.addInstance(1);\n\
             Out.rawValue = Out.rawValue + \"|\" + _Row.count;",
        );
        let ops = runner.take_instance_ops();
        assert_eq!(
            ops,
            [InstanceOp::Add {
                instances: row_paths(2)
            }]
        );
        // Removing stops at min (1) and refuses an index past the count.
        run(
            &mut runner,
            "_Row.removeInstance(5); _Row.removeInstance(-1); _Row.removeInstance(0.5);\n\
             _Row.removeInstance(1); _Row.removeInstance(0);",
        );
        assert_eq!(
            runner.take_instance_ops(),
            [InstanceOp::Remove {
                instances: row_paths(2),
                index: 1
            }]
        );
        // setInstances: as many adds or removes, clamped to occur.
        run(&mut runner, "_Row.setInstances(9);");
        assert_eq!(runner.take_instance_ops().len(), 1, "2 -> 3");
        run(&mut runner, "_Row.setInstances(0);");
        let ops = runner.take_instance_ops();
        assert_eq!(
            ops,
            [InstanceOp::Remove {
                instances: row_paths(2),
                index: 1
            }],
            "2 -> 1, the last one removed"
        );
        // A subform without data groups takes no operation.
        run(
            &mut runner,
            "Free.instanceManager.addInstance(0); T.instanceManager.addInstance(0);",
        );
        assert!(runner.take_instance_ops().is_empty());
        drop(runner);
        assert_eq!(value(&form, "form1[0].T[0].Out[0]"), "1:2:3|3");
        let rows = form.root.children.iter().find_map(|n| match n {
            Node::Subform(t) => Some(t.children.len()),
            _ => None,
        });
        assert_eq!(
            rows,
            Some(4),
            "the form itself is unchanged: 2 rows, Free, Out"
        );
    }

    #[test]
    fn names_resolve_through_the_scope_chain_and_unnamed_subforms() {
        let mut form = form();
        run(
            &mut form,
            "form1[0].Main[0].Sections[0].Inner[0].F[0]",
            "B.presence = \"visible\"; A.presence = \"hidden\"; Deep.rawValue = \"d\"; \
             Main.Sections.Note.rawValue = this.parent.name + \":\" + this.name; \
             Row[1].R.rawValue = Row.R.rawValue + \"!\"; form1.Main.Row.R.rawValue = Sections.A.T.rawValue;",
        );
        assert_eq!(
            presence(&form, "form1[0].Main[0].Sections[0].B[0]"),
            Presence::Visible
        );
        assert_eq!(
            presence(&form, "form1[0].Main[0].Sections[0].A[0]"),
            Presence::Hidden
        );
        assert_eq!(value(&form, "form1[0].Main[0].Sections[0].Deep[0]"), "d");
        assert_eq!(
            value(&form, "form1[0].Main[0].Sections[0].Note[0]"),
            "Inner:F"
        );
        assert_eq!(value(&form, "form1[0].Main[0].Row[1].R[0]"), "r0!");
        assert_eq!(value(&form, "form1[0].Main[0].Row[0].R[0]"), "A");
    }

    #[test]
    fn script_objects_text_variables_and_switch() {
        let mut form = form();
        run(
            &mut form,
            "form1[0].Main[0].Sections[0].Type[0]",
            "switch (this.rawValue) {\n  case \"1\":\n    sco.hide(Main.Sections.A);\n    break;\n  case \"2\":\n    sco.Standard()\n    sco.hide(Main.Sections.A)\n    break;\n  default:\n    // nothing\n}\nif (Kind.value == \"unit\") { Check.rawValue = 0 } else { Check.rawValue = 1 }",
        );
        assert_eq!(
            presence(&form, "form1[0].Main[0].Sections[0].B[0]"),
            Presence::Visible
        );
        assert_eq!(
            presence(&form, "form1[0].Main[0].Sections[0].A[0]"),
            Presence::Hidden
        );
        assert_eq!(value(&form, "form1[0].Main[0].Sections[0].Note[0]"), "unit");
        let check = find(&form, "form1[0].Main[0].Sections[0].Check[0]");
        let NodeView::Field(f) = view(&form, &check).unwrap() else {
            panic!();
        };
        assert_eq!((f.value.plain().as_str(), f.checked), ("0", false));
    }

    #[test]
    fn groups_checks_and_raw_values() {
        let mut form = form();
        assert_eq!(value(&form, "form1[0].Main[0].Sections[0].Type[0]"), "2");
        run(
            &mut form,
            "form1[0].Main[0].Sections[0].Note[0]",
            "var t = Type.rawValue == 2 && Type.rawValue === \"2\" && !Type.isNull;\n\
             if (t) { Type.rawValue = \"1\"; }\nvar n = Inner.F.rawValue;\n\
             if (n == null && n !== 0 && Inner.F.isNull && typeof n == \"object\") { Inner.F.rawValue = 5; }\n\
             Type.One.rawValue = 0; Check.rawValue = null;",
        );
        let group = find(&form, "form1[0].Main[0].Sections[0].Type[0]");
        let NodeView::Group(g) = view(&form, &group).unwrap() else {
            panic!();
        };
        // Set to 1, then One cleared: the group has no value and no button is checked.
        assert_eq!(g.value, "");
        assert!(g.fields.iter().all(|f| !f.checked));
        assert_eq!(
            value(&form, "form1[0].Main[0].Sections[0].Inner[0].F[0]"),
            "5"
        );
        let check = find(&form, "form1[0].Main[0].Sections[0].Check[0]");
        let NodeView::Field(f) = view(&form, &check).unwrap() else {
            panic!();
        };
        assert_eq!(f.value, XValue::Empty);
        assert!(!f.checked);

        run(
            &mut form,
            "form1[0].Main[0].Sections[0].Note[0]",
            "Type.rawValue = \"2\"; Type.Two.access = \"protected\"; this.access = \"open\";",
        );
        let NodeView::Group(g) = view(&form, &group).unwrap() else {
            panic!();
        };
        assert_eq!(g.value, "2");
        assert!(g.fields[1].checked && !g.fields[0].checked);
        assert!(g.fields[1].common.read_only);
    }

    #[test]
    fn resolve_node_and_xfa_objects() {
        let mut form = form();
        run(
            &mut form,
            "form1[0].Main[0].Sections[0].A[0]",
            "xfa.resolveNode(\"Sections.#subform[2].Inner.F\").rawValue = \"one\";\n\
             xfa.resolveNode(\"Main.Sections.#subform[2].#subform[4].Deep\").rawValue = \"two\";\n\
             xfa.resolveNode(\"Sections.#subform.T\").rawValue = \"a\";\n\
             xfa.resolveNode(\"xfa.form.form1.Main.Row[1].R\").rawValue = \"three\";\n\
             xfa.resolveNode(\"$form.form1.Main.Row.R\").rawValue = \"four\";\n\
             this.resolveNode(\"T\").rawValue = \"five\";\n\
             var missing = xfa.resolveNode(\"Nowhere.X\");\n\
             if (missing == null) { Note.rawValue = \"missing\" }\n\
             xfa.resolveNode(\"Sections.Deep[Note > 1]\").rawValue = \"never\";\n\
             xfa.host.messageBox(\"hi\"); xfa.host.setFocus(\"x\");\n\
             if (typeof xfa.host.name !== 'undefined' && xfa.host.name == \"XFAPresentationAgent\") { Note.rawValue = \"server\" }\n\
             if (xfa.event.target.name == \"A\" && xfa.event.newText == undefined) { B.presence = \"invisible\" }\n\
             Row.instanceManager.addInstance(0); _Row.removeInstance(0); this.parent.parent.index;\n\
             if (Row.instanceManager.count == 1) { Note.rawValue = Note.rawValue + \"!\" }",
        );
        assert_eq!(
            value(&form, "form1[0].Main[0].Sections[0].Inner[0].F[0]"),
            "one"
        );
        assert_eq!(value(&form, "form1[0].Main[0].Sections[0].Deep[0]"), "two");
        assert_eq!(value(&form, "form1[0].Main[0].Row[1].R[0]"), "three");
        assert_eq!(value(&form, "form1[0].Main[0].Row[0].R[0]"), "four");
        assert_eq!(
            value(&form, "T"),
            "five",
            "set to \"a\" by #subform (the first subform child, named or not), then by this.resolveNode"
        );
        assert_eq!(
            value(&form, "form1[0].Main[0].Sections[0].Note[0]"),
            "missing!",
            "the add is refused (each Row's max is 1), the remove recorded: the count is 1"
        );
        assert_eq!(
            presence(&form, "form1[0].Main[0].Sections[0].B[0]"),
            Presence::Invisible
        );
    }

    #[test]
    fn conversions_follow_javascript() {
        let mut form = form();
        run(
            &mut form,
            "form1[0].Main[0].Sections[0].Note[0]",
            "var s = \"\" + 1 + 2.5 + null + undefined + true + (1 + 2) + (\"3\" * \"4\") + (5 / 0);\n\
             this.rawValue = s;\n\
             var c = (\"1\" == 1) && (null == undefined) && !(null == 0) && !(\"\" == null) && (\"abc\" < \"abd\") && (2 < 10) && !(\"2\" < \"10\") && (\"\" == 0) && (true == 1);\n\
             var d = (1 ? \"y\" : \"n\") + (0 || \"z\") + (\"\" && \"w\") + (\"x\".length) + \"hello\".substring(1, 3) + \"Hello\".toUpperCase() + \"abc\".indexOf(\"c\") + (12.5).toFixed(1) + (7).toString();\n\
             Inner.F.rawValue = c ? d : \"wrong\";\n\
             var i = 0; while (i < 3) { i++ } for (var j = 0; j < 2; j++) { i += 10 } do { i-- } while (i > 100);\n\
             Deep.rawValue = i;",
        );
        assert_eq!(
            value(&form, "form1[0].Main[0].Sections[0].Note[0]"),
            "12.5nullundefinedtrue312Infinity"
        );
        assert_eq!(
            value(&form, "form1[0].Main[0].Sections[0].Inner[0].F[0]"),
            "yz1elHELLO212.57"
        );
        assert_eq!(value(&form, "form1[0].Main[0].Sections[0].Deep[0]"), "22");
    }

    #[test]
    fn unknown_constructs_and_errors_do_not_stop_the_rest() {
        let mut form = form();
        run(
            &mut form,
            "form1[0].Main[0].Sections[0].Note[0]",
            "var re = /ab+c/g;\nundefinedThing.foo.bar = 1;\nnope();\nxfa.resolveNode(\"Missing\").rawValue = 2;\n\
             var p = new RegExp(\"x\"); p.test(\"y\");\nthis.rawValue.split(\",\");\n\
             app.alert(\"no\"); try { Inner.F.rawValue = \"ok\" } catch (e) { Inner.F.rawValue = \"caught\" }\n\
             for (k in this) { Deep.rawValue = \"loop\" }\nA.presence = \"hidden\"; this.width = 3; this.border.fill.color.value = \"1,2,3\";",
        );
        assert_eq!(
            value(&form, "form1[0].Main[0].Sections[0].Inner[0].F[0]"),
            "ok"
        );
        assert_eq!(value(&form, "form1[0].Main[0].Sections[0].Deep[0]"), "");
        assert_eq!(
            presence(&form, "form1[0].Main[0].Sections[0].A[0]"),
            Presence::Hidden
        );
    }

    #[test]
    fn budgets_stop_runaway_scripts() {
        let mut form = form();
        let at = find(&form, "form1[0].Main[0].Sections[0].Note[0]");
        let mut runner = Runner::new(&mut form);
        assert_eq!(
            runner.run(
                "while (true) { Inner.F.rawValue = 1 }",
                &at,
                EventInfo::default()
            ),
            Err(Stopped)
        );
        assert_eq!(
            runner.run(
                "function f(n) { return f(n + 1) } Deep.rawValue = f(0); Inner.F.rawValue = 2",
                &at,
                EventInfo::default()
            ),
            Ok(())
        );
        assert_eq!(
            runner.run(
                "var s = \"x\"; for (var i = 0; i < 40; i++) { s = s + s } Deep.rawValue = s",
                &at,
                EventInfo::default()
            ),
            Ok(())
        );
        assert!(!runner.exhausted());
        drop(runner);
        assert_eq!(
            value(&form, "form1[0].Main[0].Sections[0].Inner[0].F[0]"),
            "2"
        );
        assert_eq!(
            value(&form, "form1[0].Main[0].Sections[0].Deep[0]"),
            "NaN",
            "a string past the bound is undefined, and undefined + undefined is NaN"
        );
        // The runner's own budget runs out across scripts.
        let mut runner = Runner::new(&mut form);
        runner.steps = 5;
        assert_eq!(
            runner.run(
                "var a = 1; var b = 2; var c = 3; var d = 4;",
                &at,
                EventInfo::default()
            ),
            Err(Stopped)
        );
        assert!(runner.exhausted());
        assert_eq!(
            runner.run("Deep.rawValue = 1", &at, EventInfo::default()),
            Err(Stopped)
        );
    }

    #[test]
    fn class_steps_count_in_the_named_scope() {
        // The three shapes of a Designer form: a class step from an
        // unnamed subform, one from a named subform whose second child is
        // unnamed, and one whose first subform is unnamed.
        const SHAPES: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.6/">
<subform name="form1" layout="tb">
  <subform name="Section22" layout="tb">
    <draw name="Text2"><value><text>t</text></value></draw>
    <subform layout="tb">
      <subform name="Question" layout="tb"><field name="Q"><ui><textEdit/></ui></field></subform>
      <subform layout="tb">
        <subform name="NotSubject" layout="tb"/>
        <subform name="Subject" layout="tb"><field name="TypeOwner"><ui><textEdit/></ui></field></subform>
      </subform>
    </subform>
  </subform>
  <subform name="Section26" layout="tb">
    <subform name="Paragraph" layout="tb">
      <subform name="Question" layout="tb"/>
      <subform layout="tb">
        <subform name="NotOwn" layout="tb"><field name="Types"><ui><textEdit/></ui></field></subform>
      </subform>
    </subform>
  </subform>
  <subform name="Section7" layout="tb">
    <draw name="Text2"><value><text>t</text></value></draw>
    <subform layout="tb">
      <subform name="Paragraph" layout="tb"><subform name="TypeText" layout="tb"><field name="Type"><ui><textEdit/></ui></field></subform></subform>
      <subform name="PrintParagraph" layout="tb"/>
    </subform>
    <subform name="Section7_1" layout="tb"><field name="Last"><ui><textEdit/></ui></field></subform>
  </subform>
</subform>
</template>"#;
        let mut form = template::read(SHAPES).unwrap();
        bind::bind(&mut form, None).unwrap();
        run(
            &mut form,
            "form1[0].Section26[0]",
            "xfa.resolveNode(\"Section22.#subform.#subform[2].Subject.TypeOwner\").rawValue = \"owner\";\n\
             xfa.resolveNode(\"Section26.Paragraph.#subform[1].NotOwn.Types\").rawValue = \"types\";\n\
             xfa.resolveNode(\"Section7.#subform[0].Paragraph.TypeText.Type\").rawValue = \"type\";\n\
             xfa.resolveNode(\"Section7.#subform[3].Last\").rawValue = \"last\";\n\
             xfa.resolveNode(\"Section22.#subform[1].Q\").rawValue = \"q\";\n\
             if (xfa.resolveNode(\"Section22.#field\") == null) { Section7.Section7_1.Last.rawValue += \"!\" }",
        );
        let at = |som: &str| value(&form, som);
        assert_eq!(at("form1[0].Section22[0].Subject[0].TypeOwner[0]"), "owner");
        assert_eq!(
            at("form1[0].Section26[0].Paragraph[0].NotOwn[0].Types[0]"),
            "types"
        );
        assert_eq!(
            at("form1[0].Section7[0].Paragraph[0].TypeText[0].Type[0]"),
            "type"
        );
        assert_eq!(
            at("form1[0].Section7[0].Section7_1[0].Last[0]"),
            "last!",
            "Q is under the named Question, not a field of Section22's scope"
        );
        assert_eq!(at("form1[0].Section22[0].Question[0].Q[0]"), "q");
    }

    #[test]
    fn resolve_nodes_gives_lists() {
        let mut form = form();
        run(
            &mut form,
            "form1[0].Main[0].Sections[0].A[0]",
            "xfa.resolveNode(\"Main.Row[0].R\").rawValue = 2;\n\
             xfa.resolveNode(\"Main.Row[1].R\").rawValue = 3;\n\
             var l = xfa.resolveNodes(\"Row[*].R\");\n\
             var t = 0;\n\
             for (var i = 0; i < l.length; i++) { t = t + l.item(i).rawValue * 1; }\n\
             var d = xfa.resolveNodes(\"$record.Main.Row[*]\");\n\
             var deep = this.resolveNodes(\"$form..R[*]\");\n\
             Note.rawValue = l.length + \":\" + t + \":\" + l[1].rawValue + \":\" + (l.item(5) == null) + \":\" + d.length + \":\" + deep.length + \":\" + xfa.resolveNode(\"Sections..Deep\").name + \":\" + xfa.resolveNodes(\"Nowhere[*]\").length;",
        );
        assert_eq!(
            value(&form, "form1[0].Main[0].Sections[0].Note[0]"),
            "2:5:3:true:2:2:Deep:0"
        );
    }
}
