//! The pages: a continuous, smoothly scrolling column of engine-drawn pages
//! with their overlays, plus the thumbnails and outline in the sidebar.
//!
//! Pixels are fetched lazily for the pages in view, at the window's device
//! pixel ratio, and painted with `putImageData`. Pointer gestures are sent to
//! the core in PDF points; the core decides what they mean for the armed tool.
//! Pinch (a ctrl-wheel in WebKit) and ⌘-scroll zoom around the pointer.

use crate::bridge;
use crate::store;
use leptos::html;
use leptos::prelude::*;
use tandem_protocol::keys::Os;
use tandem_protocol::ui::{ContextMenuView, EditorView, LiveBox, PageOverlay, Target, UiAction};
use wasm_bindgen::JsCast;

/// Gap between pages and the column's padding, in CSS px (styles.css `.pages`).
const GAP: f64 = 16.0;
const PAD_TOP: f64 = 24.0;
/// The reading point: this far down the view, the page under it is the current page.
const READ_AT: f64 = 0.35;

#[component]
pub fn Viewport() -> impl IntoView {
    let store = store();
    let scroller = NodeRef::<html::Section>::new();
    let scroll_top = RwSignal::new(0.0_f64);
    let height = RwSignal::new(800.0_f64);
    let pages = Memo::new(move |_| {
        store.state.with(|s| {
            s.view
                .doc
                .as_ref()
                .map(|d| d.pages.clone())
                .unwrap_or_default()
        })
    });
    let scale = Memo::new(move |_| store.state.with(|s| s.view.zoom.scale.max(0.05)));
    let fit_page = Memo::new(move |_| store.state.with(|s| s.view.zoom.mode == "fit-page"));
    let generation = Memo::new(move |_| store.state.with(|s| s.view.generation));
    let shown = Memo::new(move |_| store.state.with(|s| s.view.page));
    let tool = Memo::new(move |_| store.state.with(|s| s.view.tool.id.clone()));

    // Page tops in CSS px.
    let tops = Memo::new(move |_| {
        let s = scale.get();
        let mut y = PAD_TOP;
        pages
            .get()
            .iter()
            .map(|p| {
                let top = y;
                y += p.height * s + GAP;
                top
            })
            .collect::<Vec<f64>>()
    });
    let visible = Memo::new(move |_| {
        let (top, bottom) = (
            scroll_top.get() - 400.0,
            scroll_top.get() + height.get() + 400.0,
        );
        let s = scale.get();
        let ps = pages.get();
        tops.get()
            .iter()
            .enumerate()
            .filter(|(i, t)| **t < bottom && **t + ps[*i].height * s > top)
            .map(|(i, _)| i)
            .collect::<Vec<usize>>()
    });

    // Tell the core which pages are in view (overlays) and our size (fit zoom).
    Effect::new(move |prev: Option<Vec<usize>>| {
        let v = visible.get();
        if prev.as_ref() != Some(&v) {
            store.act(UiAction::Visible { pages: v.clone() });
        }
        v
    });
    let report_size = move || {
        if let Some(el) = scroller.get_untracked() {
            let (w, h) = (el.client_width() as f64, el.client_height() as f64);
            height.set(h);
            // Fit width never makes a page wider than a comfortable reading width
            // on a big screen (review: a 1660 px page at 1920). Re-check: the label stays
            // "Fit width"; it means "fit to a comfortable reading width" (decisions.md).
            // The page column's padding (styles.css .pages: 24 top, 16 sides, 32 bottom) is
            // not room for the page.
            store.act(UiAction::Viewport {
                width: (w - 32.0).min(1180.0),
                height: h - 56.0,
            });
        }
    };
    // The viewport's size changes with the window and with the sidebar and panel: watch it.
    Effect::new(move |_| {
        let Some(el) = scroller.get() else { return };
        report_size();
        let callback = wasm_bindgen::closure::Closure::<dyn FnMut(js_sys::Array)>::new(
            move |_entries: js_sys::Array| report_size(),
        );
        if let Ok(observer) = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            observer.observe(&el);
            callback.forget();
            on_cleanup(move || observer.disconnect());
        }
    });

    // What is being read (view#8): the document, the page at the reading point and how far
    // into it (in points). A zoom, a resize, the sidebar, the panel or a notice changes the
    // scale or the height; the view then puts the same point of the same page back at the
    // reading point (or under the pointer for a pinch), instead of keeping a scroll offset in
    // pixels that now shows another page.
    let doc_key = Memo::new(move |_| {
        store
            .state
            .with(|s| s.view.doc.as_ref().map(|d| (d.name.clone(), d.page_count)))
    });
    let anchor = RwSignal::new(None::<(Option<(String, usize)>, usize, f64)>);
    // While the view is put back, scroll events (the browser clamping the old offset) say
    // nothing about what is read.
    let restoring = RwSignal::new(false);
    // A pinch keeps the point under the pointer: (view y, view x, content x before, old scale).
    let pinch_at = RwSignal::new(None::<(f64, f64, f64, f64)>);
    let read_point = move |el: &web_sys::Element| -> Option<(usize, f64)> {
        let s = scale.get_untracked();
        let mid = el.scroll_top() as f64 + el.client_height() as f64 * READ_AT;
        tops.with_untracked(|t| {
            let i = t.iter().rposition(|top| *top <= mid)?;
            Some((i, (mid - t[i]) / s))
        })
    };
    Effect::new(move |prev: Option<(f64, f64)>| {
        let key = (scale.get(), height.get());
        if prev.is_none_or(|p| p == key) {
            return key;
        }
        let Some((doc, page, offset)) = anchor.get_untracked() else {
            return key;
        };
        if doc != doc_key.get_untracked() {
            return key;
        }
        restoring.set(true);
        // After the pages took their new size (the next frame), not before: a scroll offset
        // set now would be clamped to the old height.
        request_animation_frame(move || {
            if let Some(el) = scroller.get_untracked() {
                let s = scale.get_untracked();
                let pinch = pinch_at.get_untracked();
                let at = pinch.map_or(el.client_height() as f64 * READ_AT, |p| p.0);
                if let Some(top) = tops.with_untracked(|t| t.get(page).copied()) {
                    // Re-check 01h: Fit page shows the whole current page, its top at the top
                    // of the view (half the gap above it), not the page before's bottom.
                    let y = if fit_page.get_untracked() && pinch.is_none() {
                        top - GAP / 2.0
                    } else {
                        top + offset * s - at
                    };
                    el.set_scroll_top(y.max(0.0).round() as i32);
                }
                if let Some((_, px, content_x, old)) = pinch {
                    let k = s / old.max(0.01);
                    el.set_scroll_left((content_x * k - px).max(0.0).round() as i32);
                }
            }
            pinch_at.set(None);
            restoring.set(false);
        });
        key
    });

    // When the core moves to a page (search hit, outline, page box), bring it into view.
    let following = RwSignal::new(false);
    Effect::new(move |prev: Option<(usize, u64)>| {
        let key = (shown.get(), generation.get());
        let Some(el) = scroller.get() else { return key };
        let top = tops.with(|t| t.get(key.0).copied()).unwrap_or(0.0);
        let current = el.scroll_top() as f64;
        let in_view = top >= current - 1.0 && top < current + el.client_height() as f64 * 0.6;
        if prev.is_some_and(|p| p != key) && !in_view && !following.get_untracked() {
            el.set_scroll_top((top - 12.0).max(0.0) as i32);
        }
        following.set(false);
        key
    });

    // A comment row was chosen: bring its rectangle into view (the overlay flashes it).
    let reveal = Memo::new(move |_| store.state.with(|s| s.view.reveal.clone()));
    Effect::new(move |prev: Option<u64>| {
        let r = reveal.get();
        let seq = r.as_ref().map_or(0, |r| r.seq);
        if let Some(r) = r
            && prev.is_some_and(|p| p != seq)
            && let Some(el) = scroller.get()
        {
            let s = scale.get_untracked();
            let top = tops
                .with_untracked(|t| t.get(r.page).copied())
                .unwrap_or(0.0);
            let y = top + r.rect[1] * s;
            let view_h = el.client_height() as f64;
            let current = el.scroll_top() as f64;
            if y < current || y + (r.rect[3] - r.rect[1]) * s > current + view_h {
                following.set(true);
                el.set_scroll_top((y - view_h / 3.0).max(0.0) as i32);
            }
        }
        seq
    });

    let on_scroll = move |_| {
        let Some(el) = scroller.get_untracked() else {
            return;
        };
        let y = el.scroll_top() as f64;
        scroll_top.set(y);
        if restoring.get_untracked() {
            return;
        }
        // The page at the reading point becomes the current page: the number shown is
        // the page on screen.
        let point = read_point(&el);
        anchor.set(point.map(|(i, off)| (doc_key.get_untracked(), i, off)));
        let idx = point.map_or(0, |(i, _)| i);
        if idx != shown.get_untracked() {
            following.set(true);
            store.act(UiAction::Goto { page: idx });
        }
    };

    // Pinch and ⌘/ctrl-wheel zoom, anchored at the pointer.
    let zoom_pending = RwSignal::new(None::<f64>);
    let on_wheel = move |ev: web_sys::WheelEvent| {
        if !(ev.ctrl_key() || ev.meta_key()) {
            return;
        }
        ev.prevent_default();
        let base = zoom_pending
            .get_untracked()
            .unwrap_or_else(|| scale.get_untracked());
        let factor = (-ev.delta_y() * 0.01).exp().clamp(0.5, 2.0);
        let next = (base * factor).clamp(0.1, 8.0);
        zoom_pending.set(Some(next));
        if let Some(el) = scroller.get_untracked() {
            // Keep the point under the pointer still: it becomes what is read, and the view
            // puts it back under the pointer once the pages have their new size.
            let rect = el.get_bounding_client_rect();
            let (px, py) = (
                ev.client_x() as f64 - rect.left(),
                ev.client_y() as f64 - rect.top(),
            );
            let y = el.scroll_top() as f64 + py;
            let s = scale.get_untracked();
            let under = tops.with_untracked(|t| {
                let i = t.iter().rposition(|top| *top <= y)?;
                Some((i, (y - t[i]) / s))
            });
            if let Some((i, off)) = under.filter(|_| !restoring.get_untracked()) {
                anchor.set(Some((doc_key.get_untracked(), i, off)));
                pinch_at.set(Some((py, px, el.scroll_left() as f64 + px, s)));
            }
        }
        store.act(UiAction::Zoom {
            to: "fixed".into(),
            scale: Some(next),
        });
        zoom_pending.set(None);
    };

    view! {
        <section class="viewport" node_ref=scroller tabindex="0" aria-label="Document pages"
            data-tool=move || tool.get()
            on:scroll=on_scroll on:wheel=on_wheel>
            <div class="pages">
                {move || {
                    let ps = pages.get();
                    ps.into_iter().enumerate().map(|(i, p)| {
                        view! { <PageView index=i width=p.width height=p.height scale=scale generation=generation visible=visible/> }
                    }).collect_view()
                }}
            </div>
        </section>
    }
}

