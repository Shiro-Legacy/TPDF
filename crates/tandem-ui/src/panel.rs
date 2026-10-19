//! The side panel (§3.2: "the right panel shows what is relevant"). One panel per
//! purpose — comments, form fields, links, the selection, layers, redactions — each
//! a view of the ported inspector panel (elements by id in `View::panel_nodes`).
//! Wording is new and plain (X7): no "baseline point", no PostScript font names.

use crate::icons::Icon;
use crate::store;
use leptos::prelude::*;
use tandem_protocol::ui::{NodeView, Row, UiAction};

fn node(id: &str) -> NodeView {
    store()
        .state
        .with(|s| s.view.panel_nodes.get(id).cloned().unwrap_or_default())
}

fn list(name: &'static str) -> Vec<Row> {
    store()
        .state
        .with(|s| s.view.lists.get(name).cloned().unwrap_or_default())
}

fn click(id: &'static str) -> impl Fn(leptos::ev::MouseEvent) + Copy {
    move |_| store().act(UiAction::Click { id: id.into() })
}

#[component]
fn PButton(
    id: &'static str,
    label: &'static str,
    #[prop(optional)] class: &'static str,
    #[prop(optional)] title: &'static str,
) -> impl IntoView {
    let class = if class.is_empty() {
        "btn".to_string()
    } else {
        format!("btn {class}")
    };
    view! {
        <button class=class title=title hidden=move || node(id).hidden disabled=move || node(id).disabled on:click=click(id)>{label}</button>
    }
}

#[component]
fn PField(
    id: &'static str,
    label: &'static str,
    #[prop(optional)] kind: &'static str,
    #[prop(optional)] hint: &'static str,
    #[prop(optional)] placeholder: &'static str,
) -> impl IntoView {
    let kind = if kind.is_empty() { "text" } else { kind };
    view! {
        <label class="field" class:hidden=move || node(id).hidden>
            <span>{label}</span>
            <input class="input" type=kind placeholder=placeholder autocomplete="off" spellcheck="false"
                disabled=move || node(id).disabled prop:value=move || node(id).value
                on:change=move |ev| store().act(UiAction::Input { id: id.into(), value: event_target_value(&ev) })/>
            {(!hint.is_empty()).then(|| view! { <small>{hint}</small> })}
        </label>
    }
}

#[component]
fn PArea(
    id: &'static str,
    label: &'static str,
    #[prop(optional)] hint: &'static str,
) -> impl IntoView {
    view! {
        <label class="field" class:hidden=move || node(id).hidden>
            <span>{label}</span>
            <textarea class="input" rows="3" prop:value=move || node(id).value
                on:change=move |ev| store().act(UiAction::Input { id: id.into(), value: event_target_value(&ev) })></textarea>
            {(!hint.is_empty()).then(|| view! { <small>{hint}</small> })}
        </label>
    }
}

#[component]
fn PSelect(
    id: &'static str,
    label: &'static str,
    options: &'static [(&'static str, &'static str)],
) -> impl IntoView {
    view! {
        <label class="field" class:hidden=move || node(id).hidden>
            <span>{label}</span>
            <select class="input" disabled=move || node(id).disabled
                on:change=move |ev| store().act(UiAction::Input { id: id.into(), value: event_target_value(&ev) })>
                {options.iter().map(|(v, l)| view! { <option value=*v selected=move || node(id).value == *v>{*l}</option> }).collect_view()}
            </select>
        </label>
    }
}

#[component]
fn PCheck(id: &'static str, label: &'static str) -> impl IntoView {
    view! {
        <label class="check" hidden=move || node(id).hidden>
            <input type="checkbox" prop:checked=move || node(id).checked disabled=move || node(id).disabled
                on:change=move |ev| store().act(UiAction::Check { id: id.into(), checked: event_target_checked(&ev) })/>
            <span>{label}</span>
        </label>
    }
}

fn status(id: &'static str) -> impl IntoView {
    move || {
        let n = node(id);
        (!n.hidden && !n.text.is_empty())
            .then(|| view! { <p class="panel-status selectable" role="status">{n.text}</p> })
    }
}

