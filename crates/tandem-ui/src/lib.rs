//! The Tandem PDF user interface (docs/rust-and-ux-plan.md §3.2): a view of
//! the core's `UiState`. Every change goes through `act`, which sends one
//! `UiAction` and takes the fresh state back.

pub mod bridge;
pub mod dialogs;
pub mod editor;
pub mod icons;
pub mod pages;
pub mod panel;
pub mod screens;
pub mod start;
pub mod viewport;

use leptos::prelude::*;
use tandem_protocol::ui::{UiAction, UiState};
use wasm_bindgen::prelude::*;

/// The app-wide store: the last state and a way to act.
#[derive(Clone, Copy)]
pub struct Store {
    pub state: RwSignal<UiState>,
    /// Actions in flight (a busy cursor on slow work).
    pub pending: RwSignal<u32>,
    /// Files being dragged over the window (the shell's `drag` event).
    pub drag: RwSignal<Drag>,
    /// A note from the shell (a drop or Finder open it could not take as asked).
    pub note: RwSignal<Option<String>>,
}

/// The shell's drag state: whether files are over the window and whether one is a PDF.
#[derive(Clone, Default, PartialEq, serde::Deserialize)]
pub struct Drag {
    pub over: bool,
    pub pdf: bool,
    #[serde(default)]
    pub count: usize,
}

impl Store {
    /// Sends an action; the reply replaces the state. Errors become a notice.
    pub fn act(self, action: UiAction) {
        let state = self.state;
        let pending = self.pending;
        pending.update(|n| *n += 1);
        leptos::task::spawn_local(async move {
            match bridge::invoke::<_, UiState>("act", &serde_json::json!({ "action": action })).await {
                Ok(next) => state.set(next),
                Err(error) => state.update(|s| {
                    s.view.notice = Some(tandem_protocol::ui::Notice {
                        kind: "error".into(),
                        text: format!(
                            "Something went wrong inside the app ({error}). Try again; if it keeps happening, restart Tandem PDF."
                        ),
                        actions: vec![],
                    })
                }),
            }
            pending.update(|n| *n = n.saturating_sub(1));
        });
    }
}

pub fn store() -> Store {
    expect_context::<Store>()
}

/// A derived value that only notifies when it changes. Use it for anything that decides
/// *which* components exist (a pane open, a tab, the tool), so a state update does not
/// rebuild them — a rebuilt viewport repaints every page.
pub fn select<T: PartialEq + Clone + Send + Sync + 'static>(
    f: impl Fn(&UiState) -> T + Send + Sync + 'static,
) -> Memo<T> {
    let store = store();
    Memo::new(move |_| store.state.with(|s| f(s)))
}