#[component]
fn PageView(
    index: usize,
    width: f64,
    height: f64,
    scale: Memo<f64>,
    generation: Memo<u64>,
    visible: Memo<Vec<usize>>,
) -> impl IntoView {
    let store = store();
    let canvas = NodeRef::<html::Canvas>::new();
    // The pixels of an annotation being moved, carried with the pointer while the page is
    // drawn without it.
    let carry = NodeRef::<html::Canvas>::new();
    let painted = RwSignal::new(None::<(u64, i64, Option<i64>)>);
    let loading = RwSignal::new(true);
    let is_visible = Memo::new(move |_| visible.with(|v| v.contains(&index)));
    let css_w = move || width * scale.get();
    let css_h = move || height * scale.get();

    let overlay = Memo::new(move |_| {
        store
            .state
            .with(|s| s.view.overlays.iter().find(|o| o.page == index).cloned())
    });
    let dragging = RwSignal::new(false);
    // The press point and the pointer since, in page points, as the window saw them: a live
    // box follows the pointer at once, without waiting for the core's answer. Like the core,
    // it stays put until the pointer has gone 3 px.
    let press = RwSignal::new(None::<Press>);
    let follow = Memo::new(move |_| {
        let p = press.get().filter(|p| p.moved)?;
        let (page, ax, ay) = store.pointer_at.get()?;
        (page == index).then_some(Pointer {
            now: (p.now.0 - p.down.0, p.now.1 - p.down.1),
            answered: (ax - p.down.0, ay - p.down.1),
            shift: p.shift,
        })
    });
    // An annotation or widget being moved (not resized: its text would wrap anew), once the
    // pointer has really moved: the page is drawn without it and its pixels follow the box.
    let moving = Memo::new(move |_| {
        follow.get()?;
        let live = overlay.with(|o| o.as_ref()?.live.clone())?;
        (live.handle == "grip")
            .then_some(live.hide?)
            .map(|id| (id, live))
    });
    let hidden = Memo::new(move |_| moving.with(|m| m.as_ref().map(|(id, _)| *id)));
    // Where the carried pixels are drawn (page points): with the box while it moves, then
    // where it was let go until the page is drawn with the annotation there.
    let carried = RwSignal::new(None::<[f64; 4]>);
    Effect::new(move |_| {
        let Some((_, live)) = moving.get() else {
            return;
        };
        let answered = overlay.with(|o| {
            let t = o.as_ref()?.targets.iter().find(|t| t.id == live.target)?;
            Some(t.rect)
        });
        if let (Some(r), Some(p)) = (answered, follow.get()) {
            carried.set(Some(follow_rect(r, &live, p)));
        }
    });
    // Each paint has a number: only the latest one asked for is drawn, so a slow draw
    // without the annotation never lands over the page drawn after the drop.
    let paints = StoredValue::new(0u64);

    Effect::new(move |_| {
        if !is_visible.get() {
            return;
        }
        let dpr = bridge::device_pixel_ratio();
        let s = scale.get();
        let hide = hidden.get();
        let key = (generation.get(), (s * dpr * 1000.0).round() as i64, hide);
        if painted.get_untracked() == Some(key) {
            return;
        }
        let Some(c) = canvas.get() else { return };
        // Moving starts: its pixels are taken from the page as drawn, before the page is
        // drawn without it.
        if let (Some((_, live)), Some(to)) = (moving.get_untracked(), carry.get_untracked())
            && painted.get_untracked().is_some_and(|p| p.2.is_none())
        {
            let k = c.width() as f64 / width.max(1.0);
            let r = live.initial;
            let region = [r[0] * k, r[1] * k, (r[2] - r[0]) * k, (r[3] - r[1]) * k];
            if bridge::copy_region(&c, &to, region).is_ok() {
                carried.set(Some(r));
            }
        }
        let quiet = hide.is_some() || carried.get_untracked().is_some();
        if !quiet {
            loading.set(true);
        }
        let number = paints.get_value() + 1;
        paints.set_value(number);
        leptos::task::spawn_local(async move {
            let t0 = bridge::now_ms();
            match bridge::pixels_hiding("page", index, s * dpr, key.0, hide).await {
                // A later paint was asked for: it draws (and ends the loading look).
                Ok(_) if paints.get_value() != number => return,
                Ok(px) => match bridge::paint(&c, &px, width * s, height * s) {
                    Ok(()) => {
                        bridge::log(format!(
                            "painted {index} in {:.0} ms",
                            bridge::now_ms() - t0
                        ));
                        painted.set(Some(key));
                        // The page shows the annotation again (where it was let go).
                        if hide.is_none() {
                            carried.set(None);
                        }
                    }
                    Err(e) => bridge::log(format!("paint {index}: {e}")),
                },
                Err(e) => bridge::log(format!("pixels {index}: {e}")),
            }
            loading.set(false);
        });
    });
    // The resize or move cursor stays while the press lasts, wherever the pointer goes.
    let drag_cursor = RwSignal::new(None::<&'static str>);
    let point = move |ev: &web_sys::PointerEvent| -> (f64, f64) {
        let target: web_sys::Element = ev
            .current_target()
            .and_then(|t| t.dyn_into().ok())
            .expect("layer");
        let r = target.get_bounding_client_rect();
        let s = scale.get_untracked();
        (
            (ev.client_x() as f64 - r.left()) / s,
            (ev.client_y() as f64 - r.top()) / s,
        )
    };
    let send = move |phase: &str, ev: &web_sys::PointerEvent| {
        let (x, y) = point(ev);
        match phase {
            "down" => press.set(Some(Press {
                down: (x, y),
                now: (x, y),
                moved: false,
                shift: ev.shift_key(),
            })),
            // Let go, the box stays where it was let go until the core's answer moves it.
            "move" | "up" => press.update(|p| {
                if let Some(p) = p {
                    let s = scale.get_untracked();
                    p.moved |= (x - p.down.0).hypot(y - p.down.1) * s >= 3.0;
                    p.now = (x, y);
                    p.shift = ev.shift_key();
                }
            }),
            _ => press.set(None),
        }
        store.act(UiAction::Pointer {
            phase: phase.into(),
            page: index,
            x,
            y,
            shift: ev.shift_key(),
            alt: ev.alt_key(),
            // The command key: ⌘ on a Mac, Ctrl on Windows and Linux (⌘-click follows a link).
            meta: store.primary(ev.meta_key(), ev.ctrl_key()),
        });
    };

    view! {
        <div class="page" class:loading=move || loading.get() style=move || format!("width:{}px;height:{}px", css_w(), css_h())
            role="img" aria-label=format!("Page {}", index + 1)>
            <canvas node_ref=canvas aria-hidden="true"></canvas>
            <canvas class="carry" node_ref=carry aria-hidden="true"
                style=move || carried.get().map_or("display:none".to_string(), |r| rect_style(&r, scale.get()))></canvas>
            <div class="layer" class:dragging=move || drag_cursor.get().is_some()
                style:cursor=move || drag_cursor.get()
                on:contextmenu=move |ev: web_sys::MouseEvent| {
                    // Right-click (or Control-click): the page's menu at that point.
                    ev.prevent_default();
                    let target: Option<web_sys::Element> = ev.current_target().and_then(|t| t.dyn_into().ok());
                    if let Some(target) = target {
                        let r = target.get_bounding_client_rect();
                        let s = scale.get_untracked();
                        store.act(UiAction::ContextMenu {
                            page: index,
                            x: (ev.client_x() as f64 - r.left()) / s,
                            y: (ev.client_y() as f64 - r.top()) / s,
                        });
                    }
                }
                on:pointerdown=move |ev: web_sys::PointerEvent| {
                    // Control-click is the context menu on a Mac (handled by contextmenu); on
                    // Windows and Linux Ctrl-click is the command click.
                    if ev.button() != 0 || (ev.ctrl_key() && store.os.get_untracked() == Os::Mac) { return; }
                    // The context menu handles its own clicks.
                    if ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).is_some_and(|t| t.closest(".ctx-menu").ok().flatten().is_some()) { return; }
                    // Inputs inside the layer handle their own pointer.
                    if ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok()).is_some_and(|t| t.tag_name() == "TEXTAREA" || t.tag_name() == "INPUT") { return; }
                    ev.prevent_default();
                    if let Some(el) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) { let _ = el.set_pointer_capture(ev.pointer_id()); }
                    let handle = ev.target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()).and_then(|t| t.get_attribute("data-handle"));
                    drag_cursor.set(handle.as_deref().map(handle_cursor));
                    dragging.set(true);
                    send("down", &ev);
                }
                on:pointermove=move |ev: web_sys::PointerEvent| if dragging.get_untracked() { send("move", &ev) }
                on:pointerup=move |ev: web_sys::PointerEvent| if dragging.get_untracked() { dragging.set(false); drag_cursor.set(None); send("up", &ev) }
                on:pointercancel=move |ev: web_sys::PointerEvent| if dragging.get_untracked() { dragging.set(false); drag_cursor.set(None); send("cancel", &ev) }>
                // Rebuilt when the zoom changes too, so boxes and tint never sit at an old scale.
                {move || overlay.get().map(|o| view! { <Overlay overlay=o scale=scale.get() follow=follow/> })}
                {move || {
                    let menu = store.state.with(|s| s.view.menu.clone()).filter(|m| m.page == index);
                    menu.map(|m| view! { <ContextMenu menu=m scale=scale.get()/> })
                }}
            </div>
        </div>
    }
}

