//! The Form tool's question list (design form, phase 2): one card per question, in order;
//! the open card edits its question, type, choices, help and required flag; a grip drags a
//! card to another place (Alt+↑/↓ from the keyboard). Every change goes to the core as a
//! `form-…` edit, which lays the page out again in one undo step.

use crate::icons::Icon;
use crate::{select, store};
use leptos::prelude::*;
use serde_json::{Value, json};
use tandem_protocol::ui::UiAction;
use wasm_bindgen::JsCast;

/// The types a card offers, in the menu's order.
const TYPES: [(&str, &str); 15] = [
    ("short", "Short answer"),
    ("paragraph", "Paragraph"),
    ("one", "One choice"),
    ("several", "Several choices"),
    ("dropdown", "Drop-down"),
    ("yesno", "Yes / No"),
    ("date", "Date"),
    ("number", "Number"),
    ("email", "Email"),
    ("phone", "Phone"),
    ("agree", "I agree (checkbox)"),
    ("signature", "Signature"),
    ("rating", "Rating"),
    ("likert", "Likert grid"),
    ("section", "Section"),
];

/// The type buttons under "Add question", Microsoft Forms style: the common ones, the rest
/// behind More.
const QUICK: [&str; 8] = [
    "short",
    "paragraph",
    "one",
    "several",
    "dropdown",
    "yesno",
    "date",
    "rating",
];
const MORE: [&str; 7] = [
    "number",
    "email",
    "phone",
    "likert",
    "agree",
    "signature",
    "section",
];

const PRESETS: [(&str, &str); 6] = [
    ("yes-no", "Yes / No"),
    ("agree", "Agree scale"),
    ("1-5", "1–5"),
    ("1-10", "1–10"),
    ("0-10", "0–10"),
    ("satisfaction", "Satisfaction"),
];

fn type_label(kind: &str) -> &'static str {
    TYPES
        .iter()
        .find(|(k, _)| *k == kind)
        .map_or("Field", |(_, l)| l)
}

fn locale() -> String {
    web_sys::window()
        .and_then(|w| w.navigator().language())
        .unwrap_or_default()
}

fn edit(op: &str, mut contents: Value) {
    if let Some(map) = contents.as_object_mut() {
        map.insert("locale".into(), json!(locale()));
    }
    store().act(UiAction::Edit {
        op: op.into(),
        contents: Some(contents),
    });
}

fn keys() -> Vec<String> {
    store().state.with(|s| {
        s.view
            .form
            .as_ref()
            .map(|f| f.questions.iter().map(|q| q.key.clone()).collect())
            .unwrap_or_default()
    })
}