/// Rows with per-row buttons.
fn rows(
    name: &'static str,
    rows: Vec<Row>,
    empty: &'static str,
    actions: &'static [(&'static str, &'static str)],
) -> AnyView {
    if rows.is_empty() {
        return view! { <p class="empty">{empty}</p> }.into_any();
    }
    view! {
        <ul class="list">
            {rows.into_iter().map(|r| {
                let id = r.id.clone();
                let id_main = r.id.clone();
                // VoiceOver hears the details too, not only "Page 1 · box 2".
                let spoken = if r.detail.is_empty() {
                    r.title.clone()
                } else {
                    format!("{}, {}", r.title, r.detail)
                };
                // Redaction rows wrap their words instead of cutting them.
                let wrap = (name == "redactions").then_some("white-space:normal;overflow-wrap:anywhere");
                view! {
                    <li class="row-item" class:selected=r.selected>
                        <button class="list-row" aria-current=r.selected.then_some("true") aria-label=spoken
                            style=if r.selected { "background:var(--selected, rgba(42,109,244,.14));border-radius:6px" } else { "" }
                            on:click=move |_| store().act(UiAction::Row { list: name.into(), id: id_main.clone(), action: "select".into() })>
                            <strong>{r.title.clone()}</strong>
                            {(!r.detail.is_empty()).then(|| view! { <small style=wrap>{r.detail.clone()}</small> })}
                        </button>
                        <span class="row-actions">
                            {actions.iter().map(|(action, label)| {
                                let id = id.clone();
                                let title = r.title.clone();
                                view! { <button class="btn quiet" aria-label=format!("{label} {title}") title=format!("{label} {title}")
                                    on:click=move |_| store().act(UiAction::Row { list: name.into(), id: id.clone(), action: (*action).into() })>{*label}</button> }
                            }).collect_view()}
                        </span>
                    </li>
                }
            }).collect_view()}
        </ul>
    }.into_any()
}

#[component]
pub fn PanelBody() -> impl IntoView {
    let store = store();
    let tab = Memo::new(move |_| store.state.with(|s| s.view.panel.tab.clone()));
    let boxes = Memo::new(move |_| store.state.with(|s| s.view.redact.boxes.len()));
    let heading = move || match tab.get().as_str() {
        "fields" => "Form fields".to_string(),
        // "Redactions (3)": the count of boxes waiting (review redact#9).
        "redactions" => match boxes.get() {
            0 => "Redactions".to_string(),
            n => format!("Redactions ({n})"),
        },
        "properties" => "Selection".to_string(),
        "links" => "Links".to_string(),
        "layers" => "Layers".to_string(),
        _ => "Comments".to_string(),
    };
    const TABS: [(&str, &str); 5] = [
        ("comments", "Comments"),
        ("fields", "Fields"),
        ("links", "Links"),
        ("properties", "Selection"),
        ("layers", "Layers"),
    ];
    view! {
        <aside class="panel" aria-label=heading>
            <header>
                <h2>{heading}</h2>
                <button class="icon-btn" aria-label="Close the panel" title=move || store.os.get().titled("Close the panel", "⌥⌘2")
                    on:click=move |_| store.act(UiAction::Pane { which: "panel".into(), open: Some(false), tab: None })>
                    <Icon name="close" class="sm"/>
                </button>
            </header>
            <div class="scroll">
                {move || (tab.get() != "redactions").then(|| view! {
                    // One row: the labels shrink to fit, then the row scrolls (styles.css).
                    <div class="segmented panel-tabs" role="tablist" aria-label="Panel" style="height:auto">
                        {TABS.iter().map(|(id, label)| view! {
                            <button role="tab" aria-selected=move || (tab.get() == *id).to_string()
                                on:click=move |_| store.act(UiAction::Pane { which: "panel".into(), open: Some(true), tab: Some((*id).into()) })>{*label}</button>
                        }).collect_view()}
                    </div>
                })}
                {move || match tab.get().as_str() {
                    "fields" => view! { <Fields/> }.into_any(),
                    "links" => view! { <Links/> }.into_any(),
                    "properties" => view! { <Selection/> }.into_any(),
                    "layers" => view! { <Layers/> }.into_any(),
                    "redactions" => view! { <Redactions/> }.into_any(),
                    _ => view! { <Comments/> }.into_any(),
                }}
            </div>
            {move || (tab.get() == "redactions").then(|| view! { <RedactFooter/> })}
        </aside>
    }
}

