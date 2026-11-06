// SPDX-License-Identifier: MIT
//! The form's scripts, run where Acrobat would run them: `parse` reads
//! JavaScript, `eval` runs it against the bound form, `formcalc` reads and
//! runs FormCalc (XFA's default language: any script whose `contentType`
//! does not name JavaScript) through the same runner. This module picks
//! the events: at materialize time every `initialize` event in document order
//! (a node before its children, a group before its buttons), then the
//! `ready` events (`$form` first, then `$layout`), as Acrobat fires them
//! at open; on a fill, the handlers of the filled node (`click` and
//! `change` for a check button or a radio group and its chosen button,
//! `change` and `exit` for anything else), then the node's `validate`
//! scripts. `calculate` runs after the open-time events and again after
//! a fill: every `calculate` script in document order sets its
//! node's value to the script's completion value, in a second pass when
//! the first changed a value (so a calculation reading another
//! calculated field settles), never more. A field's validation (its
//! `validate` scripts, `nullTest`, `formatTest`) records a message when
//! it fails, as does `xfa.host.messageBox`: `fire`, `click` and
//! `validate` return the messages their handlers recorded in
//! `Report::messages` (`MAX_MESSAGES` at most, each `MAX_MESSAGE_BYTES`
//! at most). A failed validation keeps the value (Acrobat keeps it too,
//! and flags the field); nothing is rejected. Every event is skipped once
//! the runner's budget is spent.

pub mod eval;
pub mod fclib;
pub mod formcalc;
pub mod parse;
pub mod regex;
pub mod som;

pub use eval::{
    EventInfo, MAX_MESSAGE_BYTES, MAX_MESSAGES, NodeRef, NodeView, Runner, Stopped, Value, view,
};

use super::types::{DataPath, Event, Field, Form, Node};

/// What a run of events did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Scripts run to their end.
    pub ran: usize,
    /// Scripts left alone. None today: a script is JavaScript or FormCalc,
    /// and both run; kept for a language neither reads.
    pub skipped: usize,
    /// Scripts a bound stopped part way (the runner's budget included).
    pub stopped: usize,
    /// Messages for the user, in order: `xfa.host.messageBox` texts and
    /// failed validations' messages (`MAX_MESSAGES` at most).
    pub messages: Vec<String>,
}

impl Report {
    fn merge(&mut self, other: Report) {
        self.ran += other.ran;
        self.skipped += other.skipped;
        self.stopped += other.stopped;
        let room = MAX_MESSAGES.saturating_sub(self.messages.len());
        self.messages.extend(other.messages.into_iter().take(room));
    }

    fn add(&mut self, outcome: Result<(), Stopped>) {
        match outcome {
            Ok(()) => self.ran += 1,
            Err(Stopped) => self.stopped += 1,
        }
    }
}