/// The cursor a handle shows, kept for the whole drag that starts on it.
fn handle_cursor(handle: &str) -> &'static str {
    match handle {
        "nw" | "se" => "nwse-resize",
        "ne" | "sw" => "nesw-resize",
        "n" | "s" => "ns-resize",
        "e" | "w" => "ew-resize",
        _ => "move",
    }
}

/// A press on a page, in page points.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    down: (f64, f64),
    now: (f64, f64),
    /// The pointer has gone 3 px from `down`.
    moved: bool,
    shift: bool,
}

/// Where the pointer is from the press: now, and at the position the shown state answers.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pointer {
    now: (f64, f64),
    answered: (f64, f64),
    shift: bool,
}

/// The rect the core's gesture makes of `live.initial` with the pointer (dx, dy) page points
/// from the press (tandem-app session/nav.rs `gesture_rect` and `follow_corner`).
fn gesture_rect(live: &LiveBox, (dx, dy): (f64, f64), shift: bool) -> [f64; 4] {
    let [x0, y0, x1, y1] = live.initial;
    let (w0, h0) = (x1 - x0, y1 - y0);
    let min = live.min;
    let (lx, ly, rx, ry) = match live.bounds {
        Some(b) => (b[0], b[1], b[2], b[3]),
        None => (f64::MIN, f64::MIN, f64::MAX, f64::MAX),
    };
    let h = live.handle.as_str();
    if h == "grip" {
        let x = lx.max((rx - w0).min(x0 + dx));
        let y = ly.max((ry - h0).min(y0 + dy));
        return [x, y, x + w0, y + h0];
    }
    let (west, east, north, south) = (
        h.contains('w'),
        h.contains('e'),
        h.contains('n'),
        h.contains('s'),
    );
    let corner = (west || east) && (north || south) && w0 > 0.0 && h0 > 0.0;
    let scaled = |s: f64| {
        let most = (if east { (rx - x0) / w0 } else { (x1 - lx) / w0 }).min(if south {
            (ry - y0) / h0
        } else {
            (y1 - ly) / h0
        });
        let s = most.min(s.max(min / w0).max(min / h0));
        let (w, h) = (w0 * s, h0 * s);
        [
            if east { x0 } else { x1 - w },
            if south { y0 } else { y1 - h },
            if east { x0 + w } else { x1 },
            if south { y0 + h } else { y1 },
        ]
    };
    let w = w0 + if east { dx } else { -dx };
    let hh = h0 + if south { dy } else { -dy };
    if corner && live.aspect == "keep" {
        // The corner follows the pointer's projection on the box's diagonal.
        return scaled((w * w0 + hh * h0) / (w0 * w0 + h0 * h0));
    }
    if corner && live.aspect == "lock" && !shift {
        return scaled((w / w0).max(hh / h0));
    }
    let mut r = live.initial;
    if west {
        r[0] = lx.max((x1 - min).min(x0 + dx));
    }
    if east {
        r[2] = rx.min((x0 + min).max(x1 + dx));
    }
    if north {
        r[1] = ly.max((y1 - min).min(y0 + dy));
    }
    if south {
        r[3] = ry.min((y0 + min).max(y1 + dy));
    }
    r
}

