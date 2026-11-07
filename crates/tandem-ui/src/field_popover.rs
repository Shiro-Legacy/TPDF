//! The Form tool's popover: the picked field's settings beside it on the page. Its inputs
//! are the side panel's `field-prop-*` elements (the core fills them for the picked field);
//! each change is applied at once, one undo step, as the panel's Apply would.

use crate::store;
use leptos::prelude::*;
use tandem_protocol::ui::{FieldPopover, NodeView, UiAction};

const POP_W: f64 = 272.0;
const POP_GAP: f64 = 10.0;

fn node(id: &str) -> NodeView {
    store()
        .state
        .with(|s| s.view.panel_nodes.get(id).cloned().unwrap_or_default())
}

/// Sets the input in the core, then applies the field's settings.
fn apply_value(id: &'static str, value: String) {
    store().act(UiAction::Input {
        id: id.into(),
        value,
    });
    store().act(UiAction::Click {
        id: "field-prop-apply".into(),
    });
}

fn apply_check(id: &'static str, checked: bool) {
    store().act(UiAction::Check {
        id: id.into(),
        checked,
    });
    store().act(UiAction::Click {
        id: "field-prop-apply".into(),
    });
}

/// Right of the field, else left of it, else under it (CSS px on the page). A checkbox or
/// choice button usually has its words to its right: the popover leaves room for them.
fn place(rect: &[f64; 4], s: f64, page_w: f64, small: bool) -> (f64, f64) {
    let words = if small { 120.0 * s } else { 0.0 };
    let right = rect[2] * s + POP_GAP + words;
    let left = rect[0] * s - POP_GAP - POP_W;
    if right + POP_W <= page_w {
        (right, rect[1] * s)
    } else if left >= 0.0 {
        (left, rect[1] * s)
    } else {
        (
            (rect[0] * s).min(page_w - POP_W).max(0.0),
            rect[3] * s + POP_GAP,
        )
    }
}

#[component]
fn Text(
    id: &'static str,
    label: &'static str,
    #[prop(optional)] kind: &'static str,
) -> impl IntoView {
    let kind = if kind.is_empty() { "text" } else { kind };
    view! {
        <label class="field">
            <span>{label}</span>
            <input class="input" type=kind autocomplete="off" spellcheck="false"
                disabled=move || node(id).disabled prop:value=move || node(id).value
                on:change=move |ev| apply_value(id, event_target_value(&ev))/>
        </label>
    }
}

#[component]
fn Check(id: &'static str, label: &'static str) -> impl IntoView {
    view! {
        <label class="check">
            <input type="checkbox" prop:checked=move || node(id).checked disabled=move || node(id).disabled
                on:change=move |ev| apply_check(id, event_target_checked(&ev))/>
            <span>{label}</span>
        </label>
    }
}

/// A drop-down's choices, one row each: change one, remove one, or add one.
#[component]
fn ChoiceList() -> impl IntoView {
    let rows = move || {
        node("field-prop-options")
            .value
            .split('\n')
            .map(str::to_string)
            .filter(|l| !l.trim().is_empty())
            .collect::<Vec<_>>()
    };
    let save = |list: Vec<String>| apply_value("field-prop-options", list.join("\n"));
    view! {
        <div class="choice-list" role="group" aria-label="Choices">
            <span class="choice-list-label">"Choices"</span>
            {move || {
                let list = rows();
                let count = list.len();
                list.into_iter().enumerate().map(|(i, text)| view! {
                    <div class="choice-row">
                        <input class="input" aria-label=format!("Choice {}", i + 1) prop:value=text
                            on:change=move |ev| {
                                let mut list = rows();
                                let value = event_target_value(&ev);
                                if i < list.len() {
                                    list[i] = value;
                                }
                                save(list);
                            }/>
                        <button class="icon-btn sm" aria-label="Remove this choice" title="Remove this choice"
                            disabled=count <= 1
                            on:click=move |_| {
                                let mut list = rows();
                                if i < list.len() && list.len() > 1 {
                                    list.remove(i);
                                }
                                save(list);
                            }>
                            <crate::icons::Icon name="close" class="sm"/>
                        </button>
                    </div>
                }).collect_view()
            }}
            <button class="btn quiet" on:click=move |_| {
                let mut list = rows();
                let n = (list.len() + 1..).find(|n| !list.contains(&format!("Choice {n}"))).unwrap_or(1);
                list.push(format!("Choice {n}"));
                save(list);
            }>"+ Add choice"</button>
        </div>
    }
}