impl Runner<'_> {
    /// The open-time events: `initialize` in document order, then `ready`.
    pub fn initialize(&mut self) -> Report {
        let mut report = Report::default();
        let nodes = self.document_order();
        self.run_activity(&nodes, "initialize", None, &mut report);
        self.run_activity(&nodes, "ready", Some("$form"), &mut report);
        self.run_activity(&nodes, "ready", Some("$layout"), &mut report);
        report
    }

    /// The handlers of the node bound to `path` after it was filled with
    /// `value` (the save value of a button, the text of anything else).
    pub fn fire(&mut self, path: &DataPath, value: &str) -> Report {
        let mut report = Report::default();
        let Some(target) = self.bound_node(path) else {
            return report;
        };
        let (button, chosen) = match view(self.form, &target) {
            Some(NodeView::Field(f)) => (f.is_check(), None),
            Some(NodeView::Group(g)) => {
                let chosen = g
                    .fields
                    .iter()
                    .position(|f| f.check_values().0 == value)
                    .map(|i| target.child(i));
                (true, chosen)
            }
            _ => return report,
        };
        let activities: &[&str] = if button {
            &["click", "change"]
        } else {
            &["change", "exit"]
        };
        for activity in activities {
            self.run_node(&target, activity, None, Some(value), &mut report);
        }
        if let Some(chosen) = chosen {
            for activity in activities {
                self.run_node(&chosen, activity, None, Some(value), &mut report);
            }
        }
        report.messages = self.take_messages();
        let validated = self.validate(&target);
        report.merge(validated);
        report
    }

    /// The `click` handlers of the push button whose SOM name is `som`
    /// (a click on its widget). Nothing runs when no field has that name.
    pub fn click(&mut self, som: &str) -> Report {
        let mut report = Report::default();
        let Some(target) = self.field_named(som) else {
            return report;
        };
        self.run_node(&target, "click", None, None, &mut report);
        report.messages = self.take_messages();
        report
    }

    /// Calculation passes over the whole form: one, and a second when the
    /// first changed a value.
    pub fn calculate(&mut self) -> Report {
        let mut report = Report::default();
        let nodes: Vec<(NodeRef, Vec<Event>)> = self
            .document_order()
            .into_iter()
            .filter_map(|at| {
                let events: Vec<Event> = node_events(self.form, &at)
                    .iter()
                    .filter(|e| e.activity == "calculate")
                    .cloned()
                    .collect();
                (!events.is_empty()).then_some((at, events))
            })
            .collect();
        for _ in 0..2 {
            let mut changed = false;
            for (at, events) in &nodes {
                for event in events {
                    if self.exhausted() {
                        return report;
                    }
                    let info = EventInfo {
                        activity: "calculate".to_string(),
                        target: at.clone(),
                        new_text: None,
                    };
                    let before = self.raw_value(at);
                    let result = match self.run_script(event, at, info) {
                        Ok(value) => {
                            report.ran += 1;
                            value
                        }
                        Err(Stopped) => {
                            report.stopped += 1;
                            continue;
                        }
                    };
                    match result {
                        Value::Str(ref s) if s.is_empty() => self.set_raw_value(at, &Value::Null),
                        Value::Null
                        | Value::Bool(_)
                        | Value::Num(_)
                        | Value::Str(_)
                        | Value::Node(_)
                        | Value::Data(_)
                        | Value::TextVar(..) => self.set_raw_value(at, &result),
                        _ => {}
                    }
                    changed |= !same_value(&before, &self.raw_value(at));
                }
            }
            if !changed {
                break;
            }
        }
        report
    }

    /// Validates `at`: its `validate` scripts run, and a field's tests
    /// are checked. A script whose value is false (or 0) fails the
    /// `scriptTest`, an empty value the `nullTest`, a value the
    /// `validate` picture does not format the `formatTest`; each enabled
    /// test that fails records its message (the template's
    /// `validate/message/text` for the test, else "<caption or name> is
    /// invalid"), the first failing test only. The value stands either way.
    pub fn validate(&mut self, at: &NodeRef) -> Report {
        let mut report = Report::default();
        let events: Vec<Event> = node_events(self.form, at)
            .iter()
            .filter(|e| e.activity == "validate")
            .cloned()
            .collect();
        let mut script_failed = false;
        for event in events {
            if self.exhausted() {
                break;
            }
            let info = EventInfo {
                activity: "validate".to_string(),
                target: at.clone(),
                new_text: None,
            };
            match self.run_script(&event, at, info) {
                Ok(value) => {
                    report.ran += 1;
                    script_failed |= matches!(value, Value::Bool(false))
                        || matches!(value, Value::Num(n) if n == 0.0);
                }
                Err(Stopped) => report.stopped += 1,
            }
        }
        let failure = match view(self.form, at) {
            Some(NodeView::Field(f)) => validation_failure(self.form, f, script_failed),
            Some(NodeView::Group(g)) if script_failed => {
                Some(default_message(&g.common.name, "", &g.common.tooltip))
            }
            _ => None,
        };
        if let Some(text) = failure {
            self.record_message(&text);
        }
        report.messages = self.take_messages();
        report
    }

    /// Every node in document order, a parent before its children.
    fn document_order(&self) -> Vec<NodeRef> {
        let mut ordered = Vec::new();
        let mut stack = vec![NodeRef::default()];
        while let Some(at) = stack.pop() {
            let Some(node) = view(self.form, &at) else {
                continue;
            };
            // Pushed in reverse so the first child is visited first.
            for i in (0..node.child_count()).rev() {
                stack.push(at.child(i));
            }
            ordered.push(at);
        }
        ordered
    }

    fn run_activity(
        &mut self,
        nodes: &[NodeRef],
        activity: &str,
        reference: Option<&str>,
        report: &mut Report,
    ) {
        for at in nodes {
            if self.exhausted() {
                return;
            }
            self.run_node(at, activity, reference, None, report);
        }
    }

    /// Runs every event of `at` with this activity (and `ref`, when given).
    fn run_node(
        &mut self,
        at: &NodeRef,
        activity: &str,
        reference: Option<&str>,
        new_text: Option<&str>,
        report: &mut Report,
    ) {
        let events = node_events(self.form, at).to_vec();
        for event in events {
            if event.activity != activity
                || reference.is_some_and(|r| event.reference != r)
                || (reference.is_none() && activity == "ready")
            {
                continue;
            }
            let info = EventInfo {
                activity: activity.to_string(),
                target: at.clone(),
                new_text: new_text.map(str::to_string),
            };
            report.add(self.run_script(&event, at, info).map(|_| ()));
        }
    }

    /// Runs one event's script in its language, returning its value.
    fn run_script(
        &mut self,
        event: &Event,
        at: &NodeRef,
        info: EventInfo,
    ) -> Result<Value, Stopped> {
        if event.javascript {
            self.run_value(&event.script, at, info)
        } else {
            self.run_formcalc(&event.script, at, info)
        }
    }

    /// The field whose SOM name is `som`.
    fn field_named(&self, som: &str) -> Option<NodeRef> {
        let mut stack = vec![NodeRef::default()];
        while let Some(at) = stack.pop() {
            let Some(node) = view(self.form, &at) else {
                continue;
            };
            match node {
                NodeView::Field(f) if f.som == som => return Some(at),
                NodeView::Subform(s) => {
                    // Only down the subform whose name the SOM name extends.
                    if at.0.is_empty() || som.starts_with(&format!("{}.", s.som)) {
                        stack.extend((0..s.children.len()).map(|i| at.child(i)));
                    }
                }
                NodeView::Group(g) if som.starts_with(&format!("{}.", g.som)) => {
                    stack.extend((0..g.fields.len()).map(|i| at.child(i)));
                }
                _ => {}
            }
        }
        None
    }

    /// The field or group whose data path is `path`.
    fn bound_node(&self, path: &DataPath) -> Option<NodeRef> {
        fn walk(nodes: &[Node], at: &NodeRef, path: &DataPath) -> Option<NodeRef> {
            for (i, node) in nodes.iter().enumerate() {
                let here = at.child(i);
                match node {
                    Node::Subform(s) => {
                        if let Some(found) = walk(&s.children, &here, path) {
                            return Some(found);
                        }
                    }
                    Node::Field(f) if f.data.as_ref() == Some(path) => return Some(here),
                    Node::ExclGroup(g) if g.data.as_ref() == Some(path) => return Some(here),
                    _ => {}
                }
            }
            None
        }
        walk(&self.form.root.children, &NodeRef::default(), path)
    }
}

