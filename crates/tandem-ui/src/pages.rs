//! The Pages tool (§3.2 / §3.3 "Pages"): a grid of every page. Select one
//! or many (click, ⇧-click for a range, ⌘-click — Ctrl-click on Windows and Linux — to
//! add), drag to reorder,
//! and rotate, delete, extract or insert. Each action is one Undo step.
//!
//! Dragging is done with pointer events, not HTML drag and drop: the window's native file
//! drop handler (Tauri's, which opens a PDF dropped on the window) answers every drag over
//! the web view itself, so the page never sees `dragover` or `drop`.

use crate::bridge;
use crate::icons::Icon;
use crate::store;
use leptos::html;
use leptos::prelude::*;
use tandem_protocol::keys::Os;
use tandem_protocol::ui::UiAction;
use wasm_bindgen::JsCast;

thread_local! {
    /// A page drag is under way: the window's own Esc handling (`lib.rs`) leaves it to the grid.
    static DRAGGING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether pages are being dragged in the grid right now.
pub fn dragging_pages() -> bool {
    DRAGGING.with(std::cell::Cell::get)
}

/// The gap a point over page cell `cell` names: the gap before it on its left half, the one
/// after it on its right half. Gaps are 0..=count: gap `g` is between pages `g - 1` and `g`.
pub fn gap_of(cell: usize, right_half: bool) -> usize {
    cell + usize::from(right_half)
}

/// The Pages "move" for dropping `pages` (any order) into gap `gap` of a `count`-page
/// document: the `to` the core takes (the gap itself: the moved pages go, in order, between
/// the pages either side of it), or None when the drop leaves every page where it is (a drop
/// inside or right beside a run of selected pages).
pub fn drop_target(pages: &[usize], gap: usize, count: usize) -> Option<usize> {
    let mut list: Vec<usize> = pages.iter().copied().filter(|p| *p < count).collect();
    list.sort_unstable();
    list.dedup();
    if list.is_empty() {
        return None;
    }
    let gap = gap.min(count);
    // Where the first moved page lands: the pages before the gap that stay.
    let base = (0..gap).filter(|p| list.binary_search(p).is_err()).count();
    let unchanged = list.iter().enumerate().all(|(k, p)| *p == base + k);
    (!unchanged).then_some(gap)
}

/// The gap under a point of the window, from the page cell there (None between cells).
fn gap_at(x: f64, y: f64) -> Option<usize> {
    let doc = web_sys::window()?.document()?;
    let cell = doc
        .element_from_point(x as f32, y as f32)?
        .closest(".pagegrid .cell")
        .ok()??;
    let index: usize = cell.get_attribute("data-cell")?.parse().ok()?;
    if cell.class_name().split_whitespace().any(|c| c == "end") {
        return Some(index);
    }
    let rect = cell.get_bounding_client_rect();
    Some(gap_of(index, x > rect.left() + rect.width() / 2.0))
}

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
    // The gap (0..=count) the dragged pages would drop into.
    let drop_at = RwSignal::new(None::<usize>);
    // A press on a card: (card, x, y); it becomes a drag once it moves 6 px.
    let press = StoredValue::new(None::<(usize, f64, f64)>);
    // The click that ends a drag selects nothing: a click until this time (ms) is swallowed.
    let swallow_click = StoredValue::new(0.0_f64);
    // Where the pointer was last seen during a drag (a wheel scroll moves the cards under it).
    let last_point = StoredValue::new((0.0_f64, 0.0_f64));
    let end_drag = move || {
        DRAGGING.with(|d| d.set(false));
        press.set_value(None);
        dragging.set(None);
        drop_at.set(None);
    };
    // Esc during a drag drops nothing (wherever the keyboard focus is); the click of the
    // release that follows selects nothing either.
    let esc = window_event_listener(leptos::ev::keydown, move |ev| {
        if ev.key() == "Escape" && dragging.get_untracked().is_some() {
            ev.prevent_default();
            ev.stop_propagation();
            end_drag();
            swallow_click.set_value(js_sys::Date::now() + 60_000.0);
        }
    });
    on_cleanup(move || {
        esc.remove();
        DRAGGING.with(|d| d.set(false));
    });
    // The gap under the point, or none outside the grid (a drop there moves nothing); a
    // point between cards inside the grid keeps the gap shown last.
    let track = move |grid: &web_sys::Element, x: f64, y: f64| {
        last_point.set_value((x, y));
        let r = grid.get_bounding_client_rect();
        if x < r.left() || x > r.right() || y < r.top() || y > r.bottom() {
            drop_at.set(None);
        } else if let Some(gap) = gap_at(x, y) {
            drop_at.set(Some(gap));
        }
    };
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
                <button class="btn" disabled=move || read_only.get()
                    title=move || store.os.get().titled("Insert a blank page after the selected page", "⌥⇧⌘N")
                    on:click=move |_| store.act(UiAction::Edit { op: "insert-blank".into(), contents: None })><Icon name="plus" class="sm"/>"Blank page"</button>
                <span class="spacer"></span>
                <span class="kbd" style="border:0">{move || store.os.get().text("Drag pages to reorder. ⇧-click selects a range, ⌘-click adds a page.")}</span>
            </div>
            <div class="grid" role="listbox" aria-multiselectable="true" aria-label="Pages"
                class:dragging=move || dragging.get().is_some()
                on:pointermove=move |ev: web_sys::PointerEvent| {
                    let Some((i, x0, y0)) = press.get_value() else { return };
                    let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
                    if dragging.get_untracked().is_none() {
                        if (x - x0).hypot(y - y0) < 6.0 { return; }
                        if ev.buttons() & 1 == 0 || read_only.get_untracked() { press.set_value(None); return; }
                        if !selected.with_untracked(|s| s.contains(&i)) {
                            selected.set(vec![i]);
                            anchor.set(Some(i));
                        }
                        dragging.set(Some(i));
                        DRAGGING.with(|d| d.set(true));
                        if let Some(el) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                            let _ = el.set_pointer_capture(ev.pointer_id());
                        }
                    }
                    ev.prevent_default();
                    // Near the top or bottom edge the grid scrolls, so a far place can be reached.
                    if let Some(el) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                        track(&el, x, y);
                        let r = el.get_bounding_client_rect();
                        if y < r.top() + 40.0 { el.scroll_by_with_x_and_y(0.0, -20.0); }
                        else if y > r.bottom() - 40.0 { el.scroll_by_with_x_and_y(0.0, 20.0); }
                    }
                }
                on:pointerup=move |_| {
                    let was = dragging.get_untracked();
                    let gap = drop_at.get_untracked();
                    end_drag();
                    if was.is_none() {
                        // A drag Esc stopped: only the click of this release is swallowed.
                        let now = js_sys::Date::now();
                        if swallow_click.get_value() > now + 400.0 { swallow_click.set_value(now + 400.0); }
                        return;
                    }
                    swallow_click.set_value(js_sys::Date::now() + 400.0);
                    let (Some(gap), false) = (gap, read_only.get_untracked()) else { return };
                    let list = selected.get_untracked();
                    let n = pages.with_untracked(|p| p.len());
                    if let Some(to) = drop_target(&list, gap, n) {
                        let mut list = list; list.sort_unstable();
                        store.act(UiAction::Pages { action: "move".into(), pages: list, to: Some(to) });
                    }
                }
                on:pointercancel=move |_| end_drag()
                on:scroll=move |ev: web_sys::Event| {
                    if dragging.get_untracked().is_none() { return; }
                    if let Some(el) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) {
                        let (x, y) = last_point.get_value();
                        track(&el, x, y);
                    }
                }
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if dragging.get_untracked().is_some() { return; }
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
                        <div class="cell" data-cell=i.to_string() class:drop-before=move || drop_at.get() == Some(i)>
                            <button class="pagecard" role="option" node_ref=card aria-selected=move || is_sel().to_string()
                                tabindex=move || if tab_stop() { "0" } else { "-1" }
                                on:focus=move |_| focus_at.set(Some(i))
                                aria-label=format!("Page {} of {total}", p.label)
                                on:pointerdown=move |ev: web_sys::PointerEvent| {
                                    swallow_click.set_value(0.0);
                                    if ev.button() == 0 && !read_only.get_untracked() {
                                        press.set_value(Some((i, f64::from(ev.client_x()), f64::from(ev.client_y()))));
                                    }
                                }
                                on:click=move |ev| {
                                    if js_sys::Date::now() < swallow_click.get_value() { swallow_click.set_value(0.0); return; }
                                    focus_at.set(Some(i)); click(i, &ev)
                                }
                                on:dblclick=move |_| { store.act(UiAction::Goto { page: i }); store.act(UiAction::Tool { id: "select".into(), variant: None }); }>
                                <canvas node_ref=canvas style=format!("width:{w}px;height:{h}px")></canvas>
                                <span>{p.label.clone()}</span>
                            </button>
                        </div>
                    }
                }).collect_view() }}
                <div class="cell end" data-cell=move || pages.with(|p| p.len()).to_string()
                    class:drop-before=move || drop_at.get().is_some_and(|g| g == pages.with(|p| p.len()))></div>
            </div>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::{drop_target, gap_of};

    /// The page order after the core moves `pages` into gap `to` (its `move_plan`).
    fn after(count: usize, pages: &[usize], to: usize) -> Vec<usize> {
        let stay: Vec<usize> = (0..count).filter(|p| !pages.contains(p)).collect();
        let base = stay.iter().filter(|p| **p < to).count();
        let mut order = stay;
        let mut moved = pages.to_vec();
        moved.sort_unstable();
        for (k, p) in moved.into_iter().enumerate() {
            order.insert(base + k, p);
        }
        order
    }

    #[test]
    fn a_point_names_the_gap_on_its_side_of_the_card() {
        assert_eq!(gap_of(0, false), 0);
        assert_eq!(gap_of(0, true), 1);
        assert_eq!(gap_of(4, true), 5);
    }

    #[test]
    fn one_page_dragged_up_or_down_lands_between_the_pages_it_was_dropped_between() {
        // Page 3 dropped between pages 1 and 2 becomes page 2.
        assert_eq!(drop_target(&[2], 1, 4), Some(1));
        assert_eq!(after(4, &[2], 1), [0, 2, 1, 3]);
        // Page 1 dropped between pages 3 and 4 goes after page 3.
        assert_eq!(drop_target(&[0], 3, 4), Some(3));
        assert_eq!(after(4, &[0], 3), [1, 2, 0, 3]);
        // Page 2 dropped between pages 3 and 4: one place down.
        assert_eq!(drop_target(&[1], 3, 4), Some(3));
        assert_eq!(after(4, &[1], 3), [0, 2, 1, 3]);
    }

    #[test]
    fn several_pages_keep_their_order_in_either_direction() {
        assert_eq!(drop_target(&[3, 2], 1, 5), Some(1));
        assert_eq!(after(5, &[2, 3], 1), [0, 2, 3, 1, 4]);
        assert_eq!(drop_target(&[0, 1], 4, 5), Some(4));
        assert_eq!(after(5, &[0, 1], 4), [2, 3, 0, 1, 4]);
        // A scattered selection gathers at the gap.
        assert_eq!(drop_target(&[0, 4], 2, 5), Some(2));
        assert_eq!(after(5, &[0, 4], 2), [1, 0, 4, 2, 3]);
    }

    #[test]
    fn the_ends_of_the_grid() {
        assert_eq!(drop_target(&[3], 0, 4), Some(0));
        assert_eq!(after(4, &[3], 0), [3, 0, 1, 2]);
        assert_eq!(drop_target(&[0], 4, 4), Some(4));
        assert_eq!(after(4, &[0], 4), [1, 2, 3, 0]);
        assert_eq!(drop_target(&[0], 9, 4), Some(4), "past the end is the end");
        assert_eq!(drop_target(&[0], 0, 4), None, "already first");
        assert_eq!(drop_target(&[3], 4, 4), None, "already last");
    }

    #[test]
    fn a_drop_on_or_beside_the_dragged_pages_changes_nothing() {
        // Either side of the dragged page itself.
        assert_eq!(drop_target(&[2], 2, 4), None);
        assert_eq!(drop_target(&[2], 3, 4), None);
        // Inside or beside a run of selected pages.
        for gap in 1..=4 {
            assert_eq!(drop_target(&[1, 2, 3], gap, 5), None, "gap {gap}");
        }
        // Inside a scattered selection it still gathers them.
        assert_eq!(drop_target(&[0, 2], 1, 4), Some(1));
        assert_eq!(after(4, &[0, 2], 1), [0, 2, 1, 3]);
        assert_eq!(drop_target(&[], 1, 4), None);
        assert_eq!(drop_target(&[7], 1, 4), None, "no such page");
    }
}
