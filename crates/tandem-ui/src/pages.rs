//! The Pages tool (§3.2 / §3.3 "Pages"): a grid of every page. Select one
//! or many (click, ⇧-click for a range, ⌘-click to add), drag to reorder,
//! and rotate, delete, extract or insert. Each action is one Undo step.

use crate::bridge;
use crate::icons::Icon;
use crate::store;
use leptos::html;
use leptos::prelude::*;
use tandem_protocol::ui::UiAction;

#[component]
pub fn PageGrid() -> impl IntoView {
    let store = store();
    let pages = Memo::new(move |_| store.state.with(|s| s.view.doc.as_ref().map(|d| d.pages.clone()).unwrap_or_default()));
    let generation = Memo::new(move |_| store.state.with(|s| s.view.generation));
    let read_only = Memo::new(move |_| store.state.with(|s| s.view.doc.as_ref().is_some_and(|d| d.read_only)));
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
    // Keep the selection inside the document after a delete.
    Effect::new(move |_| {
        let n = pages.with(|p| p.len());
        selected.update(|s| s.retain(|i| *i < n));
    });
    let act = move |action: &'static str| {
        let mut list = selected.get_untracked();
        list.sort_unstable();
        if list.is_empty() {
            return;
        }
        store.act(UiAction::Pages { action: action.into(), pages: list, to: None });
    };
    let count = move || selected.with(|s| s.len());
    // A document keeps at least one page.
    let cannot_delete = move || read_only.get() || count() == 0 || count() >= pages.with(|p| p.len());
    let click = move |i: usize, ev: &web_sys::MouseEvent| {
        if ev.shift_key() {
            let a = anchor.get_untracked().unwrap_or(i);
            let (lo, hi) = (a.min(i), a.max(i));
            selected.set((lo..=hi).collect());
        } else if ev.meta_key() {
            selected.update(|s| if let Some(k) = s.iter().position(|x| *x == i) { s.remove(k); } else { s.push(i); });
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
                <button class="btn" disabled=move || count() == 0 on:click=move |_| act("extract")><Icon name="export" class="sm"/>"Extract…"</button>
                <button class="btn" disabled=move || read_only.get() on:click=move |_| store.act(UiAction::Edit { op: "insert".into(), contents: None })><Icon name="plus" class="sm"/>"Insert pages…"</button>
                <span class="spacer"></span>
                <span class="kbd" style="border:0">"Drag pages to reorder. ⇧-click selects a range, ⌘-click adds a page."</span>
            </div>
            <div class="grid" role="listbox" aria-multiselectable="true" aria-label="Pages"
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    let n = pages.with_untracked(|p| p.len());
                    let cur = selected.with_untracked(|s| s.last().copied()).unwrap_or(0);
                    match ev.key().as_str() {
                        "ArrowRight" | "ArrowDown" if cur + 1 < n => { ev.prevent_default(); selected.set(vec![cur + 1]); anchor.set(Some(cur + 1)); }
                        "ArrowLeft" | "ArrowUp" if cur > 0 => { ev.prevent_default(); selected.set(vec![cur - 1]); anchor.set(Some(cur - 1)); }
                        "Delete" | "Backspace" if !read_only.get_untracked() => { ev.prevent_default(); act("delete"); }
                        "Enter" => { ev.prevent_default(); store.act(UiAction::Goto { page: cur }); store.act(UiAction::Tool { id: "select".into(), variant: None }); }
                        "a" if ev.meta_key() => { ev.prevent_default(); selected.set((0..n).collect()); }
                        _ => {}
                    }
                }>
                {move || pages.get().into_iter().enumerate().map(|(i, p)| {
                    let canvas = NodeRef::<html::Canvas>::new();
                    let w = 150.0_f64;
                    let h = (p.height / p.width.max(1.0)) * w;
                    Effect::new(move |_| {
                        let g = generation.get();
                        let Some(c) = canvas.get() else { return };
                        let s = w / p.width.max(1.0) * bridge::device_pixel_ratio();
                        leptos::task::spawn_local(async move {
                            if let Ok(px) = bridge::pixels("thumb", i, s, g).await { let _ = bridge::paint(&c, &px, w, h); }
                        });
                    });
                    let is_sel = move || selected.with(|s| s.contains(&i));
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
                            <button class="pagecard" role="option" aria-selected=move || is_sel().to_string()
                                aria-label=format!("Page {}", p.label) draggable=move || (!read_only.get()).to_string()
                                on:click=move |ev| click(i, &ev)
                                on:dblclick=move |_| { store.act(UiAction::Goto { page: i }); store.act(UiAction::Tool { id: "select".into(), variant: None }); }
                                on:dragstart=move |_| { if !selected.with_untracked(|s| s.contains(&i)) { selected.set(vec![i]); } dragging.set(Some(i)); }
                                on:dragend=move |_| { dragging.set(None); drop_at.set(None); }>
                                <canvas node_ref=canvas style=format!("width:{w}px;height:{h}px")></canvas>
                                <span>{p.label.clone()}</span>
                            </button>
                        </div>
                    }
                }).collect_view()}
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