/// The message of `f`'s first failing validation test, when one fails
/// (`script_failed`: its `validate` script said false). A field without a
/// `validate` element has only its script test (on, as XFA's default).
fn validation_failure(form: &Form, f: &Field, script_failed: bool) -> Option<String> {
    let default = || default_message(&f.common.name, &f.caption_text(), &f.common.tooltip);
    let Some(v) = &f.validate else {
        return script_failed.then(default);
    };
    let pick = |own: &str| {
        if own.is_empty() {
            default()
        } else {
            own.to_string()
        }
    };
    let empty = f.value.is_empty() || f.value.plain().trim().is_empty();
    if v.null_test.on() && empty {
        return Some(pick(&v.null_message));
    }
    if v.format_test.on()
        && !empty
        && v.picture
            .as_ref()
            .is_some_and(|p| !super::picture::validates(form, f, &p.0))
    {
        return Some(pick(&v.format_message));
    }
    if v.script_test.on() && script_failed {
        return Some(pick(&v.script_message));
    }
    None
}

/// "<caption or tooltip or name> is invalid".
fn default_message(name: &str, caption: &str, tooltip: &str) -> String {
    let label = [caption, tooltip, name]
        .into_iter()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or("This field");
    format!("{label} is invalid")
}

/// The events (and `calculate`/`validate` scripts) of the node at `at`.
fn node_events<'a>(form: &'a Form, at: &NodeRef) -> &'a [Event] {
    match view(form, at) {
        Some(NodeView::Subform(s)) => &s.events,
        Some(NodeView::Field(f)) => &f.events,
        Some(NodeView::Group(g)) => &g.events,
        _ => &[],
    }
}