/// The picked field's settings beside it.
#[component]
pub fn Popover(pop: FieldPopover, scale: f64, page_w: f64) -> impl IntoView {
    let store = store();
    let small = matches!(pop.kind.as_str(), "checkbox" | "radio");
    let (x, y) = place(&pop.rect, scale, page_w, small);
    let kind = pop.kind.clone();
    let title = match kind.as_str() {
        "text" => "Text field",
        "checkbox" => "Checkbox",
        "radio" => "Choice buttons",
        "combo" => "Drop-down list",
        "list" => "List",
        "signature" => "Signature field",
        _ => "Field",
    };
    let text = kind == "text";
    let combo = kind == "combo";
    let radio = kind == "radio";
    let checkbox = kind == "checkbox";
    let signature = kind == "signature";
    let buttons = pop.buttons.join(", ");
    let esc = move || {
        store.act(UiAction::Key {
            key: "Escape".into(),
            shift: false,
            alt: false,
            meta: false,
        })
    };
    view! {
        <div class="field-popover" role="dialog" aria-label=title
            style=format!("left:{x}px;top:{y}px")
            on:keydown=move |ev: web_sys::KeyboardEvent| {
                let chord = ev.meta_key() || ev.ctrl_key() || ev.alt_key();
                if chord {
                    return;
                }
                // Keys typed here are the popover's (Delete must not delete the field).
                ev.stop_propagation();
                if ev.key() == "Escape" {
                    ev.prevent_default();
                    esc();
                }
            }
            on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
            on:contextmenu=|ev: web_sys::MouseEvent| ev.stop_propagation()>
            <header>
                <strong>{title}</strong>
                <span class="spacer"></span>
                <button class="icon-btn sm" aria-label="Close" title="Close (Esc)" on:click=move |_| esc()>
                    <crate::icons::Icon name="close" class="sm"/>
                </button>
            </header>
            <Text id="field-prop-name" label="Name"/>
            {text.then(|| view! {
                <Check id="field-prop-multiline" label="Several lines"/>
                <Text id="field-prop-maxlen" label="Most characters (0 for no limit)" kind="number"/>
            })}
            {combo.then(|| view! { <ChoiceList/> })}
            {radio.then(|| view! {
                <p class="field-popover-note">{format!("Choices: {buttons}")}<br/>
                    <small>"With Choice buttons picked, click the page to add another."</small></p>
            })}
            {(!signature).then(|| view! {
                <Check id="field-prop-required" label="Must be filled in"/>
                <Check id="field-prop-readonly" label="Cannot be changed"/>
            })}
            {checkbox.then(|| view! { <Check id="field-prop-default-check" label="Starts checked"/> })}
            {(!checkbox && !signature).then(|| view! { <Text id="field-prop-default" label="Starts with"/> })}
            <Text id="field-prop-label" label="Description for screen readers"/>
            <details class="field-popover-position">
                <summary>"Exact position"</summary>
                <div class="grid2">
                    <Text id="field-prop-x" label="From the left" kind="number"/>
                    <Text id="field-prop-y" label="From the top" kind="number"/>
                    <Text id="field-prop-w" label="Width" kind="number"/>
                    <Text id="field-prop-h" label="Height" kind="number"/>
                </div>
                <small>"In points (72 to an inch)."</small>
            </details>
            <footer>
                <button class="btn quiet field-delete" disabled=move || node("field-prop-delete").disabled
                    on:click=move |_| store.act(UiAction::Click { id: "field-prop-delete".into() })>"Delete field"</button>
            </footer>
        </div>
    }
}
