//! The Pages tool (§3.2 / §3.3 "Pages"): a grid of every page. Select one
//! or many (click, ⇧-click for a range, ⌘-click — Ctrl-click on Windows and Linux — to
//! add), drag to reorder,
//! and rotate, delete, extract or insert. Each action is one Undo step.

use crate::bridge;
use crate::icons::Icon;
use crate::store;
use leptos::html;
use leptos::prelude::*;
use tandem_protocol::keys::Os;
use tandem_protocol::ui::UiAction;
use wasm_bindgen::JsCast;

#[component]
pub fn PageGrid() -> impl IntoView {
    let store = store();
    let pages = Memo::new(move |_| {
        store.state.with(|s| {
            s.view
                .doc
                .as_ref()
                .map(|d| d.pages.clone())
                .unwrap_or_default()
        })
    });
    let generation = Memo::new(move |_| store.state.with(|s| s.view.generation));
    let read_only = Memo::new(move |_| {
        store
            .state
            .with(|s| s.view.doc.as_ref().is_some_and(|d| d.read_only))
    });
    let selected = RwSignal::new(Vec::<usize>::new());
    let anchor = RwSignal::new(None::<usize>);
    let dragging = RwSignal::new(None::<usize>);
    let drop_at = RwSignal::new(None::<usize>);
    // Start with the current page selected.
    Effect::new(move |_| {
        let p = store.state.with_untracked(|s| s.view.page);
        if selected.get_untracked().is_empty() {
            selected.set(vec![p]);
        }
    });
    // The core's selection after an action (moved pages stay selected at their new places,
    // a delete selects the page after it).
    let core_selection = Memo::new(move |_| store.state.with(|s| s.view.page_selection.clone()));
    Effect::new(move |prev: Option<Vec<usize>>| {
        let c = core_selection.get();
        if prev.is_some_and(|p| p != c) && !c.is_empty() {
            anchor.set(c.first().copied());
            selected.set(c.clone());
        }
        c
    });
    // Keep the selection inside the document after a delete, and tell the core (the Page
    // menu acts on these pages while the grid is shown).
    Effect::new(move |_| {
        let n = pages.with(|p| p.len());
        selected.update(|s| s.retain(|i| *i < n));
    });
    Effect::new(move |prev: Option<Vec<usize>>| {
        let mut list = selected.get();
        list.sort_unstable();
        if prev.as_ref() != Some(&list) {
            store.act(UiAction::Pages {
                action: "select".into(),
                pages: list.clone(),
                to: None,
            });
        }
        list
    });
    // The page that has keyboard focus in the grid (roving tabindex: one tab stop).
    let focus_at = RwSignal::new(None::<usize>);
    let focus_cell = move |i: usize| {
        focus_at.set(Some(i));
        if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| {
            d.query_selector(&format!(".pagegrid .cell:nth-child({}) .pagecard", i + 1))
                .ok()
                .flatten()
        }) && let Ok(el) = el.dyn_into::<web_sys::HtmlElement>()
        {
            let _ = el.focus();
        }
    };
    let move_by = move |delta: i64| {
        let mut list = selected.get_untracked();
        list.sort_unstable();
        let n = pages.with_untracked(|p| p.len());
        let (Some(first), Some(last)) = (list.first().copied(), list.last().copied()) else {
            return;
        };
        let to = if delta < 0 {
            if first == 0 {
                return;
            }
            first - 1
        } else {
            if last + 1 >= n {
                return;
            }
            last + 2
        };
        store.act(UiAction::Pages {
            action: "move".into(),
            pages: list,
            to: Some(to),
        });
    };
    let act = move |action: &'static str| {
        let mut list = selected.get_untracked();
        list.sort_unstable();
        if list.is_empty() {
            return;
        }
        store.act(UiAction::Pages {
            action: action.into(),
            pages: list,
            to: None,
        });
    };
    let count = move || selected.with(|s| s.len());
    // A document keeps at least one page.
    let cannot_delete =
        move || read_only.get() || count() == 0 || count() >= pages.with(|p| p.len());
    let click = move |i: usize, ev: &web_sys::MouseEvent| {
        if ev.shift_key() {
            let a = anchor.get_untracked().unwrap_or(i);
            let (lo, hi) = (a.min(i), a.max(i));
            selected.set((lo..=hi).collect());
        } else if store.primary(ev.meta_key(), ev.ctrl_key()) {
            selected.update(|s| {
                if let Some(k) = s.iter().position(|x| *x == i) {
                    s.remove(k);
                } else {
                    s.push(i);
                }
            });
            anchor.set(Some(i));
        } else {
            selected.set(vec![i]);
            anchor.set(Some(i));
        }
    };
    view! {
        <section class="pagegrid" aria-label="Pages">
            <div class="contextbar" role="toolbar" aria-label="Page actions">
                <span class="chip plain" role="status">{move || match count() { 0 => "No pages selected".to_string(), 1 => "1 page selected".to_string(), n => format!("{n} pages selected") }}</span>
                <button class="btn" disabled=move || read_only.get() || count() == 0 on:click=move |_| act("rotate-left")><Icon name="rotate-left" class="sm"/>"Rotate left"</button>
                <button class="btn" disabled=move || read_only.get() || count() == 0 on:click=move |_| act("rotate-right")><Icon name="rotate" class="sm"/>"Rotate right"</button>
                <button class="btn" disabled=cannot_delete on:click=move |_| act("delete")
                    title="Delete the selected pages (Undo brings them back)"><Icon name="trash" class="sm"/>"Delete"</button>
                <button class="btn" disabled=move || read_only.get() || selected.with(|s| s.is_empty() || s.contains(&0))
                    title=move || store.os.get().titled("Move the selected pages one place earlier", "⌥⌘↑") on:click=move |_| move_by(-1)>"Move earlier"</button>
                <button class="btn" disabled=move || read_only.get() || selected.with(|s| s.is_empty() || s.iter().any(|i| *i + 1 >= pages.with(|p| p.len())))
                    title=move || store.os.get().titled("Move the selected pages one place later", "⌥⌘↓") on:click=move |_| move_by(1)>"Move later"</button>
                <button class="btn" disabled=move || count() == 0 on:click=move |_| act("extract")><Icon name="export" class="sm"/>"Extract…"</button>
                <button class="btn" disabled=move || read_only.get() on:click=move |_| store.act(UiAction::Edit { op: "insert".into(), contents: None })><Icon name="plus" class="sm"/>"Insert pages…"</button>
                <span class="spacer"></span>
                <span class="kbd" style="border:0">{move || store.os.get().text("Drag pages to reorder. ⇧-click selects a range, ⌘-click adds a page.")}</span>
            </div>
            <div class="grid" role="listbox" aria-multiselectable="true" aria-label="Pages"
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    let os = store.os.get_untracked();
                    // Windows/Linux menu shortcuts (Ctrl+Shift+Backspace deletes the selected
                    // pages, Ctrl+Shift+Page Up moves them) are the menu bar's alone.
                    if os.menu_chord(ev.ctrl_key(), ev.alt_key(), ev.shift_key(), &ev.key()) { return; }
                    let n = pages.with_untracked(|p| p.len());
                    let cur = focus_at.get_untracked().or_else(|| selected.with_untracked(|s| s.last().copied())).unwrap_or(0);
                    let step = |to: usize, ev: &web_sys::KeyboardEvent| {
                        ev.prevent_default();
                        if ev.shift_key() {
                            let a = anchor.get_untracked().unwrap_or(cur);
                            selected.set((a.min(to)..=a.max(to)).collect());
                        } else {
                            selected.set(vec![to]);
                            anchor.set(Some(to));
                        }
                        focus_cell(to);
                    };
                    match ev.key().as_str() {
                        // ⌥⌘↑/↓ on a Mac (never Ctrl+Alt elsewhere: that is AltGr).
                        "ArrowUp" if os == Os::Mac && ev.meta_key() && ev.alt_key() => { ev.prevent_default(); move_by(-1); }
                        "ArrowDown" if os == Os::Mac && ev.meta_key() && ev.alt_key() => { ev.prevent_default(); move_by(1); }
                        "ArrowRight" | "ArrowDown" if cur + 1 < n => step(cur + 1, &ev),
                        "ArrowLeft" | "ArrowUp" if cur > 0 => step(cur - 1, &ev),
                        "Home" if n > 0 => step(0, &ev),
                        "End" if n > 0 => step(n - 1, &ev),
                        "Delete" | "Backspace" if !read_only.get_untracked() => { ev.prevent_default(); act("delete"); }
                        "Enter" => { ev.prevent_default(); store.act(UiAction::Goto { page: cur }); store.act(UiAction::Tool { id: "select".into(), variant: None }); }
                        "a" if store.primary(ev.meta_key(), ev.ctrl_key()) => { ev.prevent_default(); selected.set((0..n).collect()); }
                        _ => {}
                    }
                }>
                {move || { let all = pages.get(); let total = all.len(); all.into_iter().enumerate().map(move |(i, p)| {
                    let canvas = NodeRef::<html::Canvas>::new();
                    let card = NodeRef::<html::Button>::new();
                    let seen = RwSignal::new(false);
                    // Page sizes are as shown (rotation applied): a turned page is landscape.
                    let w = 150.0_f64;
                    let h = (p.height / p.width.max(1.0)) * w;
                    // Drawn only once it scrolls into view (a 500-page file does not render
                    // 500 pictures up front).
                    Effect::new(move |_| {
                        let Some(el) = card.get() else { return };
                        let callback = wasm_bindgen::closure::Closure::<dyn FnMut(js_sys::Array)>::new(
                            move |entries: js_sys::Array| {
                                if entries.iter().any(|e| e.dyn_into::<web_sys::IntersectionObserverEntry>().map(|e| e.is_intersecting()).unwrap_or(false)) {
                                    seen.set(true);
                                }
                            },
                        );
                        if let Ok(observer) = web_sys::IntersectionObserver::new(callback.as_ref().unchecked_ref()) {
                            observer.observe(&el);
                            callback.forget();
                            on_cleanup(move || observer.disconnect());
                        }
                    });
                    Effect::new(move |_| {
                        let g = generation.get();
                        if !seen.get() { return; }
                        let Some(c) = canvas.get() else { return };
                        let s = w / p.width.max(1.0) * bridge::device_pixel_ratio();
                        leptos::task::spawn_local(async move {
                            if let Ok(px) = bridge::pixels("thumb", i, s, g).await { let _ = bridge::paint(&c, &px, w, h); }
                        });
                    });
                    let is_sel = move || selected.with(|s| s.contains(&i));
                    // One tab stop: the focused card, else the last selected one.
                    let tab_stop = move || focus_at.get().or_else(|| selected.with(|s| s.last().copied())).unwrap_or(0) == i;
                    view! {
                        <div class="cell" class:drop-before=move || drop_at.get() == Some(i)
                            on:dragover=move |ev: web_sys::DragEvent| { ev.prevent_default(); drop_at.set(Some(i)); }
                            on:drop=move |ev: web_sys::DragEvent| {
                                ev.prevent_default();
                                drop_at.set(None);
                                if dragging.get_untracked().is_some() && !read_only.get_untracked() {
                                    let mut list = selected.get_untracked(); list.sort_unstable();
                                    store.act(UiAction::Pages { action: "move".into(), pages: list, to: Some(i) });
                                }
                                dragging.set(None);
                            }>
                            <button class="pagecard" role="option" node_ref=card aria-selected=move || is_sel().to_string()
                                tabindex=move || if tab_stop() { "0" } else { "-1" }
                                on:focus=move |_| focus_at.set(Some(i))
                                aria-label=format!("Page {} of {total}", p.label) draggable=move || (!read_only.get()).to_string()
                                on:click=move |ev| { focus_at.set(Some(i)); click(i, &ev) }
                                on:dblclick=move |_| { store.act(UiAction::Goto { page: i }); store.act(UiAction::Tool { id: "select".into(), variant: None }); }
                                on:dragstart=move |_| { if !selected.with_untracked(|s| s.contains(&i)) { selected.set(vec![i]); } dragging.set(Some(i)); }
                                on:dragend=move |_| { dragging.set(None); drop_at.set(None); }>
                                <canvas node_ref=canvas style=format!("width:{w}px;height:{h}px")></canvas>
                                <span>{p.label.clone()}</span>
                            </button>
                        </div>
                    }
                }).collect_view() }}
                <div class="cell end" class:drop-before=move || drop_at.get() == Some(usize::MAX)
                    on:dragover=move |ev: web_sys::DragEvent| { ev.prevent_default(); drop_at.set(Some(usize::MAX)); }
                    on:drop=move |ev: web_sys::DragEvent| {
                        ev.prevent_default(); drop_at.set(None);
                        if dragging.get_untracked().is_some() {
                            let mut list = selected.get_untracked(); list.sort_unstable();
                            store.act(UiAction::Pages { action: "move".into(), pages: list, to: Some(pages.with_untracked(|p| p.len())) });
                        }
                        dragging.set(None);
                    }></div>
            </div>
        </section>
    }
}