/// Two raw values (`Null`, `Str` or `Undefined`) are the same.
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Null, Value::Null) | (Value::Undefined, Value::Undefined) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xfa::types::{Form, Presence};
    use crate::xfa::{bind, template, xml};

    const TEMPLATE: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.6/">
<subform name="form1" layout="tb">
  <event activity="initialize"><script contentType="application/x-javascript">S.Log.rawValue = "root";</script></event>
  <subform name="S" layout="tb">
    <event activity="initialize"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",S";</script></event>
    <event activity="ready" ref="$layout"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",layout";</script></event>
    <event activity="ready" ref="$form"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",form";</script></event>
    <field name="Log"><ui><textEdit/></ui>
      <event activity="initialize"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",Log";</script></event>
    </field>
    <field name="Calc"><ui><textEdit/></ui>
      <event activity="initialize"><script contentType="application/x-formcalc">$ = 1</script></event>
    </field>
    <exclGroup name="G">
      <event activity="change"><script contentType="application/x-javascript">if (this.rawValue == "b") { Hidden.presence = "visible"; Shown.presence = "hidden"; } Log.rawValue = Log.rawValue + ",Gchange:" + xfa.event.newText;</script></event>
      <field name="A"><ui><checkButton/></ui><items><text>a</text><text>off</text></items></field>
      <field name="B"><ui><checkButton/></ui><items><text>b</text><text>off</text></items>
        <event activity="click"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",Bclick";</script></event>
      </field>
    </exclGroup>
    <field name="C"><ui><checkButton/></ui><items><integer>1</integer><integer>0</integer></items>
      <event activity="click"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",Cclick:" + this.rawValue;</script></event>
    </field>
    <field name="T"><ui><textEdit/></ui>
      <event activity="exit"><script contentType="application/x-javascript">Log.rawValue = Log.rawValue + ",Texit:" + xfa.event.target.rawValue;</script></event>
    </field>
    <subform name="Hidden" presence="hidden"><draw name="D"/></subform>
    <subform name="Shown"><draw name="D"/></subform>
  </subform>
</subform>
</template>"#;

    fn form(datasets: &str) -> Form {
        let mut form = template::read(TEMPLATE).unwrap();
        let datasets = xml::parse(datasets).unwrap();
        bind::bind(&mut form, Some(&datasets)).unwrap();
        form
    }

    fn log(form: &Form) -> String {
        let crate::xfa::types::Node::Subform(s) = &form.root.children[0] else {
            panic!();
        };
        let crate::xfa::types::Node::Field(f) = &s.children[0] else {
            panic!();
        };
        f.value.plain()
    }

    fn calc_field(form: &Form) -> String {
        let crate::xfa::types::Node::Subform(s) = &form.root.children[0] else {
            panic!();
        };
        let crate::xfa::types::Node::Field(f) = &s.children[1] else {
            panic!();
        };
        f.value.plain()
    }

    fn presence_of(form: &Form, name: &str) -> Presence {
        let crate::xfa::types::Node::Subform(s) = &form.root.children[0] else {
            panic!();
        };
        s.children
            .iter()
            .find(|n| n.common().name == name)
            .unwrap()
            .common()
            .presence
    }

    const DATA: &str = "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><xfa:data>\
        <form1><S><Log>L</Log><G>b</G><C>1</C><T>hello</T></S></form1></xfa:data></xfa:datasets>";

    #[test]
    fn initialize_runs_in_document_order_then_ready() {
        let mut form = form(DATA);
        let mut runner = Runner::new(&mut form);
        let report = runner.initialize();
        assert_eq!(
            report,
            Report {
                ran: 6,
                skipped: 0,
                stopped: 0,
                messages: Vec::new(),
            }
        );
        drop(runner);
        assert_eq!(log(&form), "root,S,Log,form,layout");
        assert_eq!(calc_field(&form), "1", "the FormCalc initialize ran");
        // Data alone does not fire change handlers at open.
        assert_eq!(presence_of(&form, "Hidden"), Presence::Hidden);
    }

    #[test]
    fn a_fill_fires_the_node_and_its_chosen_button() {
        let mut form = form(DATA);
        let mut runner = Runner::new(&mut form);
        let group = DataPath::parse("form1[0]/S[0]/G[0]").unwrap();
        assert_eq!(runner.fire(&group, "b").ran, 2);
        let check = DataPath::parse("form1[0]/S[0]/C[0]").unwrap();
        assert_eq!(runner.fire(&check, "1").ran, 1);
        let text = DataPath::parse("form1[0]/S[0]/T[0]").unwrap();
        assert_eq!(runner.fire(&text, "hello").ran, 1);
        let missing = DataPath::parse("form1[0]/S[0]/Nope[0]").unwrap();
        assert_eq!(runner.fire(&missing, "x"), Report::default());
        drop(runner);
        assert_eq!(log(&form), "L,Gchange:b,Bclick,Cclick:1,Texit:hello");
        assert_eq!(presence_of(&form, "Hidden"), Presence::Visible);
        assert_eq!(presence_of(&form, "Shown"), Presence::Hidden);
    }

    const CALC: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.6/">