#[component]
fn App() -> impl IntoView {
    let store = Store {
        state: RwSignal::new(UiState::default()),
        pending: RwSignal::new(0),
        drag: RwSignal::new(Drag::default()),
        note: RwSignal::new(None),
    };
    provide_context(store);
    // Background changes (menus, Finder, a drive script) arrive as events.
    bridge::listen::<UiState>("state", move |next| store.state.set(next));
    bridge::listen::<Drag>("drag", move |d| store.drag.set(d));
    bridge::listen::<String>("note", move |n| store.note.set(Some(n)));
    leptos::task::spawn_local(async move {
        if let Ok(first) = bridge::invoke::<_, UiState>("state", &serde_json::json!({})).await {
            store.state.set(first);
        }
        // A note sent before this page listened (files handed over at launch).
        if let Ok(Some(note)) =
            bridge::invoke::<_, Option<String>>("take_note", &serde_json::json!({})).await
        {
            store.note.set(Some(note));
        }
    });
    // A note goes away by itself after a while (it is also announced), or with its ×.
    Effect::new(move |_| {
        if let Some(text) = store.note.get() {
            leptos::task::spawn_local(async move {
                sleep_ms(9000).await;
                if store.note.get_untracked().as_deref() == Some(text.as_str()) {
                    store.note.set(None);
                }
            });
        }
    });
    let screen = Memo::new(move |_| store.state.with(|s| s.view.screen.clone()));
    // Keys the page area handles. Text fields, the search box and dialogs keep their own keys;
    // menu shortcuts (⌘S, ⌘Z…) arrive from the menu bar, not here.
    let keys = window_event_listener(leptos::ev::keydown, move |ev: web_sys::KeyboardEvent| {
        use wasm_bindgen::JsCast;
        let in_field = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok())
            .is_some_and(|t| {
                matches!(t.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT")
                    || t.is_content_editable()
            });
        let dialog_open = store.state.with_untracked(|s| s.view.dialog.is_some());
        if dialog_open || store.state.with_untracked(|s| s.view.screen != "editor") {
            return;
        }
        let key = ev.key();
        // Esc leaves Compare (the core keeps Escape for the page's own things).
        if key == "Escape" && !in_field && store.state.with_untracked(|s| s.view.compare.is_some())
        {
            ev.prevent_default();
            store.act(UiAction::Click {
                id: "compare-close".into(),
            });
            return;
        }
        if ev.meta_key() && !in_field {
            let what = match key.as_str() {
                "c" => Some("copy"),
                "x" => Some("cut"),
                "v" => Some("paste"),
                _ => None,
            };
            if let Some(what) = what {
                ev.prevent_default();
                store.act(UiAction::Clipboard { what: what.into() });
            }
            return;
        }
        if in_field && key != "Escape" {
            return;
        }
        // Something on the page is selected or being edited: the page's keys act on it
        // (arrows nudge, Delete removes, Tab moves between fields). Otherwise arrows, Page
        // Up/Down, Home, End and Space scroll the document natively.
        let focused = store.state.with_untracked(|s| {
            s.view
                .overlays
                .iter()
                .any(|o| o.editor.is_some() || o.targets.iter().any(|t| t.selected))
        });
        let arrows = matches!(
            key.as_str(),
            "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight"
        );
        if arrows && !focused && !ev.alt_key() {
            if let Some(v) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.query_selector(".viewport").ok().flatten())
                && let Ok(v) = v.dyn_into::<web_sys::HtmlElement>()
            {
                let in_viewport = ev
                    .target()
                    .and_then(|t| t.dyn_into::<web_sys::Node>().ok())
                    .is_some_and(|t| v.contains(Some(&t)));
                if !in_viewport {
                    let _ = v.focus();
                }
            }
            return;
        }
        let page_key = matches!(
            key.as_str(),
            "Escape" | "Delete" | "Backspace" | "Tab" | "Enter" | "+" | "-" | "=" | "F10"
        ) || arrows;
        if page_key {
            if key != "Tab"
                || store
                    .state
                    .with_untracked(|s| s.view.tool.id == "fill-sign")
            {
                ev.prevent_default();
            }
            store.act(UiAction::Key {
                key,
                shift: ev.shift_key(),
                alt: ev.alt_key(),
                meta: ev.meta_key(),
            });
        }
    });
    on_cleanup(move || keys.remove());
    // ⌘F from the menu bar puts the cursor in the search field.
    bridge::listen::<String>("focus", move |id| {
        if let Some(el) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(&id))
        {
            use wasm_bindgen::JsCast;
            if let Ok(input) = el.dyn_into::<web_sys::HtmlInputElement>() {
                let _ = input.focus();
                input.select();
            }
        }
    });
    view! {
        <div class="sr-only" role="status" aria-live="polite">{move || store.state.with(|s| s.view.announce.clone())}</div>
        {move || match screen.get().as_str() {
            "editor" => view! { <editor::Editor/> }.into_any(),
            "password" => view! { <screens::PasswordScreen/> }.into_any(),
            _ => view! { <start::Start/> }.into_any(),
        }}
        <dialogs::Dialogs/>
        <div class="toasts">
            {move || store.note.get().map(|n| view! {
                <div class="toast note" role="status">
                    <span>{n}</span>
                    <button class="toast-close" aria-label="Close" on:click=move |_| store.note.set(None)>
                        <icons::Icon name="close" class="sm"/>
                    </button>
                </div>
            })}
            {move || store.state.with(|s| s.view.toast.clone()).map(|t| view! { <div class="toast" role="status">{t}</div> })}
        </div>
        {move || {
            let d = store.drag.get();
            d.over.then(|| {
                let text = match (d.pdf, d.count) {
                    (false, _) => "Only PDF files can be opened",
                    (true, 1) => "Drop to open this PDF",
                    (true, _) => "Drop to open the first PDF · one file opens at a time",
                };
                view! { <div class=if d.pdf { "drop-veil" } else { "drop-veil refuse" } aria-hidden="true">{text}</div> }
            })
        }}
        {move || {
            store.state.with(|s| s.view.busy.clone()).map(|b| {
                view! {
                    <div class="busy-veil" aria-busy="true">
                        <div class="busy-card" role="alertdialog" aria-label=b.text.clone()>
                            <strong>{b.text.clone()}</strong>
                            {b.progress.map(|p| view! { <div class="progress"><i style=format!("width:{:.0}%", p * 100.0)></i></div> })}
                        </div>
                    </div>
                }
            })
        }}
    }
}

/// Waits `ms` milliseconds (a timer promise; no extra crate).
async fn sleep_ms(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

#[wasm_bindgen(start)]
pub fn start() {
    leptos::mount::mount_to_body(App);
}