#[component]
fn Comments() -> impl IntoView {
    let store = store();
    view! {
        <div>
            {move || rows("comments", store.state.with(|s| s.view.comments.clone()),
                "No comments on this page. Use Highlight or Note in the tool strip to add one.",
                &[("edit", "Edit"), ("delete", "Delete")])}
        </div>
    }
}

#[component]
fn Fields() -> impl IntoView {
    let store = store();
    view! {
        <div>
            {status("fields-warning")}
            {move || rows("fields", store.state.with(|s| s.view.fields.clone()),
                "This page has no form fields to fill. You can add some below.", &[])}
            <div class="row wrap" style="margin:12px 0">
                <PButton id="fill-sign" label="Sign…"/>
                <PButton id="fill-sign-draw" label="Draw my signature…"/>
            </div>
            <details class="designer" hidden=move || node("field-designer").hidden>
                <summary>"Create or change form fields"</summary>
                {status("field-designer-hint")}
                <h3>"Add a field"</h3>
                <PSelect id="field-new-kind" label="Kind" options=&[("text", "Text box"), ("checkbox", "Checkbox"), ("combo", "Drop-down list"), ("radio", "Choice buttons"), ("signature", "Signature")]/>
                <PField id="field-new-name" label="Name" hint="A short name that is unique in this form, like “email”."/>
                <div class="grid2">
                    <PField id="field-new-x" label="From the left" kind="number"/>
                    <PField id="field-new-y" label="From the top" kind="number"/>
                    <PField id="field-new-w" label="Width" kind="number"/>
                    <PField id="field-new-h" label="Height" kind="number"/>
                </div>
                <small style="color:var(--muted)">"Sizes are in points (72 to an inch)."</small>
                <PField id="field-new-maxlen" label="Most characters (0 for no limit)" kind="number"/>
                <PCheck id="field-new-multiline" label="Several lines"/>
                <PArea id="field-new-options" label="Choices, one per line"/>
                <PArea id="field-new-kids" label="Choice buttons, one name per line"/>
                <PField id="field-new-label" label="Description for screen readers"/>
                <PCheck id="field-new-readonly" label="Cannot be changed"/>
                <PCheck id="field-new-required" label="Must be filled in"/>
                <PField id="field-new-default" label="Starts with"/>
                <PCheck id="field-new-default-check" label="Starts checked"/>
                <PButton id="field-new-add" label="Add field" class="primary"/>
                <h3>"Change a field"</h3>
                <label class="field">
                    <span>"Field"</span>
                    <select class="input" on:change=move |ev| store.act(UiAction::Input { id: "field-select".into(), value: event_target_value(&ev) })>
                        {move || store.state.with(|s| s.view.lists.get("field-select").cloned().unwrap_or_default()).into_iter().map(|r| view! {
                            <option value=r.id.clone() selected=r.selected>{r.title.clone()}</option>
                        }).collect_view()}
                    </select>
                </label>
                <PField id="field-prop-name" label="Name"/>
                <div class="grid2">
                    <PField id="field-prop-x" label="From the left" kind="number"/>
                    <PField id="field-prop-y" label="From the top" kind="number"/>
                    <PField id="field-prop-w" label="Width" kind="number"/>
                    <PField id="field-prop-h" label="Height" kind="number"/>
                </div>
                <PField id="field-prop-maxlen" label="Most characters" kind="number"/>
                <PCheck id="field-prop-multiline" label="Several lines"/>
                <PArea id="field-prop-options" label="Choices, one per line"/>
                <PField id="field-prop-label" label="Description for screen readers"/>
                <PCheck id="field-prop-readonly" label="Cannot be changed"/>
                <PCheck id="field-prop-required" label="Must be filled in"/>
                <PField id="field-prop-default" label="Starts with"/>
                <PCheck id="field-prop-default-check" label="Starts checked"/>
                <div class="row"><PButton id="field-prop-delete" label="Delete field"/><span class="spacer"></span><PButton id="field-prop-apply" label="Apply" class="primary"/></div>
            </details>
        </div>
    }
}