<subform name="form1" layout="tb">
  <subform name="S" layout="tb">
    <field name="Plus"><ui><textEdit/></ui>
      <calculate><script contentType="application/x-javascript">Double.rawValue * 1 + 1</script></calculate>
    </field>
    <field name="Double"><ui><textEdit/></ui>
      <calculate><script contentType="application/x-javascript">A.rawValue * 2</script></calculate>
    </field>
    <field name="A"><ui><textEdit/></ui>
      <validate><script contentType="application/x-javascript">B.rawValue = "touched"; this.rawValue > 0</script></validate>
    </field>
    <field name="B"><ui><textEdit/></ui></field>
    <field name="Set"><ui><textEdit/></ui>
      <calculate><script contentType="application/x-javascript">if (A.isNull) { this.rawValue = "none"; } else { this.rawValue = "some"; }</script></calculate>
    </field>
    <field name="Empty"><ui><textEdit/></ui><value><text>was</text></value>
      <calculate><script contentType="application/x-javascript">""</script></calculate>
    </field>
    <field name="Formcalc"><ui><textEdit/></ui><value><text>kept</text></value>
      <calculate><script contentType="application/x-formcalc">$ = 7</script></calculate>
    </field>
  </subform>
</subform>
</template>"#;

    fn calc_form(a: &str) -> Form {
        let mut form = template::read(CALC).unwrap();
        let data = format!(
            "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><xfa:data>\
             <form1><S><A>{a}</A></S></form1></xfa:data></xfa:datasets>"
        );
        let datasets = xml::parse(&data).unwrap();
        bind::bind(&mut form, Some(&datasets)).unwrap();
        form
    }

    fn values(form: &Form) -> Vec<(String, String)> {
        let crate::xfa::types::Node::Subform(s) = &form.root.children[0] else {
            panic!();
        };
        s.children
            .iter()
            .map(|n| match n {
                crate::xfa::types::Node::Field(f) => (f.common.name.clone(), f.value.plain()),
                _ => panic!(),
            })
            .collect()
    }

    fn value_of(form: &Form, name: &str) -> String {
        values(form).into_iter().find(|(n, _)| n == name).unwrap().1
    }

    #[test]
    fn calculations_set_values_and_settle_in_a_second_pass() {
        let mut form = calc_form("3");
        let mut runner = Runner::new(&mut form);
        let report = runner.calculate();
        // Four JavaScript calculations and one FormCalc, twice: Plus read
        // Double before Double was calculated, so the first pass changed
        // values.
        assert_eq!(
            report,
            Report {
                ran: 10,
                skipped: 0,
                stopped: 0,
                messages: Vec::new(),
            }
        );
        drop(runner);
        assert_eq!(value_of(&form, "Double"), "6");
        assert_eq!(value_of(&form, "Plus"), "7");
        assert_eq!(value_of(&form, "Set"), "some", "this.rawValue set inside");
        assert_eq!(value_of(&form, "Empty"), "", "an empty result empties");
        assert_eq!(
            value_of(&form, "Formcalc"),
            "7",
            "FormCalc's `$ = 7` set it"
        );
        assert_eq!(value_of(&form, "B"), "", "validate runs only on a fill");
    }

    #[test]
    fn a_settled_form_calculates_once() {
        let mut form = calc_form("3");
        let mut runner = Runner::new(&mut form);
        runner.calculate();
        // Everything holds its calculated value now: one pass changes
        // nothing, so no second runs.
        assert_eq!(runner.calculate().ran, 5);
        let mut form = calc_form("");
        let mut runner = Runner::new(&mut form);
        runner.calculate();
        drop(runner);
        assert_eq!(value_of(&form, "Set"), "none");
        assert_eq!(value_of(&form, "Double"), "0", "null * 2 is 0");
    }

    #[test]
    fn a_fill_runs_the_fields_validate_and_keeps_the_value() {
        let mut form = calc_form("-1");
        let before = values(&form);
        let mut runner = Runner::new(&mut form);
        let a = DataPath::parse("form1[0]/S[0]/A[0]").unwrap();
        let report = runner.fire(&a, "-1");
        assert_eq!(report.ran, 1);
        assert_eq!(
            report.messages,
            ["A is invalid"],
            "no message in the template"
        );
        let b = DataPath::parse("form1[0]/S[0]/B[0]").unwrap();
        assert_eq!(runner.fire(&b, "x"), Report::default());
        drop(runner);
        // The script ran (it set B); its false result changed no value.
        assert_eq!(value_of(&form, "B"), "touched");
        assert_eq!(value_of(&form, "A"), "-1");
        let after = values(&form);
        let changed: Vec<_> = before.iter().zip(&after).filter(|(x, y)| x != y).collect();
        assert_eq!(changed.len(), 1);
    }

    const FORMCALC: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.6/">