/// Where a live box is drawn: the core's answer for the pointer it saw, moved on by what its
/// gesture does with the pointer since. Until the core's box leaves where it began (its 3 px),
/// the answer is for the press point.
fn follow_rect(answered: [f64; 4], live: &LiveBox, p: Pointer) -> [f64; 4] {
    let still = answered
        .iter()
        .zip(&live.initial)
        .all(|(a, b)| (a - b).abs() < 1e-6);
    let base = gesture_rect(live, if still { (0.0, 0.0) } else { p.answered }, p.shift);
    let now = gesture_rect(live, p.now, p.shift);
    [0, 1, 2, 3].map(|i| answered[i] + now[i] - base[i])
}

fn rect_style(r: &[f64; 4], s: f64) -> String {
    format!(
        "left:{}px;top:{}px;width:{}px;height:{}px",
        r[0] * s,
        r[1] * s,
        (r[2] - r[0]).max(0.0) * s,
        (r[3] - r[1]).max(0.0) * s
    )
}

/// The page's context menu (right-click, Control-click): what the core offers there.
#[component]
fn ContextMenu(menu: ContextMenuView, scale: f64) -> impl IntoView {
    let store = store();
    let list = NodeRef::<html::Div>::new();
    Effect::new(move |_| {
        if let Some(el) = list.get()
            && let Ok(Some(first)) = el.query_selector("button:not([disabled])")
            && let Ok(first) = first.dyn_into::<web_sys::HtmlElement>()
        {
            let _ = first.focus();
        }
    });
    let keydown = move |ev: web_sys::KeyboardEvent| {
        let key = ev.key();
        if key == "Escape" {
            ev.prevent_default();
            ev.stop_propagation();
            store.act(UiAction::MenuItem {
                item: "close".into(),
            });
            return;
        }
        if key != "ArrowDown" && key != "ArrowUp" {
            return;
        }
        ev.prevent_default();
        ev.stop_propagation();
        let Some(el) = list.get_untracked() else {
            return;
        };
        let Ok(items) = el.query_selector_all("button:not([disabled])") else {
            return;
        };
        let n = items.length();
        if n == 0 {
            return;
        }
        let active = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.active_element());
        let mut at = 0;
        for i in 0..n {
            if let (Some(item), Some(a)) = (items.item(i), active.as_ref())
                && item.is_same_node(Some(a))
            {
                at = i;
            }
        }
        let next = if key == "ArrowDown" {
            (at + 1) % n
        } else {
            (at + n - 1) % n
        };
        if let Some(item) = items
            .item(next)
            .and_then(|i| i.dyn_into::<web_sys::HtmlElement>().ok())
        {
            let _ = item.focus();
        }
    };
    view! {
        <div class="ctx-menu" role="menu" aria-label="Page" node_ref=list
            style=format!("left:{}px;top:{}px", menu.x * scale, menu.y * scale)
            on:pointerdown=move |ev| ev.stop_propagation()
            on:keydown=keydown>
            {menu.items.into_iter().map(|(id, label, enabled)| view! {
                <button role="menuitem" class="btn quiet" disabled=!enabled
                    on:click=move |_| store.act(UiAction::MenuItem { item: id.clone() })>{label}</button>
            }).collect_view()}
        </div>
    }
}