#[component]
fn Links() -> impl IntoView {
    view! {
        <div>
            <div class="row wrap" style="margin-bottom:12px">
                <PButton id="link-draw" label="Draw a link" title="Drag on the page where the link should be"/>
                <PButton id="link-page-add" label="Link to a page…"/>
                <PButton id="link-uri-add" label="Link to a website…"/>
            </div>
            {move || rows("links", list("links"), "This page has no links.", &[("edit", "Change"), ("delete", "Delete")])}
        </div>
    }
}

#[component]
fn Selection() -> impl IntoView {
    view! {
        <div>
            <section hidden=move || node("content-edit-form").hidden class="panel-section">
                <h3>"Change this text"</h3>
                <p class="selectable">{move || node("content-edit-label").text}</p>
                <PField id="content-new-text" label="New text"/>
                {status("content-hint")}
                {status("content-repertoire")}
                <div class="row">
                    <PButton id="content-delete" label="Delete"/>
                    <span class="spacer"></span>
                    <PButton id="content-cancel" label="Cancel"/>
                    <PButton id="content-apply" label="Apply" class="primary"/>
                </div>
            </section>
            <section hidden=move || node("content-object-form").hidden class="panel-section">
                <h3>"Picture or drawing"</h3>
                <p>{move || node("content-object-label").text}</p>
                <div class="grid2">
                    <PField id="content-box-x" label="From the left" kind="number"/>
                    <PField id="content-box-y" label="From the top" kind="number"/>
                    <PField id="content-box-w" label="Width" kind="number"/>
                    <PField id="content-box-h" label="Height" kind="number"/>
                </div>
                {status("content-object-hint")}
                <div class="row wrap">
                    <PButton id="content-rotate-ccw" label="Rotate left"/>
                    <PButton id="content-rotate-cw" label="Rotate right"/>
                    <PButton id="content-flip-h" label="Flip across"/>
                    <PButton id="content-flip-v" label="Flip up and down"/>
                    <PButton id="content-crop" label="Crop"/>
                </div>
                <div class="row wrap" style="margin-top:8px">
                    <PButton id="content-replace" label="Replace picture…"/>
                    <PButton id="content-delete-object" label="Delete"/>
                    <span class="spacer"></span>
                    <PButton id="content-cancel-object" label="Cancel"/>
                    <PButton id="content-apply-object" label="Apply" class="primary"/>
                </div>
            </section>
            <section class="panel-section">
                <h3>"Add text"</h3>
                <p class="hint">"Choose the look, then click on the page where the text should start. You can also click empty space with the Text tool."</p>
                <PSelect id="content-add-font" label="Font" options=&[
                    ("Helvetica", "Helvetica"), ("Helvetica-Bold", "Helvetica Bold"), ("Helvetica-Oblique", "Helvetica Italic"), ("Helvetica-BoldOblique", "Helvetica Bold Italic"),
                    ("Times-Roman", "Times"), ("Times-Bold", "Times Bold"), ("Times-Italic", "Times Italic"), ("Times-BoldItalic", "Times Bold Italic"),
                    ("Courier", "Courier"), ("Courier-Bold", "Courier Bold"), ("Courier-Oblique", "Courier Italic"), ("Courier-BoldOblique", "Courier Bold Italic")]/>
                <div class="grid2">
                    <PField id="content-add-size" label="Size" kind="number"/>
                    <PField id="content-add-color" label="Colour" kind="color"/>
                </div>
                <div class="row"><PButton id="content-add-arm" label="Place on the page"/></div>
            </section>
            <details class="panel-section" open=move || !node("content-find-replace").open.eq(&false)>
                <summary>"Find and replace on this page"</summary>
                <PField id="content-find" label="Find"/>
                <PField id="content-replace-with" label="Replace with"/>
                <div class="row">
                    <label class="check"><input type="radio" name="content-scope" prop:checked=move || node("content-scope-page").checked
                        on:change=move |_| store().act(UiAction::Check { id: "content-scope-page".into(), checked: true })/>"This page"</label>
                    <label class="check"><input type="radio" name="content-scope" prop:checked=move || node("content-scope-run").checked disabled=move || node("content-scope-run").disabled
                        on:change=move |_| store().act(UiAction::Check { id: "content-scope-run".into(), checked: true })/>"The selected text only"</label>
                </div>
                <PButton id="content-preview-button" label="Find"/>
                {move || {
                    let matches = list("matches");
                    (!matches.is_empty()).then(|| view! {
                        <ul class="list">
                            {matches.into_iter().map(|m| { let id = m.id.clone(); view! {
                                <li><label class="check">
                                    <input type="checkbox" prop:checked=m.selected on:change=move |ev| store().act(UiAction::Check { id: id.clone(), checked: event_target_checked(&ev) })/>
                                    <span class="selectable">{m.title.clone()}<br/><small style="color:var(--muted)">{m.detail.clone()}</small></span>
                                </label></li>
                            }}).collect_view()}
                        </ul>
                    })
                }}
                {status("content-replace-summary")}
                <p class="hint">"Matches exact text, with capital letters as typed."</p>
                <div class="row"><PButton id="content-replace-cancel" label="Clear"/><span class="spacer"></span><PButton id="content-replace-apply" label="Replace checked" class="primary"/></div>
            </details>
            <details class="panel-section">
                <summary>"Text and pictures on this page"</summary>
                {move || {
                    let found = list("content");
                    // The core's line says why a page shows nothing (a refused page explains itself).
                    let empty = node("content-empty").text;
                    if found.is_empty() && !empty.is_empty() {
                        view! { <p class="empty">{empty}</p> }.into_any()
                    } else {
                        rows("content", found, "Nothing on this page can be changed.", &[])
                    }
                }}
                <PButton id="content-refresh" label="Look again"/>
            </details>
        </div>
    }
}

