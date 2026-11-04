//! Comments: the sidebar's list of every page's comments, grouped by page, and the popover
//! that opens a comment beside its annotation. The core decides what is listed and what is
//! open (`View::doc_comments`, `View::comment_popover`); this only draws them.

use crate::store;
use leptos::html;
use leptos::prelude::*;
use tandem_protocol::ui::{CommentPopover, CommentRow, UiAction};
use wasm_bindgen::JsCast;

/// The most a comment holds, in UTF-16 units (the engine's `MAX_NOTE_CHARS`).
const MAX_CHARS: usize = tandem_protocol::limits::MAX_NOTE_CHARS;

/// Tab and Shift-Tab stay inside the popover (a dialog): from its last control to its first,
/// and back.
fn trap_tab(ev: &web_sys::KeyboardEvent, root: NodeRef<html::Div>) {
    if ev.key() != "Tab" {
        return;
    }
    let Some(root) = root.get_untracked() else {
        return;
    };
    let Ok(list) = root.query_selector_all("textarea, button:not([disabled])") else {
        return;
    };
    let items: Vec<web_sys::HtmlElement> = (0..list.length())
        .filter_map(|i| list.item(i))
        .filter_map(|n| n.dyn_into().ok())
        .collect();
    let (Some(first), Some(last)) = (items.first(), items.last()) else {
        return;
    };
    let active = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element());
    let at = |e: &web_sys::HtmlElement| active.as_ref().is_some_and(|a| a.is_same_node(Some(e)));
    let to = if ev.shift_key() && at(first) {
        Some(last)
    } else if !ev.shift_key() && at(last) {
        Some(first)
    } else {
        None
    };
    if let Some(to) = to {
        ev.prevent_default();
        let _ = to.focus();
    }
}

/// The popover's width in CSS px (styles.css `.comment-popover`).
const POPOVER_W: f64 = 288.0;
/// Its gap from the annotation.
const POPOVER_GAP: f64 = 10.0;

fn row_act(id: &str, action: &str) {
    store().act(UiAction::Row {
        list: "doc-comments".into(),
        id: id.into(),
        action: action.into(),
    });
}

/// "Note", "Highlight": the class of the small marker drawn before the kind.
fn kind_class(kind: &str) -> &'static str {
    match kind {
        "Note" => "kind-note",
        "Highlight" => "kind-highlight",
        "Underline" | "Squiggly underline" => "kind-underline",
        "Strikethrough" => "kind-strike",
        "Text box" => "kind-textbox",
        _ => "kind-other",
    }
}