#[component]
fn Overlay(
    overlay: PageOverlay,
    scale: f64,
    /// The pointer of the press under way, from the press.
    follow: Memo<Option<Pointer>>,
) -> impl IntoView {
    let store = store();
    let s = scale;
    let current = overlay.current_hit;
    let page = overlay.page;
    // A chosen comment flashes once on its page (a new element per request replays it).
    let flash = move || {
        store
            .state
            .with(|st| st.view.reveal.clone())
            .filter(|r| r.page == page)
    };
    view! {
        {overlay.hits.iter().enumerate().map(|(i, r)| view! {
            <div class="search-hit" class:current=Some(i) == current style=rect_style(r, s)></div>
        }).collect_view()}
        {overlay.selection.iter().map(|r| view! { <div class="text-sel" style=rect_style(r, s)></div> }).collect_view()}
        {overlay.targets.iter().map(|t| {
            let live = overlay.live.clone().filter(|l| l.target == t.id).map(|l| (l, follow));
            target_view(t, s, live)
        }).collect_view()}
        {overlay.tint.iter().map(|r| view! { <div class="tint" style=rect_style(r, s)></div> }).collect_view()}
        {overlay.redactions.iter().enumerate().map(|(i, t)| redact_box_view(i, t, s)).collect_view()}
        {overlay.draft.map(|r| view! { <div class="area-draft" aria-hidden="true" style=rect_style(&r, s)></div> })}
        {move || flash().map(|r| view! { <div class="reveal-flash" aria-hidden="true" style=rect_style(&r.rect, s)></div> })}
        {overlay.editor.clone().map(|e| view! { <InlineEditor editor=e scale=s/> })}
        {overlay.selection.first().map(|r| view! { <SelectionBar left=r[0] * s top=r[1] * s/> })}
    }
}

/// A pending redaction box. With the Redact tool on, Tab reaches it like a signature
/// target: focusing picks it (as its panel row does), Delete removes it, the arrows move it.
fn redact_box_view(i: usize, t: &Target, s: f64) -> impl IntoView + use<> {
    let store = store();
    let r = t.rect;
    let id = t.id.clone();
    let selected = t.selected;
    let live = t.movable;
    let el = NodeRef::<html::Div>::new();
    // Keep the keyboard on the picked box when the overlay is drawn afresh.
    Effect::new(move |_| {
        if !(live && selected) {
            return;
        }
        let Some(el) = el.get() else { return };
        let active = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.active_element());
        let lost = active.as_ref().is_none_or(|a| {
            a.tag_name() == "BODY" || a.class_name().split_whitespace().any(|c| c == "viewport")
        });
        if lost {
            let _ = el.focus();
        }
    });
    let pick = {
        let id = id.clone();
        move |_| {
            if !selected {
                store.act(UiAction::Row {
                    list: "redactions".into(),
                    id: id.clone(),
                    action: "select".into(),
                });
            }
        }
    };
    let on_key = move |ev: web_sys::KeyboardEvent| {
        let key = ev.key();
        if ev.meta_key() || ev.ctrl_key() {
            return;
        }
        match key.as_str() {
            "Delete" | "Backspace" => {
                ev.prevent_default();
                ev.stop_propagation();
                store.act(UiAction::Row {
                    list: "redactions".into(),
                    id: id.clone(),
                    action: "remove".into(),
                });
            }
            "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" | "Escape" => {
                ev.prevent_default();
                ev.stop_propagation();
                store.act(UiAction::Key {
                    key,
                    shift: ev.shift_key(),
                    alt: ev.alt_key(),
                    meta: false,
                });
            }
            _ => {}
        }
    };
    let label = format!(
        "Area to redact {}{}",
        i + 1,
        if selected { ", selected" } else { "" }
    );
    view! {
        <div class="redact-box" node_ref=el class:peek=t.label == "peek" class:selected=selected style=rect_style(&r, s)
            role=if live { "button" } else { "img" } aria-label=label
            aria-pressed=live.then(|| selected.to_string())
            tabindex=live.then_some("0") on:focus=pick on:keydown=on_key></div>
        {(selected && live).then(|| handles(&r, s, &[]))}
    }
}

/// §3.2: selecting text shows a small bar with what can be done with it.
#[component]
fn SelectionBar(left: f64, top: f64) -> impl IntoView {
    let store = store();
    let has_text = crate::select(|s| s.view.selected_text.is_some());
    let mark = move |op: &'static str| {
        store.act(UiAction::Edit {
            op: op.into(),
            contents: Some(serde_json::json!("selection")),
        })
    };
    view! {
        <div class="selbar" role="toolbar" aria-label="Selected text" hidden=move || !has_text.get()
            style=format!("left:{}px;top:{}px", left.max(0.0), (top - 40.0).max(0.0))
            on:pointerdown=move |ev| ev.stop_propagation()>
            <button on:click=move |_| mark("highlight")>"Highlight"</button>
            <button on:click=move |_| mark("underline")>"Underline"</button>
            <button on:click=move |_| mark("strikeout")>"Strikethrough"</button>
            <button on:click=move |_| store.act(UiAction::Clipboard { what: "copy".into() })>"Copy"</button>
            <button on:click=move |_| store.act(UiAction::Edit { op: "redact-selection".into(), contents: None })>"Redact"</button>
        </div>
    }
}