#[component]
fn Layers() -> impl IntoView {
    let store = store();
    view! {
        <div>
            {status("layers-note")}
            {move || {
                let layers = store.state.with(|s| s.view.layers.clone());
                if layers.is_empty() {
                    view! { <p class="empty">"This document has no layers."</p> }.into_any()
                } else {
                    view! {
                        <ul class="list">
                            {layers.into_iter().map(|l| { let id = l.id.clone(); view! {
                                <li><label class="check" style="padding:8px">
                                    <input type="checkbox" prop:checked=l.selected on:change=move |_| store.act(UiAction::Row { list: "layers".into(), id: id.clone(), action: "toggle".into() })/>
                                    <span>{l.title.clone()}</span>
                                </label></li>
                            }}).collect_view()}
                        </ul>
                        <p class="hint">"Showing or hiding a layer changes only what you see; it is not saved."</p>
                    }.into_any()
                }
            }}
        </div>
    }
}

#[component]
fn Redactions() -> impl IntoView {
    let store = store();
    view! {
        <div>
            <p class="hint">"Drag over anything you want removed. Nothing is removed until you choose Redact. Right-click the page to redact the whole page."</p>
            <p class="hint">"The pink tint shows what will be removed. It can reach past a box: a form field your box touches is removed whole."</p>
            {move || store.state.with(|s| s.view.doc.as_ref().is_some_and(|d| d.read_only)).then(|| view! {
                <p class="hint">"This file is signed: boxes can be removed and drawn again, not moved. Redacting saves an unsigned copy."</p>
            })}
            {move || rows("redactions", store.state.with(|s| s.view.redact.boxes.clone()),
                "No boxes yet. Drag over anything you want removed.", &[("remove", "Remove")])}
            <button class="btn quiet" on:click=move |_| store.act(UiAction::Edit { op: "redact-search".into(), contents: None })>"Find text to redact…"</button>
        </div>
    }
}

#[component]
fn RedactFooter() -> impl IntoView {
    let store = store();
    let r = move || store.state.with(|s| s.view.redact.clone());
    view! {
        <footer>
            <label class="check">
                <input type="checkbox" prop:checked=move || r().underneath on:change=move |ev| store.act(UiAction::Edit {
                    op: "redact-underneath".into(), contents: Some(serde_json::json!({ "on": event_target_checked(&ev) })) })/>
                "Show what's underneath"
            </label>
            {move || r().flatten_note.map(|n| view! { <p class="notice warning" style="border-radius:6px">{n}</p> })}
            <button class="btn redact lg wide" disabled=move || !r().can_apply
                on:click=move |_| store.act(UiAction::Edit { op: "redact-apply-open".into(), contents: None })>{move || r().apply_label}</button>
        </footer>
    }
}