/// Who and when, as one muted line ("Ada · 4 Nov 2026, 09:30"), or nothing.
fn byline(author: &str, date: &str) -> Option<String> {
    let parts: Vec<&str> = [author, date]
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn card(r: CommentRow) -> impl IntoView {
    let id = r.id.clone();
    let id_edit = r.id.clone();
    let id_delete = r.id.clone();
    let spoken = format!("{} on page {}: {}", r.kind, r.page_label, r.text);
    let by = byline(&r.author, &r.date);
    view! {
        <li class="comment-card" class:current=r.current data-comment-id=r.id.clone()>
            <button class="comment-main" aria-current=r.current.then_some("true") aria-label=spoken
                on:click=move |_| row_act(&id, "select")>
                <span class="comment-kind">
                    <span class=format!("kind-mark {}", kind_class(&r.kind)) aria-hidden="true"></span>
                    {r.kind.clone()}
                </span>
                <span class="comment-text">{r.text.clone()}</span>
                {by.map(|b| view! { <span class="comment-by">{b}</span> })}
            </button>
            {r.editable.then(|| view! {
                <span class="comment-actions">
                    // A comment with no place on the page cannot open beside it.
                    {r.located.then(|| view! {
                        <button class="btn quiet sm" aria-label=format!("Edit {} on page {}", r.kind, r.page_label)
                            on:click=move |_| row_act(&id_edit, "edit")>"Edit"</button>
                    })}
                    <button class="btn quiet sm" aria-label=format!("Delete {} on page {}", r.kind, r.page_label)
                        on:click=move |_| row_act(&id_delete, "delete")>"Delete"</button>
                </span>
            })}
        </li>
    }
}

/// The sidebar's Comments tab: every page's comments under their page's label.
#[component]
pub fn CommentsList() -> impl IntoView {
    let store = store();
    let rows = Memo::new(move |_| {
        store
            .state
            .with(|s| s.view.doc_comments.clone().unwrap_or_default())
    });
    let list = NodeRef::<html::Div>::new();
    // The current comment (chosen on the page, or just added) is kept in view, once each
    // time it changes, so the list can still be scrolled by hand.
    let current =
        Memo::new(move |_| rows.with(|r| r.iter().find(|c| c.current).map(|c| c.id.clone())));
    Effect::new(move |_| {
        let Some(id) = current.get() else { return };
        let Some(el) = list.get() else { return };
        if let Ok(Some(row)) = el.query_selector(&format!("[data-comment-id=\"{id}\"]")) {
            let options = web_sys::ScrollIntoViewOptions::new();
            options.set_block(web_sys::ScrollLogicalPosition::Nearest);
            row.scroll_into_view_with_scroll_into_view_options(&options);
        }
    });
    view! {
        <div class="comment-list" node_ref=list>
            {move || {
                let rows = rows.get();
                if rows.is_empty() {
                    return view! {
                        <p class="empty">"No comments yet. Choose Note in the tool strip and click the page, or select text and highlight it."</p>
                    }.into_any();
                }
                // Grouped by page, in page order (the core lists them so).
                let mut groups: Vec<(usize, String, Vec<CommentRow>)> = Vec::new();
                for r in rows {
                    match groups.last_mut() {
                        Some((page, _, items)) if *page == r.page => items.push(r),
                        _ => groups.push((r.page, r.page_label.clone(), vec![r])),
                    }
                }
                groups.into_iter().map(|(page, label, items)| {
                    let heading = format!("Page {label}");
                    let count = items.len();
                    let group_label = format!("{heading}, {count} comments");
                    view! {
                        <section class="comment-group" aria-label=group_label>
                            <h3 class="comment-page">
                                <button class="comment-page-link" title=format!("Go to page {label}")
                                    on:click=move |_| store.act(UiAction::Goto { page })>{heading}</button>
                                <span class="comment-count">{count}</span>
                            </h3>
                            <ul class="comment-cards">{items.into_iter().map(card).collect_view()}</ul>
                        </section>
                    }
                }).collect_view().into_any()
            }}
        </div>
    }
}

/// Where the popover sits on the page (CSS px): to the right of the annotation, else to its
/// left, else under it; never off the page's left edge.
fn popover_at(rect: &[f64; 4], s: f64, page_w: f64) -> (f64, f64) {
    let right = rect[2] * s + POPOVER_GAP;
    let left = rect[0] * s - POPOVER_GAP - POPOVER_W;
    if right + POPOVER_W <= page_w {
        (right, rect[1] * s)
    } else if left >= 0.0 {
        (left, rect[1] * s)
    } else {
        (
            (rect[0] * s).min(page_w - POPOVER_W).max(0.0),
            rect[3] * s + POPOVER_GAP,
        )
    }
}

/// The comment open beside its annotation: what it says, who wrote it and when, a box to
/// change it, Save and Delete. Esc closes it without saving; a click outside keeps what was
/// typed (the core saves it).
#[component]
pub fn Popover(pop: CommentPopover, scale: f64, page_w: f64) -> impl IntoView {
    let store = store();
    // What has been typed so far, as the core holds it (the mounting memo leaves it out).
    let typed = store.state.with_untracked(|s| {
        s.view
            .comment_popover
            .as_ref()
            .filter(|p| p.id == pop.id)
            .map(|p| p.draft.clone())
    });
    let draft = RwSignal::new(typed.unwrap_or_else(|| pop.text.clone()));
    let area = NodeRef::<html::Textarea>::new();
    let root = NodeRef::<html::Div>::new();
    // Where the keyboard goes back to when it closes: the list row it was opened from, else
    // the comment on the page.
    let from_list = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .is_some_and(|a| a.closest(".comment-list").ok().flatten().is_some());
    let back_id = pop.id.clone();
    on_cleanup(move || {
        let id = back_id.clone();
        set_timeout(
            move || {
                let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
                    return;
                };
                // Only when the keyboard was left nowhere (it was inside the popover).
                let lost = doc.active_element().is_none_or(|a| a.tag_name() == "BODY");
                if !lost {
                    return;
                }
                let row = format!(".comment-list [data-comment-id=\"{id}\"] .comment-main");
                let mark = format!(".hit[data-target=\"annot-{id}\"]");
                let order = if from_list { [row, mark] } else { [mark, row] };
                let found = order
                    .iter()
                    .find_map(|q| doc.query_selector(q).ok().flatten())
                    .or_else(|| doc.query_selector(".viewport").ok().flatten());
                if let Some(el) = found.and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok()) {
                    let _ = el.focus();
                }
            },
            std::time::Duration::ZERO,
        );
    });
    Effect::new(move |_| {
        if let Some(a) = area.get() {
            let _ = a.focus();
            let end = a.value().len() as u32;
            let _ = a.set_selection_range(end, end);
        }
    });
    let id = pop.id.clone();
    let send = move |action: &str, text: Option<String>| {
        store.act(UiAction::Comment {
            action: action.into(),
            id: id.clone(),
            text,
        })
    };
    let send_out = send.clone();
    let outside = window_event_listener(leptos::ev::pointerdown, move |ev| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
        let inside = target
            .clone()
            .zip(root.get_untracked())
            .is_some_and(|(t, r)| r.contains(Some(&t)));
        // A press on a page is the core's own to handle (it closes this one and may open
        // another): only presses elsewhere (the sidebar, the toolbar) close it from here.
        let on_page = target.is_some_and(|t| t.closest(".page").ok().flatten().is_some());
        if !inside && !on_page {
            send_out("close", Some(draft.get_untracked()));
        }
    });
    on_cleanup(move || outside.remove());
    let (x, y) = popover_at(&pop.rect, scale, page_w);
    let by = byline(&pop.author, &pop.date);
    let editable = pop.editable;
    let changed = move || draft.with(|d| *d != pop.text);
    let send_key = send.clone();
    let send_input = send.clone();
    let send_save = send.clone();
    let send_delete = send.clone();
    let send_esc = send.clone();
    let send_close = send;
    let label = pop.kind.clone();
    view! {
        <div class="comment-popover" node_ref=root role="dialog" aria-label=label.clone()
            style=format!("left:{x}px;top:{y}px")
            on:keydown=move |ev: web_sys::KeyboardEvent| {
                // Keys inside the popover are its own (the page's Delete, Return and Tab
                // must not act on the comment behind it); Esc closes it from anywhere in it.
                ev.stop_propagation();
                if ev.key() == "Escape" {
                    ev.prevent_default();
                    send_esc("close", Some(draft.get_untracked()));
                }
                trap_tab(&ev, root);
            }
            on:pointerdown=|ev: web_sys::PointerEvent| ev.stop_propagation()
            on:contextmenu=|ev: web_sys::MouseEvent| ev.stop_propagation()>
            <header>
                <span class="comment-kind">
                    <span class=format!("kind-mark {}", kind_class(&pop.kind)) aria-hidden="true"></span>
                    {pop.kind.clone()}
                </span>
                <button class="icon-btn sm" aria-label="Close" title="Close (Esc)"
                    on:click=move |_| send_close("close", Some(draft.get_untracked()))>"×"</button>
            </header>
            {by.map(|b| view! { <p class="comment-by">{b}</p> })}
            <textarea node_ref=area aria-label=format!("{label} text") readonly=!editable
                maxlength=MAX_CHARS.to_string()
                placeholder=if editable { "Write a comment" } else { "" }
                prop:value=move || draft.get()
                on:input=move |ev| {
                    let v = event_target_value(&ev);
                    draft.set(v.clone());
                    send_input("input", Some(v));
                }
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    match ev.key().as_str() {
                        // ⌘Return (Ctrl+Enter on Windows and Linux) saves; Return is a new line.
                        "Enter" if store.primary(ev.meta_key(), ev.ctrl_key()) && !ev.is_composing() => {
                            ev.prevent_default();
                            send_key("save", Some(draft.get_untracked()));
                        }
                        _ => {}
                    }
                }></textarea>
            {move || (draft.with(|d| d.encode_utf16().count()) >= MAX_CHARS).then(|| view! {
                <p class="comment-limit" role="status">{format!("A comment holds up to {MAX_CHARS} characters.")}</p>
            })}
            {editable.then(|| view! {
                <footer>
                    <button class="btn quiet comment-delete" on:click=move |_| send_delete("delete", None)>"Delete"</button>
                    <span class="spacer"></span>
                    <button class="btn primary" disabled=move || !changed()
                        title=move || format!("Save ({})", store.os.get().text("⌘Return"))
                        on:click=move |_| send_save("save", Some(draft.get_untracked()))>"Save"</button>
                </footer>
            })}
        </div>
    }
}