/// The handles of a selected box, where the core hit-tests them (`HitNode::handle_box`):
/// corners and edge midpoints by name, and the move grip just outside the top-left corner.
/// Without names (redaction boxes) the four corners.
fn handles(r: &[f64; 4], s: f64, names: &[String]) -> impl IntoView + use<> {
    let (x0, y0, x1, y1) = (r[0] * s, r[1] * s, r[2] * s, r[3] * s);
    // How far from its dot a press catches a handle (tandem-app session/hit.rs
    // `handle_reach`): the cursor shows over that area.
    let reach = |side: f64| (side / 4.0).clamp(4.0, 7.0);
    let (rx, ry) = (reach(x1 - x0 - 2.0), reach(y1 - y0 - 2.0));
    let corners = ["nw", "ne", "sw", "se"].map(String::from);
    let names: Vec<String> = if names.is_empty() {
        corners.to_vec()
    } else {
        names.to_vec()
    };
    names
        .into_iter()
        .map(|name| {
            // `.handle` centres itself on its left/top (styles.css margin -5px).
            let (x, y, class) = if name == "grip" {
                (x0 - 8.0, y0 - 8.0, "handle grip")
            } else {
                let x = if name.contains('w') {
                    x0
                } else if name.contains('e') {
                    x1
                } else {
                    (x0 + x1) / 2.0
                };
                let y = if name.contains('n') {
                    y0
                } else if name.contains('s') {
                    y1
                } else {
                    (y0 + y1) / 2.0
                };
                (x, y, "handle")
            };
            view! { <div class=class data-handle=name aria-hidden="true" style=format!("left:{x}px;top:{y}px;--rx:{rx}px;--ry:{ry}px")></div> }
        })
        .collect_view()
}

/// A live target's gesture and the pointer of the press.
type Follow = (LiveBox, Memo<Option<Pointer>>);

/// A target on the page. A live one (being moved or resized) follows the pointer between the
/// core's answers.
fn target_view(t: &Target, s: f64, live: Option<Follow>) -> impl IntoView + use<> {
    let store = store();
    let class = format!("hit {}", t.kind);
    let movable = t.selected && t.movable;
    let r = t.rect;
    // Targets are reachable by VoiceOver with their names; a link is a link, the rest are
    // buttons (a click selects or edits them).
    let role = match t.kind.as_str() {
        "link" => "link",
        "group" | "crop" => "presentation",
        _ => "button",
    };
    // Tab reaches the things on a page one can act on; text runs (hundreds on a page) are
    // reached by VoiceOver, not by Tab.
    let tabindex = match t.kind.as_str() {
        "field" | "signature" | "textbox" | "link" | "object" | "annot" => Some("0"),
        "group" | "crop" => None,
        _ => Some("-1"),
    };
    let label = if t.label.is_empty() {
        match t.kind.as_str() {
            "field" => "Form field",
            "signature" => "Signature",
            "textbox" => "Text box",
            "object" => "Picture or drawing",
            "annot" => "Comment",
            "text" | "paragraph" => "Text",
            _ => "",
        }
        .to_string()
    } else {
        t.label.clone()
    };
    let el = NodeRef::<html::Div>::new();
    let focused = t.focused;
    // The overlay is drawn afresh after every change: keep the keyboard on the target the
    // core has focused (unless the person moved focus somewhere else meanwhile).
    Effect::new(move |_| {
        if !focused {
            return;
        }
        let Some(el) = el.get() else { return };
        let active = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.active_element());
        // A removed element leaves the focus on the body; a click on the page leaves it on
        // the page column.
        let lost = active.as_ref().is_none_or(|a| {
            a.tag_name() == "BODY" || a.class_name().split_whitespace().any(|c| c == "viewport")
        });
        if lost && !el.is_same_node(active.as_ref().map(|a| a.as_ref())) {
            let _ = el.focus();
        }
    });
    let id = t.id.clone();
    let on_focus = move |_| {
        if !focused {
            store.act(UiAction::Focus { target: id.clone() });
        }
    };
    // The keys a selected target answers to (fill.rs: arrows nudge, +/- resize, Delete
    // removes, Enter acts, Esc lets go); the rest go on to the window.
    let on_key = move |ev: web_sys::KeyboardEvent| {
        let key = ev.key();
        let ours = matches!(
            key.as_str(),
            "ArrowUp"
                | "ArrowDown"
                | "ArrowLeft"
                | "ArrowRight"
                | "Delete"
                | "Backspace"
                | "Enter"
                | " "
                | "+"
                | "="
                | "-"
                | "Escape"
        );
        if !ours || ev.meta_key() || ev.ctrl_key() {
            return;
        }
        ev.prevent_default();
        ev.stop_propagation();
        store.act(UiAction::Key {
            key,
            shift: ev.shift_key(),
            alt: ev.alt_key(),
            meta: false,
        });
    };
    let names = t.handles.clone();
    let is_live = live.is_some();
    let now = move || match &live {
        Some((lb, follow)) => follow.get().map_or(r, |p| follow_rect(r, lb, p)),
        None => r,
    };
    let now2 = now.clone();
    view! {
        <div class=class node_ref=el class:selected=t.selected class:focused=t.focused class:live=is_live
            style=move || rect_style(&now(), s) title=label.clone()
            role=role aria-label=label aria-pressed=(role == "button").then(|| t.selected.to_string())
            tabindex=tabindex on:focus=on_focus on:keydown=on_key></div>
        {move || movable.then(|| handles(&now2(), s, &names))}
    }
}

