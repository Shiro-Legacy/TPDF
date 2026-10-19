//! Dialogs and the side panel. Each dialog is drawn from `view.dialog`
//! (its id and data) and answers with `UiAction::Dialog`.

use crate::icons::Icon;
use crate::store;
use leptos::prelude::*;
use tandem_protocol::ui::UiAction;

#[component]
pub fn Dialogs() -> impl IntoView {
    let store = store();
    let dialog = move || store.state.with(|s| s.view.dialog.clone());
    view! {
        {move || dialog().map(|d| {
            let id = d.id.clone();
            let id_close = d.id.clone();
            view! {
                <div class="scrim" on:keydown=move |ev| if ev.key() == "Escape" {
                    ev.stop_propagation();
                    store.act(UiAction::Dialog { id: id_close.clone(), button: "cancel".into(), values: serde_json::json!({}) });
                }>
                    <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title">
                        <h2 id="dialog-title">{d.title.clone()}</h2>
                        <p>{d.data.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string()}</p>
                        <div class="actions">
                            <button class="btn" on:click={ let id = id.clone(); move |_| store.act(UiAction::Dialog { id: id.clone(), button: "cancel".into(), values: serde_json::json!({}) }) }>"Cancel"</button>
                            <button class="btn primary" on:click=move |_| store.act(UiAction::Dialog { id: id.clone(), button: "ok".into(), values: serde_json::json!({}) })>"OK"</button>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

#[component]
pub fn PanelBody() -> impl IntoView {
    let store = store();
    let tab = move || store.state.with(|s| s.view.panel.tab.clone());
    let comments = move || store.state.with(|s| s.view.comments.clone());
    view! {
        <aside class="panel" aria-label="Details">
            <header>
                <h2>{move || match tab().as_str() {
                    "fields" => "Form fields", "redactions" => "Redactions", "signatures" => "Signatures",
                    "properties" => "Selection", "links" => "Links", "layers" => "Layers", "compare" => "Compare",
                    _ => "Comments",
                }}</h2>
                <button class="icon-btn" aria-label="Close the panel" on:click=move |_| store.act(UiAction::Pane { which: "panel".into(), open: Some(false), tab: None })>
                    <Icon name="close" class="sm"/>
                </button>
            </header>
            <div class="scroll">
                {move || {
                    let rows = comments();
                    if rows.is_empty() {
                        view! { <p class="empty">"No comments yet. Use Highlight or Note in the tool strip to add one."</p> }.into_any()
                    } else {
                        view! {
                            <ul class="list">
                                {rows.into_iter().map(|r| { let id = r.id.clone(); view! {
                                    <li><button class="list-row" aria-selected=r.selected.to_string()
                                        on:click=move |_| store.act(UiAction::Row { list: "comments".into(), id: id.clone(), action: "select".into() })>
                                        <strong>{r.title.clone()}</strong><small>{r.detail.clone()}</small>
                                    </button></li>
                                }}).collect_view()}
                            </ul>
                        }.into_any()
                    }
                }}
            </div>
        </aside>
    }
}