/// The cards that are questions (the ones a drag can reorder), in order.
fn question_keys() -> Vec<String> {
    store().state.with(|s| {
        s.view
            .form
            .as_ref()
            .map(|f| {
                f.questions
                    .iter()
                    .filter(|q| !q.unlisted)
                    .map(|q| q.key.clone())
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn move_to(key: &str, to: usize) {
    edit("form-move", json!({"key": key, "to": to}));
}

/// The gap (0..=n) a pointer at `y` is over, from the question cards' boxes.
fn gap_at(list: &web_sys::Element, y: f64) -> Option<usize> {
    let cards = list.query_selector_all(".q-card[data-question]").ok()?;
    let mut gap = 0;
    for i in 0..cards.length() {
        let el = cards.item(i)?.dyn_into::<web_sys::Element>().ok()?;
        let r = el.get_bounding_client_rect();
        if y > r.top() + r.height() / 2.0 {
            gap = i as usize + 1;
        }
    }
    Some(gap)
}

/// The list's drag: the card lifted (its key and place) and the gap it would drop in.
#[derive(Clone, Copy)]
struct Drag {
    lifted: RwSignal<Option<(String, usize)>>,
    gap: RwSignal<Option<usize>>,
}

#[component]
pub fn QuestionList() -> impl IntoView {
    let title = select(|s| {
        s.view
            .form
            .as_ref()
            .map(|f| f.title.clone())
            .unwrap_or_default()
    });
    let desc = select(|s| {
        s.view
            .form
            .as_ref()
            .map(|f| f.desc.clone())
            .unwrap_or_default()
    });
    let title_moved = select(|s| s.view.form.as_ref().is_some_and(|f| f.title_moved));
    let note = select(|s| {
        s.view
            .form
            .as_ref()
            .map(|f| (f.where_note.clone(), f.blocked.clone()))
            .unwrap_or_default()
    });
    let list_keys = select(|s| {
        s.view
            .form
            .as_ref()
            .map(|f| {
                f.questions
                    .iter()
                    .map(|q| q.key.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    let any = move || !list_keys.get().is_empty();
    let drag = Drag {
        lifted: RwSignal::new(None),
        gap: RwSignal::new(None),
    };
    provide_context(drag);
    let list_ref = NodeRef::<leptos::html::Ol>::new();
    let end_drag = move || {
        drag.lifted.set(None);
        drag.gap.set(None);
    };
    view! {
        <section class="questions" aria-label="Questions">
            {move || {
                let (where_note, blocked) = note.get();
                if !blocked.is_empty() {
                    view! { <p class="notice warning">{blocked}</p> }.into_any()
                } else if where_note.is_empty() {
                    view! { <p class="hint">"Type your questions here and Tandem lays them out on the page. Or draw fields straight on the page with the tools above."</p> }.into_any()
                } else {
                    view! { <p class="hint">{where_note}" Click a question's words on the page to change it, or drag them to move it."</p> }.into_any()
                }
            }}
            <label class="field">
                <span>"Form title"</span>
                <input class="input" placeholder="Untitled form" prop:value=move || title.get()
                    on:change=move |ev| edit("form-title", json!({"title": event_target_value(&ev)}))/>
            </label>
            <label class="field">
                <span>"Description"</span>
                <textarea class="input" rows="2" placeholder="What this form is for (optional)" prop:value=move || desc.get()
                    on:change=move |ev| edit("form-title", json!({"desc": event_target_value(&ev)}))></textarea>
            </label>
            {move || title_moved.get().then(|| view! {
                <p class="hint">"The title was moved on the page. "
                    <button class="btn quiet" on:click=move |_| edit("form-title-back", json!({}))>"Put it back at the top"</button></p>
            })}
            <ol class="q-list" node_ref=list_ref class:dragging=move || drag.lifted.get().is_some()
                on:pointermove=move |ev: web_sys::PointerEvent| {
                    if drag.lifted.get_untracked().is_none() { return; }
                    if ev.buttons() & 1 == 0 { end_drag(); return; }
                    ev.prevent_default();
                    let Some(list) = list_ref.get_untracked() else { return };
                    let list: &web_sys::Element = &list;
                    drag.gap.set(gap_at(list, f64::from(ev.client_y())));
                    // Near the panel's edges the panel scrolls, so a far place can be reached.
                    if let Some(scroller) = list.closest(".scroll").ok().flatten() {
                        let r = scroller.get_bounding_client_rect();
                        let y = f64::from(ev.client_y());
                        if y < r.top() + 40.0 { scroller.scroll_by_with_x_and_y(0.0, -16.0); }
                        else if y > r.bottom() - 40.0 { scroller.scroll_by_with_x_and_y(0.0, 16.0); }
                    }
                }
                on:pointerup=move |_| {
                    let lifted = drag.lifted.get_untracked();
                    let gap = drag.gap.get_untracked();
                    end_drag();
                    if let (Some((key, from)), Some(gap)) = (lifted, gap) {
                        let to = if gap > from { gap - 1 } else { gap };
                        if to != from { move_to(&key, to); }
                    }
                }
                on:pointercancel=move |_| end_drag()
                on:lostpointercapture=move |_| end_drag()>
                <For each=move || list_keys.get() key=|k| k.clone() children=move |key: String| view! { <Card key=key/> }/>
            </ol>
            <div class="q-add">
                <button class="btn primary" disabled=move || !note.get().1.is_empty()
                    title="A new question; its type follows what you type"
                    on:click=move |_| edit("form-add-question", json!({"key": store().state.with(|s| s.view.form.as_ref().and_then(|f| f.open.clone()))}))>
                    <Icon name="plus" class="sm"/>"Add question"
                </button>
                <div class="q-types" role="group" aria-label="Add a question of this type">
                    {QUICK.iter().map(|kind| view! {
                        <button class="q-chip" disabled=move || !note.get().1.is_empty()
                            on:click=move |_| edit("form-add-question", json!({"type": kind, "key": store().state.with(|s| s.view.form.as_ref().and_then(|f| f.open.clone()))}))>{type_label(kind)}</button>
                    }).collect_view()}
                    <details class="q-more">
                        <summary class="q-chip">"More"</summary>
                        <div class="q-more-list">
                            {MORE.iter().map(|kind| view! {
                                <button class="q-chip" disabled=move || !note.get().1.is_empty()
                                    on:click=move |_| edit("form-add-question", json!({"type": kind, "key": store().state.with(|s| s.view.form.as_ref().and_then(|f| f.open.clone()))}))>{type_label(kind)}</button>
                            }).collect_view()}
                        </div>
                    </details>
                </div>
            </div>
            {move || any().then(|| view! {
                <div class="row wrap q-bulk">
                    <button class="btn quiet" on:click=move |_| edit("form-required-all", json!({"on": true}))>"Make all required"</button>
                    <button class="btn quiet" on:click=move |_| edit("form-required-all", json!({"on": false}))>"Make none required"</button>
                </div>
            })}
        </section>
    }
}

/// One card: closed, its number, question and type; open, everything about it.
#[component]
fn Card(key: String) -> impl IntoView {
    let drag = expect_context::<Drag>();
    let k = key.clone();
    let open =
        select(move |s| s.view.form.as_ref().and_then(|f| f.open.clone()) == Some(k.clone()));
    let k = key.clone();
    let unlisted = select(move |s| {
        s.view
            .form
            .as_ref()
            .and_then(|f| f.questions.iter().find(|q| q.key == k))
            .is_some_and(|q| q.unlisted)
    });
    let k = key.clone();
    let lifted = move || drag.lifted.get().is_some_and(|(l, _)| l == k);
    let k = key.clone();
    let drop_before = move || {
        let place = question_keys().iter().position(|q| *q == k);
        drag.gap.get().is_some() && drag.gap.get() == place && drag.lifted.get().is_some()
    };
    let k = key.clone();
    let drop_after = move || {
        let keys = question_keys();
        let place = keys.iter().position(|q| *q == k);
        drag.lifted.get().is_some()
            && place.is_some_and(|p| p + 1 == keys.len())
            && drag.gap.get() == Some(keys.len())
    };
    let k = key.clone();
    view! {
        <li class="q-card" class:open=move || open.get() class:lifted=lifted class:unlisted=move || unlisted.get()
            class:drop-before=drop_before class:drop-after=drop_after
            data-question=move || (!unlisted.get()).then_some("1")>
            {move || {
                let key = k.clone();
                if open.get() { view! { <OpenCard key=key/> }.into_any() } else { view! { <ClosedCard key=key/> }.into_any() }
            }}
        </li>
    }
}

/// The grip: a press lifts the card; Alt+↑/↓ moves it one place.
#[component]
fn Grip(key: String) -> impl IntoView {
    let drag = expect_context::<Drag>();
    let k = key.clone();
    let k2 = key.clone();
    view! {
        <button class="q-grip" aria-label="Drag to move this question (or Alt+arrow keys)" title="Drag to move (Alt+↑ / Alt+↓)"
            on:pointerdown=move |ev: web_sys::PointerEvent| {
                if ev.button() != 0 { return; }
                let keys = question_keys();
                let Some(from) = keys.iter().position(|q| *q == k) else { return };
                ev.prevent_default();
                drag.lifted.set(Some((k.clone(), from)));
                drag.gap.set(Some(from));
                // The list hears the moves and the release.
                let list = ev
                    .current_target()
                    .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                    .and_then(|el| el.closest(".q-list").ok().flatten());
                if let Some(list) = list { let _ = list.set_pointer_capture(ev.pointer_id()); }
            }
            on:keydown=move |ev: web_sys::KeyboardEvent| {
                if !ev.alt_key() { return; }
                let keys = question_keys();
                let Some(from) = keys.iter().position(|q| *q == k2) else { return };
                match ev.key().as_str() {
                    "ArrowUp" if from > 0 => { ev.prevent_default(); move_to(&k2, from - 1); }
                    "ArrowDown" if from + 1 < keys.len() => { ev.prevent_default(); move_to(&k2, from + 1); }
                    _ => {}
                }
            }>
            "⠿"
        </button>
    }
}

#[component]
fn ClosedCard(key: String) -> impl IntoView {
    // The card follows its question (a move renumbers it, an edit retitles it).
    let k = key.clone();
    let shown = select(move |s| {
        s.view
            .form
            .as_ref()
            .and_then(|f| f.questions.iter().find(|q| q.key == k).cloned())
            .unwrap_or_default()
    });
    move || {
        let c = shown.get();
        let title = if c.unlisted {
            if c.text.is_empty() {
                format!("{} · no question", c.name)
            } else {
                format!("{} · not in the list", c.text)
            }
        } else if c.text.is_empty() && c.kind == "section" {
            "Untitled section".to_string()
        } else if c.text.is_empty() {
            "Untitled question".to_string()
        } else {
            c.text.clone()
        };
        let number = (c.number > 0).then(|| format!("{}.", c.number));
        let k = key.clone();
        let badge = type_label(&c.kind);
        let badge_title = if c.guessed && c.kind != "short" && !c.text.is_empty() {
            "Guessed from the question: open the card to pick another type"
        } else {
            ""
        };
        view! {
            <div class="q-row">
                {(!c.unlisted).then(|| view! { <Grip key=key.clone()/> })}
                <button class="q-summary" class:section={c.kind == "section"} aria-expanded="false"
                    on:click=move |_| edit("form-open", json!({"key": k}))>
                    {number.map(|n| view! { <span class="q-number">{n}</span> })}
                    <span class="q-text">{title}{c.required.then_some(" *")}</span>
                    <span class="q-badge" title=badge_title>{badge}</span>
                    {(c.pinned && !c.unlisted).then(|| view! { <span class="q-badge pinned" title="Moved by hand: it keeps its place on the page">"Placed by hand"</span> })}
                </button>
            </div>
        }
    }
}

#[component]
fn OpenCard(key: String) -> impl IntoView {
    let k = key.clone();
    let text = select(move |s| {
        s.view
            .form
            .as_ref()
            .and_then(|f| f.questions.iter().find(|q| q.key == k))
            .map(|q| q.text.clone())
            .unwrap_or_default()
    });
    let k = key.clone();
    let help = select(move |s| {
        s.view
            .form
            .as_ref()
            .and_then(|f| f.questions.iter().find(|q| q.key == k))
            .map(|q| q.help.clone())
            .unwrap_or_default()
    });
    let k = key.clone();
    let shape = select(move |s| {
        s.view
            .form
            .as_ref()
            .and_then(|f| f.questions.iter().find(|q| q.key == k))
            .map(|q| {
                (
                    q.kind.clone(),
                    q.guessed,
                    q.required,
                    q.pinned,
                    q.foreign,
                    q.unlisted,
                    q.number,
                )
            })
            .unwrap_or_default()
    });
    let k = key.clone();
    let extra = select(move |s| {
        s.view
            .form
            .as_ref()
            .and_then(|f| f.questions.iter().find(|q| q.key == k))
            .map(|q| (q.other, q.steps, q.stars, q.new_page))
            .unwrap_or_default()
    });
    let k = key.clone();
    let send = move |change: Value| {
        let mut change = change;
        change["key"] = json!(k);
        edit("form-question", change);
    };
    let text_ref = NodeRef::<leptos::html::Textarea>::new();
    // A card opened by "Add question" starts with its question box ready to type in.
    Effect::new(move |_| {
        if let Some(el) = text_ref.get()
            && text.get_untracked().is_empty()
        {
            let _ = el.focus();
        }
    });
    let k = key.clone();
    let next = move || {
        // Return: the next card, or a new question after this one.
        let all = keys();
        let at = all.iter().position(|q| *q == k);
        match at.and_then(|i| all.get(i + 1)) {
            Some(next) if !next.starts_with('u') => edit("form-open", json!({"key": next})),
            _ => edit("form-add-question", json!({"key": k})),
        }
    };
    let k_add = key.clone();
    let k_move = key.clone();
    let k_unpin = key.clone();
    view! {
        <div class="q-open" role="group" aria-label=move || format!("Question {}", shape.get().6)>
            <div class="q-row">
                {let k = key.clone(); move || (!shape.get().5).then(|| view! { <Grip key=k.clone()/> })}
                <strong class="q-number">{move || { let n = shape.get().6; if n > 0 { format!("Question {n}") } else { "Field on the page".to_string() } }}</strong>
                <span class="spacer"></span>
                <button class="icon-btn sm" aria-label="Close this question" title="Close"
                    on:click=move |_| edit("form-open", json!({"key": Value::Null}))>
                    <Icon name="close" class="sm"/>
                </button>
            </div>
            <label class="field">
                <span>{move || if shape.get().0 == "section" { "Section heading" } else { "Question" }}</span>
                <textarea class="input q-question" rows="2" node_ref=text_ref
                    placeholder=move || if shape.get().0 == "section" { "Like “About you”" } else { "Type a question, like “What is your email address?”" }
                    prop:value=move || text.get()
                    on:change={let send = send.clone(); move |ev| send(json!({"text": event_target_value(&ev)}))}
                    on:keydown={let send = send.clone(); let next = next.clone(); move |ev: web_sys::KeyboardEvent| {
                        if ev.key() == "Enter" && !ev.shift_key() {
                            ev.prevent_default();
                            let value = ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlTextAreaElement>().ok()).map(|t| t.value());
                            if let Some(value) = value.filter(|v| *v != text.get_untracked()) { send(json!({"text": value})); }
                            if ev.meta_key() || ev.ctrl_key() {
                                edit("form-add-question", json!({"key": k_add.clone()}));
                            } else {
                                next();
                            }
                        }
                    }}></textarea>
            </label>
            <label class="field">
                <span>{move || if shape.get().1 && shape.get().0 != "short" && !text.get().is_empty() { "Answer type (guessed from the question)" } else { "Answer type" }}</span>
                <select class="input" disabled=move || { let s = shape.get(); s.4 || s.5 }
                    on:change={let send = send.clone(); move |ev| send(json!({"type": event_target_value(&ev)}))}>
                    {move || {
                        let kind = shape.get().0;
                        TYPES.iter().map(|(v, l)| view! { <option value=*v selected=kind == *v>{*l}</option> }).collect_view()
                    }}
                </select>
            </label>
            {let send = send.clone(); move || {
                let (kind, ..) = shape.get();
                let send = send.clone();
                match kind.as_str() {
                    "one" | "several" => Some(view! {
                        <Choices key=key.clone() rows=false/>
                        <label class="check">
                            <input type="checkbox" prop:checked=move || extra.get().0
                                on:change=move |ev| send(json!({"other": event_target_checked(&ev)}))/>
                            <span>"Add “Other” with a box to type in"</span>
                        </label>
                    }.into_any()),
                    "dropdown" => Some(view! { <Choices key=key.clone() rows=false/> }.into_any()),
                    "likert" => Some(view! {
                        <Choices key=key.clone() rows=true/>
                        <Choices key=key.clone() rows=false/>
                    }.into_any()),
                    "rating" => {
                        let send_steps = send.clone();
                        Some(view! {
                            <div class="grid2 q-rating">
                                <label class="field">
                                    <span>"Shown as"</span>
                                    <select class="input" on:change=move |ev| send(json!({"style": event_target_value(&ev)}))>
                                        <option value="stars" selected=move || extra.get().2>"Stars"</option>
                                        <option value="numbers" selected=move || !extra.get().2>"Numbers"</option>
                                    </select>
                                </label>
                                <label class="field">
                                    <span>"Steps"</span>
                                    <select class="input" on:change=move |ev| {
                                        if let Ok(n) = event_target_value(&ev).parse::<u64>() { send_steps(json!({"steps": n})); }
                                    }>
                                        {(3..=10usize).map(|n| view! { <option value=n.to_string() selected=move || extra.get().1 == n>{n.to_string()}</option> }).collect_view()}
                                    </select>
                                </label>
                            </div>
                        }.into_any())
                    }
                    _ => None,
                }
            }}
            <label class="field">
                <span>{move || if shape.get().0 == "section" { "Description" } else { "Help text" }}</span>
                <input class="input" prop:value=move || help.get()
                    placeholder=move || if shape.get().0 == "section" { "Under the heading (optional)" } else { "Shown under the question (optional)" }
                    disabled=move || { let s = shape.get(); s.4 || s.5 }
                    on:change={let send = send.clone(); move |ev| send(json!({"help": event_target_value(&ev)}))}/>
            </label>
            {let send = send.clone(); move || (shape.get().0 == "section").then(|| {
                let send = send.clone();
                view! {
                    <label class="check">
                        <input type="checkbox" prop:checked=move || extra.get().3
                            on:change=move |ev| send(json!({"newPage": event_target_checked(&ev)}))/>
                        <span>"Start on a new page"</span>
                    </label>
                }
            })}
            {move || (!matches!(shape.get().0.as_str(), "signature" | "section")).then(|| {
                let send = send.clone();
                view! {
                    <label class="check">
                        <input type="checkbox" prop:checked=move || shape.get().2
                            on:change=move |ev| send(json!({"required": event_target_checked(&ev)}))/>
                        <span>"Must be answered"</span>
                    </label>
                }
            })}
            {move || {
                let (_, _, _, pinned, foreign, unlisted, _) = shape.get();
                if unlisted {
                    Some(view! { <p class="hint">"Drawn on the page. Type a question to add it to the list; it keeps its place."</p> }.into_any())
                } else if foreign {
                    Some(view! { <p class="hint">"On a page made elsewhere: the question becomes the field's description, and the page is not changed."</p> }.into_any())
                } else if pinned {
                    let k = k_unpin.clone();
                    Some(view! {
                        <p class="hint">"Moved by hand: it keeps its place and the other questions flow around it. "
                            <button class="btn quiet" on:click=move |_| edit("form-unpin", json!({"key": k}))>"Put back in line"</button></p>
                    }.into_any())
                } else {
                    None
                }
            }}
            {move || (!shape.get().5).then(|| {
                let (k_up, k_down, k_copy, k_del) = (k_move.clone(), k_move.clone(), k_move.clone(), k_move.clone());
                view! {
                    <footer class="row wrap q-actions">
                        <button class="btn quiet" title="Move up (Alt+↑ on the grip)"
                            disabled={let k = k_up.clone(); move || question_keys().first() == Some(&k)}
                            on:click=move |_| {
                                if let Some(i) = question_keys().iter().position(|q| *q == k_up) && i > 0 { move_to(&k_up, i - 1); }
                            }>"Move up"</button>
                        <button class="btn quiet" title="Move down (Alt+↓ on the grip)"
                            disabled={let k = k_down.clone(); move || question_keys().last() == Some(&k)}
                            on:click=move |_| {
                                if let Some(i) = question_keys().iter().position(|q| *q == k_down) { move_to(&k_down, i + 1); }
                            }>"Move down"</button>
                        <button class="btn quiet" on:click=move |_| edit("form-duplicate", json!({"key": k_copy}))>"Duplicate"</button>
                        <span class="spacer"></span>
                        <button class="btn quiet q-delete" on:click=move |_| edit("form-delete", json!({"key": k_del}))>"Delete"</button>
                    </footer>
                }
            })}
        </div>
    }
}

/// A choice question's choices (`rows`: a grid's statements): one row each, drag to
/// reorder, paste several lines to add several, and one-click sets.
#[component]
fn Choices(key: String, rows: bool) -> impl IntoView {
    let k = key.clone();
    let grid = select(move |s| {
        s.view
            .form
            .as_ref()
            .and_then(|f| f.questions.iter().find(|q| q.key == k))
            .is_some_and(|q| q.kind == "likert")
    });
    let k = key.clone();
    let field = if rows { "rows" } else { "choices" };
    let word = if rows { "Statement" } else { "Choice" };
    let least = if rows { 1 } else { 2 };
    let list_of = select(move |s| {
        s.view
            .form
            .as_ref()
            .and_then(|f| f.questions.iter().find(|q| q.key == k))
            .map(|q| {
                if rows {
                    q.rows.clone()
                } else {
                    q.choices.clone()
                }
            })
            .unwrap_or_default()
    });
    let rows_sig = list_of;
    let k = key.clone();
    let save = move |list: Vec<String>| edit("form-question", json!({"key": k, field: list}));
    let save_add = save.clone();
    let k_preset = key.clone();
    let lifted = RwSignal::new(None::<usize>);
    let over = RwSignal::new(None::<usize>);
    let title = move || match (rows, grid.get()) {
        (true, _) => "Statements (one row each)",
        (false, true) => "Scale (the columns)",
        (false, false) => "Choices",
    };
    view! {
        <div class="choice-list" role="group" aria-label=title>
            <span class="choice-list-label">{title}</span>
            {move || {
                let rows = rows_sig;
                let list = rows.get();
                let count = list.len();
                list.into_iter().enumerate().map(|(i, text)| {
                    let save = save.clone();
                    let save_paste = save.clone();
                    let save_drop = save.clone();
                    let save_remove = save.clone();
                    view! {
                        <div class="choice-row" class:drop-before=move || over.get() == Some(i) && lifted.get().is_some_and(|l| l != i)
                            on:pointerenter=move |_| if lifted.get_untracked().is_some() { over.set(Some(i)); }
                            on:pointerup=move |_| {
                                if let (Some(from), Some(to)) = (lifted.get_untracked(), over.get_untracked()) {
                                    lifted.set(None); over.set(None);
                                    if from != to {
                                        let mut list = rows_sig.get_untracked();
                                        let item = list.remove(from);
                                        list.insert(to.min(list.len()), item);
                                        save_drop(list);
                                    }
                                }
                            }>
                            <span class="q-grip sm" aria-hidden="true" title="Drag to move"
                                on:pointerdown=move |ev: web_sys::PointerEvent| { ev.prevent_default(); lifted.set(Some(i)); over.set(Some(i)); }>"⠿"</span>
                            <input class="input" aria-label=format!("{word} {}", i + 1) prop:value=text
                                on:change=move |ev| {
                                    let mut list = rows.get_untracked();
                                    let value = event_target_value(&ev);
                                    if i < list.len() { list[i] = value; }
                                    save(list);
                                }
                                on:paste=move |ev: web_sys::ClipboardEvent| {
                                    // Several lines pasted: one choice each.
                                    let pasted = ev.clipboard_data().and_then(|d| d.get_data("text").ok()).unwrap_or_default();
                                    if !pasted.contains('\n') { return; }
                                    ev.prevent_default();
                                    let mut list = rows.get_untracked();
                                    let lines: Vec<String> = pasted.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
                                    if i < list.len() { list.splice(i..=i, lines); } else { list.extend(lines); }
                                    save_paste(list);
                                }/>
                            <button class="icon-btn sm" aria-label=format!("Remove this {}", word.to_lowercase()) title=format!("Remove this {}", word.to_lowercase())
                                disabled=count <= least
                                on:click=move |_| {
                                    let mut list = rows.get_untracked();
                                    if i < list.len() && list.len() > least { list.remove(i); }
                                    save_remove(list);
                                }>
                                <Icon name="close" class="sm"/>
                            </button>
                        </div>
                    }
                }).collect_view()
            }}
            <div class="row wrap">
                <button class="btn quiet" on:click={let save = save_add.clone(); move |_| {
                    let mut list = rows_sig.get_untracked();
                    let n = (list.len() + 1..).find(|n| !list.contains(&format!("{word} {n}"))).unwrap_or(1);
                    list.push(format!("{word} {n}"));
                    save(list);
                }}>{format!("+ Add {}", word.to_lowercase())}</button>
            </div>
            {(!rows).then(|| view! {
                <div class="q-presets" role="group" aria-label="Fill in a set of choices">
                    <span class="choice-list-label">"Quick sets:"</span>
                    {PRESETS.iter().map(|(preset, label)| {
                        let k = k_preset.clone();
                        view! { <button class="q-chip" on:click=move |_| edit("form-preset", json!({"key": k, "preset": preset}))>{*label}</button> }
                    }).collect_view()}
                </div>
            })}
        </div>
    }
}