#[component]
fn InlineEditor(editor: EditorView, scale: f64) -> impl IntoView {
    let store = store();
    let value = RwSignal::new(editor.value.clone());
    let area = NodeRef::<html::Textarea>::new();
    Effect::new(move |_| {
        if let Some(a) = area.get() {
            let _ = a.focus();
        }
    });
    let r = editor.rect;
    // `font_px` is in page points: at the zoom drawn it is CSS px.
    let style = format!(
        "{};font-size:{}px",
        rect_style(&r, scale),
        (editor.font_px * scale).max(9.0)
    );
    let is_note = editor.kind == "note";
    // Text one writes on the page (a text box, new text, the page's own text being edited)
    // finishes on Return, as when one clicks away; Shift-Return is a new line.
    let is_writing = matches!(editor.kind.as_str(), "textbox" | "add" | "text");
    let multiline = editor.multiline;
    let label = if !editor.label.is_empty() {
        editor.label.clone()
    } else {
        match editor.kind.as_str() {
            "note" => "Note text",
            "field" => "Field value",
            _ => "Text",
        }
        .to_string()
    };
    let options = editor.options.clone();
    let input = move |ev| {
        let v = event_target_value(&ev);
        value.set(v.clone());
        store.act(UiAction::EditorInput { value: v });
    };
    let keydown = move |ev: web_sys::KeyboardEvent| {
        match ev.key().as_str() {
            "Escape" => {
                ev.prevent_default();
                ev.stop_propagation();
                store.act(UiAction::Key {
                    key: "Escape".into(),
                    shift: false,
                    alt: false,
                    meta: false,
                });
            }
            // A one-line editor and text one writes on the page commit on Return; a
            // multi-line field and a note commit on ⌘Return (Ctrl+Enter on Windows and
            // Linux), as the note's placeholder says.
            "Enter"
                if (store.primary(ev.meta_key(), ev.ctrl_key())
                    || is_writing
                    || !is_note && !multiline)
                    && !ev.shift_key()
                    // Return that picks an input method's candidate (Chinese, Japanese…)
                    // stays the input method's.
                    && !ev.is_composing() =>
            {
                ev.prevent_default();
                store.act(UiAction::EditorCommit);
            }
            "Tab" => {
                ev.prevent_default();
                store.act(UiAction::Key {
                    key: "Tab".into(),
                    shift: ev.shift_key(),
                    alt: false,
                    meta: false,
                });
            }
            _ => {}
        }
    };
    if !options.is_empty() {
        let label2 = label.clone();
        return view! {
            <select class="editor-inline" style=style aria-label=label2
                on:change=move |ev| { store.act(UiAction::EditorInput { value: event_target_value(&ev) }); store.act(UiAction::EditorCommit); }
                on:keydown=keydown>
                {options.into_iter().map(|(v, l)| { let selected = v == value.get_untracked(); view! { <option value=v.clone() selected=selected>{l}</option> } }).collect_view()}
            </select>
        }.into_any();
    }
    if is_note {
        view! {
            <div class="note-editor" style=format!("left:{}px;top:{}px", r[0] * scale, r[1] * scale)>
                <textarea node_ref=area aria-label=label placeholder=editor.placeholder.clone() prop:value=move || value.get()
                    on:input=input on:keydown=keydown on:blur=move |_| store.act(UiAction::EditorCommit)></textarea>
                <div class="row"><span class="spacer"></span><small>"Esc to finish"</small></div>
            </div>
        }.into_any()
    } else {
        view! {
            <textarea class="editor-inline" node_ref=area style=style aria-label=label placeholder=editor.placeholder.clone()
                prop:value=move || value.get() on:input=input on:keydown=keydown
                on:blur=move |_| store.act(UiAction::EditorCommit)></textarea>
        }.into_any()
    }
}

/// One thumbnail: drawn only once it scrolls into the sidebar (a 500-page file does not
/// render 500 pictures up front), kept in view when it is the current page.
#[component]
fn Thumb(
    index: usize,
    width: f64,
    height: f64,
    label: String,
    total: usize,
    generation: Memo<u64>,
    shown: Memo<usize>,
) -> impl IntoView {
    let store = store();
    let canvas = NodeRef::<html::Canvas>::new();
    let item = NodeRef::<html::Button>::new();
    let seen = RwSignal::new(false);
    let w = 132.0_f64;
    let h = (height / width.max(1.0)) * w;
    Effect::new(move |_| {
        let Some(el) = item.get() else { return };
        let callback = wasm_bindgen::closure::Closure::<dyn FnMut(js_sys::Array)>::new(
            move |entries: js_sys::Array| {
                let hit = entries.iter().any(|e| {
                    e.dyn_into::<web_sys::IntersectionObserverEntry>()
                        .map(|e| e.is_intersecting())
                        .unwrap_or(false)
                });
                if hit {
                    seen.set(true);
                }
            },
        );
        if let Ok(observer) = web_sys::IntersectionObserver::new(callback.as_ref().unchecked_ref())
        {
            observer.observe(&el);
            callback.forget();
            on_cleanup(move || observer.disconnect());
        }
    });
    Effect::new(move |_| {
        let g = generation.get();
        if !seen.get() {
            return;
        }
        let Some(c) = canvas.get() else { return };
        let s = w / width.max(1.0) * bridge::device_pixel_ratio();
        leptos::task::spawn_local(async move {
            if let Ok(px) = bridge::pixels("thumb", index, s, g).await {
                let _ = bridge::paint(&c, &px, w, h);
            }
        });
    });
    // The current page's thumbnail stays in view.
    Effect::new(move |_| {
        if shown.get() == index
            && let Some(el) = item.get()
        {
            let options = web_sys::ScrollIntoViewOptions::new();
            options.set_block(web_sys::ScrollLogicalPosition::Nearest);
            el.scroll_into_view_with_scroll_into_view_options(&options);
        }
    });
    let current = move || shown.get() == index;
    view! {
        <li>
            <button class="thumb" node_ref=item aria-current=move || current().then_some("page")
                tabindex=move || if current() { "0" } else { "-1" }
                aria-label=format!("Page {label} of {total}") on:click=move |_| store.act(UiAction::Goto { page: index })
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    let to = match ev.key().as_str() { "ArrowDown" | "ArrowRight" => Some(index + 1), "ArrowUp" | "ArrowLeft" => index.checked_sub(1), _ => None };
                    if let Some(to) = to {
                        ev.prevent_default();
                        ev.stop_propagation();
                        store.act(UiAction::Goto { page: to });
                        // Roving focus follows the current page.
                        if let Some(next) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.query_selector(&format!(".thumbs li:nth-child({}) button", to + 1)).ok().flatten())
                            && let Ok(next) = next.dyn_into::<web_sys::HtmlElement>()
                        {
                            let _ = next.focus();
                        }
                    }
                }>
                <canvas node_ref=canvas style=format!("width:{w}px;height:{h}px")></canvas>
                <span>{label.clone()}</span>
            </button>
        </li>
    }
}