<subform name="form1" layout="tb">
  <subform name="S" layout="tb">
    <field name="A"><ui><textEdit/></ui></field>
    <field name="Double"><ui><textEdit/></ui>
      <calculate><script>A * 2</script></calculate>
    </field>
    <subform name="Extra" layout="tb">
      <event activity="initialize"><script contentType="application/x-formcalc">if (A &lt; 5) then
  $.presence = "hidden" ; small orders need no extra section
endif</script></event>
      <draw name="D"/>
    </subform>
  </subform>
</subform>
</template>"#;

    #[test]
    fn formcalc_calculations_and_events_run() {
        let mut form = template::read(FORMCALC).unwrap();
        let datasets = xml::parse(
            "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><xfa:data>\
             <form1><S><A>3</A></S></form1></xfa:data></xfa:datasets>",
        )
        .unwrap();
        bind::bind(&mut form, Some(&datasets)).unwrap();
        let mut runner = Runner::new(&mut form);
        assert_eq!(runner.initialize().ran, 1);
        assert_eq!(
            runner.calculate().ran,
            2,
            "the first pass set Double: a second ran"
        );
        drop(runner);
        let crate::xfa::types::Node::Subform(s) = &form.root.children[0] else {
            panic!();
        };
        let crate::xfa::types::Node::Field(double) = &s.children[1] else {
            panic!();
        };
        assert!(!double.events[0].javascript, "no contentType is FormCalc");
        assert_eq!(double.value.plain(), "6");
        assert_eq!(presence_of(&form, "Extra"), Presence::Hidden);
    }

    const VALIDATED: &str = r#"<template xmlns="http://www.xfa.org/schema/xfa-template/3.6/">
