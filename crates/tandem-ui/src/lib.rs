//! The Tandem PDF user interface (docs/rust-and-ux-plan.md §3.2): a view of
//! the core's `UiState`. Every change goes through `act`, which sends one
//! `UiAction` and takes the fresh state back.

pub mod bridge;
pub mod dialogs;
pub mod editor;
pub mod icons;
pub mod pages;
pub mod panel;
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
pub fn select<T: PartialEq + Clone + Send + Sync + 'static>(f: impl Fn(&UiState) -> T + Send + Sync + 'static) -> Memo<T> {
    let store = store();
    Memo::new(move |_| store.state.with(|s| f(s)))
}

#[component]
fn App() -> impl IntoView {
    let store = Store { state: RwSignal::new(UiState::default()), pending: RwSignal::new(0) };
    provide_context(store);
    // Background changes (menus, Finder, a drive script) arrive as events.
    bridge::listen::<UiState>("state", move |next| store.state.set(next));
    leptos::task::spawn_local(async move {
        if let Ok(first) = bridge::invoke::<_, UiState>("state", &serde_json::json!({})).await {
            store.state.set(first);
        }
    });
    let screen = Memo::new(move |_| store.state.with(|s| s.view.screen.clone()));
    view! {
        <div class="sr-only" role="status" aria-live="polite">{move || store.state.with(|s| s.view.announce.clone())}</div>
        {move || match screen.get().as_str() {
            "editor" | "password" => view! { <editor::Editor/> }.into_any(),
            _ => view! { <start::Start/> }.into_any(),
        }}
        <dialogs::Dialogs/>
        {move || store.state.with(|s| s.view.toast.clone()).map(|t| view! { <div class="toast" role="status">{t}</div> })}
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

#[wasm_bindgen(start)]
pub fn start() {
    leptos::mount::mount_to_body(App);
}
