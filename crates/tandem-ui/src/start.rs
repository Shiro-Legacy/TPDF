//! The start screen (UX audit X4): Open, New blank, New from images, recent
//! files and a drop zone. Shown when no document is open.

use crate::icons::Icon;
use crate::store;
use leptos::prelude::*;
use tandem_protocol::ui::UiAction;

#[component]
pub fn Start() -> impl IntoView {
    let store = store();
    let recent = move || store.state.with(|s| s.view.recent.clone());
    let notice = move || store.state.with(|s| s.view.notice.clone());
    view! {
        <main class="start" aria-labelledby="start-title">
            <div class="start-inner">
                <div class="brand">
                    <img src="assets/icon.png" alt=""/>
                    <span>"Tandem PDF"</span>
                    <span class="offline-pill" title="Your files never leave this Mac.">"Works offline"</span>
                </div>
                {move || notice().map(|n| view! {
                    <div class=format!("notice {}", n.kind) role=if n.kind == "error" { "alert" } else { "status" } style="margin-top:24px;border-radius:9px">
                        <p>{n.text}</p>
                    </div>
                })}
                <h1 id="start-title">"Open a PDF to get started."</h1>
                <p class="lede">"Read, fill, sign, edit and redact — everything stays on this Mac."</p>
                <div class="start-actions">
                    <button class="card primary" on:click=move |_| store.act(UiAction::Open { path: None })>
                        <Icon name="open"/>
                        <strong>"Open a PDF…"</strong>
                        <small>"Or drop one anywhere on this window. ⌘O"</small>
                    </button>
                    <button class="card" on:click=move |_| store.act(UiAction::New { size: None })>
                        <Icon name="new"/>
                        <strong>"New blank PDF"</strong>
                        <small>"One empty page to type on. ⌘N"</small>
                    </button>
                    <button class="card" on:click=move |_| store.act(UiAction::Edit { op: "new-from-images".into(), contents: None })>
                        <Icon name="images"/>
                        <strong>"New from images…"</strong>
                        <small>"Turn photos or scans into a PDF. ⌥⌘N"</small>
                    </button>
                </div>
                <section class="recent" aria-labelledby="recent-title">
                    <h2 id="recent-title">"Recent files"</h2>
                    {move || {
                        let files = recent();
                        if files.is_empty() {
                            view! { <p class="lede" style="font-size:13px">"Files you open will be listed here."</p> }.into_any()
                        } else {
                            view! {
                                <ul class="list">
                                    {files.into_iter().map(|f| {
                                        let path = f.path.clone();
                                        view! {
                                            <li>
                                                <button class="recent-row" on:click=move |_| store.act(UiAction::Open { path: Some(path.clone()) }) title=f.path.clone()>
                                                    <span class="file-badge">"PDF"</span>
                                                    <span>
                                                        <strong>{f.name.clone()}</strong>
                                                        <small>{format!("{} · {}", f.folder, f.when)}</small>
                                                    </span>
                                                </button>
                                            </li>
                                        }
                                    }).collect_view()}
                                </ul>
                            }.into_any()
                        }
                    }}
                </section>
                <p class="privacy">"Tandem PDF has no account and sends nothing over the network."</p>
            </div>
        </main>
    }
}