#[component]
pub fn Thumbnails() -> impl IntoView {
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
    let shown = Memo::new(move |_| store.state.with(|s| s.view.page));
    view! {
        <ul class="thumbs" aria-label="Pages">
            {move || {
                let ps = pages.get();
                let total = ps.len();
                ps.into_iter().enumerate().map(|(i, p)| view! {
                    <Thumb index=i width=p.width height=p.height label=p.label.clone() total=total generation=generation shown=shown/>
                }).collect_view()
            }}
        </ul>
    }
}

#[component]
pub fn OutlineList() -> impl IntoView {
    let store = store();
    let rows = move || store.state.with(|s| s.view.outline.clone());
    // Collapsed entries (by id): their children are hidden.
    let collapsed = RwSignal::new(std::collections::BTreeSet::<String>::new());
    view! {
        {move || {
            let rows = rows();
            if rows.is_empty() {
                view! { <p class="empty" style="color:var(--muted);padding:16px;text-align:center">"This document has no outline. Add bookmarks from Insert ▸ Bookmark…"</p> }.into_any()
            } else {
                // Hide the rows under a collapsed entry.
                let shut = collapsed.get();
                let mut hidden_below: Option<usize> = None;
                let shown: Vec<_> = rows.into_iter().filter(|r| {
                    if let Some(d) = hidden_below {
                        if r.depth > d { return false; }
                        hidden_below = None;
                    }
                    if r.has_children && shut.contains(&r.id) {
                        hidden_below = Some(r.depth);
                    }
                    true
                }).collect();
                view! {
                    <ul class="outline-list" role="tree" aria-label="Outline">
                        {shown.into_iter().map(|r| {
                            let page = r.page;
                            let id = r.id.clone();
                            let toggle_id = r.id.clone();
                            let open = !shut.contains(&r.id);
                            let current = r.current;
                            view! {
                                <li role="treeitem" attr:aria-level=(r.depth + 1).to_string()
                                    aria-expanded=r.has_children.then(|| open.to_string())
                                    aria-current=current.then_some("location")
                                    style=format!("padding-left:{}px;display:flex;align-items:center;gap:2px", r.depth * 12)>
                                    {if r.has_children {
                                        view! {
                                            <button class="icon-btn" style="width:20px;height:20px;padding:0;flex:none"
                                                aria-label=if open { format!("Collapse {}", r.title) } else { format!("Expand {}", r.title) }
                                                on:click=move |_| collapsed.update(|c| { if !c.remove(&toggle_id) { c.insert(toggle_id.clone()); } })>
                                                {if open { "▾" } else { "▸" }}
                                            </button>
                                        }.into_any()
                                    } else {
                                        view! { <span style="width:20px;flex:none" aria-hidden="true"></span> }.into_any()
                                    }}
                                    <button class:current=current title=r.uri.clone().unwrap_or_default()
                                        style=if current { "font-weight:600;background:var(--selected, rgba(42,109,244,.12));border-radius:4px" } else { "" }
                                        on:click=move |_| {
                                            if let Some(p) = page { store.act(UiAction::Goto { page: p }); }
                                            else { store.act(UiAction::Row { list: "outline".into(), id: id.clone(), action: "open".into() }); }
                                        }>{r.title.clone()}</button>
                                </li>
                            }
                        }).collect_view()}
                    </ul>
                }.into_any()
            }
        }}
    }
}

#[cfg(test)]
mod tests {
    use super::{LiveBox, Pointer, follow_rect, gesture_rect};

    const R: [f64; 4] = [100.0, 200.0, 180.0, 240.0];

    fn live(handle: &str, aspect: &str) -> LiveBox {
        LiveBox {
            target: "1".into(),
            handle: handle.into(),
            initial: R,
            min: 8.0,
            aspect: aspect.into(),
            bounds: Some([0.0, 0.0, 612.0, 792.0]),
            hide: None,
        }
    }

    #[test]
    fn a_move_shifts_every_edge_and_stays_on_the_page() {
        assert_eq!(
            gesture_rect(&live("grip", ""), (5.0, -3.0), false),
            [105.0, 197.0, 185.0, 237.0]
        );
        assert_eq!(
            gesture_rect(&live("grip", ""), (-500.0, 0.0), false),
            [0.0, 200.0, 80.0, 240.0]
        );
    }

    #[test]
    fn a_handle_moves_its_own_edges_down_to_the_smallest_side() {
        assert_eq!(
            gesture_rect(&live("se", ""), (10.0, 6.0), false),
            [100.0, 200.0, 190.0, 246.0]
        );
        assert_eq!(
            gesture_rect(&live("w", ""), (-8.0, 50.0), false),
            [92.0, 200.0, 180.0, 240.0]
        );
        assert_eq!(
            gesture_rect(&live("e", ""), (-500.0, 0.0), false),
            [100.0, 200.0, 108.0, 240.0]
        );
        assert_eq!(
            gesture_rect(&live("e", ""), (900.0, 0.0), false),
            [100.0, 200.0, 612.0, 240.0]
        );
    }

    #[test]
    fn locked_corners_scale_sides_stretch_and_shift_frees_them() {
        // 80 × 40, scaled by the larger stretch.
        assert_eq!(
            gesture_rect(&live("se", "lock"), (20.0, 0.0), false),
            [100.0, 200.0, 200.0, 250.0]
        );
        assert_eq!(
            gesture_rect(&live("se", "lock"), (20.0, 0.0), true),
            [100.0, 200.0, 200.0, 240.0]
        );
        assert_eq!(
            gesture_rect(&live("s", "lock"), (0.0, 10.0), false),
            [100.0, 200.0, 180.0, 250.0]
        );
        // A signature's corner: the pointer's projection on the diagonal, Shift or not.
        let keep = gesture_rect(&live("se", "keep"), (16.0, 8.0), true);
        assert_eq!(keep, [100.0, 200.0, 196.0, 248.0]);
    }

    #[test]
    fn the_box_follows_on_from_the_cores_answer() {
        let lb = live("e", "");
        // The core answered for 10 pt and snapped the edge 2 pt further; the pointer is at 25.
        let p = Pointer {
            now: (25.0, 0.0),
            answered: (10.0, 0.0),
            shift: false,
        };
        assert_eq!(
            follow_rect([100.0, 200.0, 192.0, 240.0], &lb, p),
            [100.0, 200.0, 207.0, 240.0]
        );
        // Before the core's box has moved at all, its answer is for the press point.
        assert_eq!(follow_rect(R, &lb, p), [100.0, 200.0, 205.0, 240.0]);
    }
}