<subform name="form1" layout="tb">
  <subform name="S" layout="tb">
    <field name="Box"><ui><textEdit/></ui>
      <event activity="change"><script contentType="application/x-javascript">
        for (var i = 0; i &lt; 20; i++) { xfa.host.messageBox("note " + i, "Title", 3, 1); }
        xfa.host.setFocus("Box"); xfa.host.resetData();
        this.rawValue = xfa.host.messageBox("x".repeat(3000)) == 1 ? "ok" : "no";
      </script></event>
    </field>
    <field name="Positive"><caption><value><text>Amount</text></value></caption><ui><textEdit/></ui>
      <validate scriptTest="error"><message><text name="scriptTest">Enter a positive amount.</text></message>
        <script contentType="application/x-javascript">this.rawValue &gt; 0</script></validate>
    </field>
    <field name="Required"><ui><textEdit/></ui>
      <validate nullTest="error"><message><text name="nullTest">This one is required.</text></message></validate>
    </field>
    <field name="Warned"><ui><textEdit/></ui><validate nullTest="warning"/></field>
    <field name="Digits"><ui><textEdit/></ui>
      <validate formatTest="error"><picture>text{999}</picture>
        <message><text name="formatTest">Three digits, please.</text></message></validate>
    </field>
    <field name="Lenient"><ui><textEdit/></ui><validate formatTest="disabled"><picture>text{999}</picture></validate></field>
    <field name="Fc"><ui><textEdit/></ui>
      <event activity="change"><script>$host.messageBox("from FormCalc")</script></event>
    </field>
  </subform>
</subform>
</template>"#;

    fn validated(data: &str) -> Form {
        let mut form = template::read(VALIDATED).unwrap();
        let datasets = xml::parse(&format!(
            "<xfa:datasets xmlns:xfa=\"http://www.xfa.org/schema/xfa-data/1.0/\"><xfa:data>\
             <form1><S>{data}</S></form1></xfa:data></xfa:datasets>"
        ))
        .unwrap();
        bind::bind(&mut form, Some(&datasets)).unwrap();
        form
    }

    fn fired(data: &str, name: &str, value: &str) -> (Report, Form) {
        let mut form = validated(data);
        let mut runner = Runner::new(&mut form);
        let path = DataPath::parse(&format!("form1[0]/S[0]/{name}[0]")).unwrap();
        let report = runner.fire(&path, value);
        drop(runner);
        (report, form)
    }

    #[test]
    fn message_boxes_are_recorded_bounded_and_answer_ok() {
        let (report, form) = fired("<Box>x</Box>", "Box", "x");
        assert_eq!(report.messages.len(), MAX_MESSAGES);
        assert_eq!(report.messages[0], "note 0");
        assert_eq!(report.messages[15], "note 15");
        // The 21st call found no room, and still answered OK.
        assert_eq!(value_of(&form, "Box"), "ok");
        let mut form = validated("");
        let mut runner = Runner::new(&mut form);
        runner.record_message(&"\u{e9}".repeat(600));
        runner.record_message("   ");
        let taken = runner.take_messages();
        assert_eq!(taken.len(), 1, "a blank message is dropped");
        assert_eq!(
            taken[0].len(),
            MAX_MESSAGE_BYTES,
            "cut at a character boundary"
        );
        assert!(runner.take_messages().is_empty());
        let (report, _) = fired("<Fc>1</Fc>", "Fc", "1");
        assert_eq!(report.messages, ["from FormCalc"]);
    }

    #[test]
    fn failed_validations_record_their_messages_and_keep_the_value() {
        let (report, form) = fired("<Positive>-3</Positive>", "Positive", "-3");
        assert_eq!(report.messages, ["Enter a positive amount."]);
        assert_eq!(value_of(&form, "Positive"), "-3", "kept");
        let (report, _) = fired("<Positive>3</Positive>", "Positive", "3");
        assert!(report.messages.is_empty());

        let (report, _) = fired("<Required/>", "Required", "");
        assert_eq!(report.messages, ["This one is required."]);
        let (report, _) = fired("<Required>x</Required>", "Required", "x");
        assert!(report.messages.is_empty());
        let (report, _) = fired("<Warned/>", "Warned", "");
        assert_eq!(
            report.messages,
            ["Warned is invalid"],
            "the default message"
        );

        let (report, _) = fired("<Digits>12a</Digits>", "Digits", "12a");
        assert_eq!(report.messages, ["Three digits, please."]);
        let (report, _) = fired("<Digits>123</Digits>", "Digits", "123");
        assert!(report.messages.is_empty());
        let (report, _) = fired("<Digits/>", "Digits", "");
        assert!(
            report.messages.is_empty(),
            "an empty value is the null test's"
        );
        let (report, _) = fired("<Lenient>12a</Lenient>", "Lenient", "12a");
        assert!(report.messages.is_empty(), "a disabled test");
    }
}
