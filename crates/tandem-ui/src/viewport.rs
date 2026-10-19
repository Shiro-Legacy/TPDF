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
use tandem_protocol::ui::{EditorView, PageOverlay, Target, UiAction};
use wasm_bindgen::JsCast;

/// Gap between pages and the column's padding, in CSS px (styles.css `.pages`).
const GAP: f64 = 16.0;
const PAD_TOP: f64 = 24.0;

#[component]
pub fn Viewport() -> impl IntoView {
    let store = store();
    let scroller = NodeRef::<html::Section>::new();
    let scroll_top = RwSignal::new(0.0_f64);
    let height = RwSignal::new(800.0_f64);
    let pages = Memo::new(move |_| store.state.with(|s| s.view.doc.as_ref().map(|d| d.pages.clone()).unwrap_or_default()));
    let scale = Memo::new(move |_| store.state.with(|s| s.view.zoom.scale.max(0.05)));
    let generation = Memo::new(move |_| store.state.with(|s| s.view.generation));
    let shown = Memo::new(move |_| store.state.with(|s| s.view.page));
    let tool = Memo::new(move |_| store.state.with(|s| s.view.tool.id.clone()));

    // Page tops in CSS px.
    let tops = Memo::new(move |_| {
        let s = scale.get();
        let mut y = PAD_TOP;
        pages.get().iter().map(|p| { let top = y; y += p.height * s + GAP; top }).collect::<Vec<f64>>()
    });
    let visible = Memo::new(move |_| {
        let (top, bottom) = (scroll_top.get() - 400.0, scroll_top.get() + height.get() + 400.0);
        let s = scale.get();
        let ps = pages.get();
        tops.get().iter().enumerate()
            .filter(|(i, t)| **t < bottom && **t + ps[*i].height * s > top)
            .map(|(i, _)| i).collect::<Vec<usize>>()
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
            // on a big screen (review: a 1660 px page at 1920).
            // The page column's padding (styles.css .pages: 24 top, 16 sides, 32 bottom) is
            // not room for the page.
            store.act(UiAction::Viewport { width: (w - 32.0).min(1180.0), height: h - 56.0 });
        }
    };
    // The viewport's size changes with the window and with the sidebar and panel: watch it.
    Effect::new(move |_| {
        let Some(el) = scroller.get() else { return };
        report_size();
        let callback = wasm_bindgen::closure::Closure::<dyn FnMut(js_sys::Array)>::new(move |_entries: js_sys::Array| report_size());
        if let Ok(observer) = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            observer.observe(&el);
            callback.forget();
            on_cleanup(move || observer.disconnect());
        }
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

    let on_scroll = move |_| {
        let Some(el) = scroller.get_untracked() else { return };
        let y = el.scroll_top() as f64;
        scroll_top.set(y);
        // The page most in view becomes the current page.
        let mid = y + el.client_height() as f64 * 0.35;
        let idx = tops.with_untracked(|t| t.iter().rposition(|top| *top <= mid)).unwrap_or(0);
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
        let base = zoom_pending.get_untracked().unwrap_or_else(|| scale.get_untracked());
        let factor = (-ev.delta_y() * 0.01).exp().clamp(0.5, 2.0);
        let next = (base * factor).clamp(0.1, 8.0);
        zoom_pending.set(Some(next));
        if let Some(el) = scroller.get_untracked() {
            // Keep the point under the pointer still.
            let rect = el.get_bounding_client_rect();
            let (px, py) = (ev.client_x() as f64 - rect.left(), ev.client_y() as f64 - rect.top());
            let (sx, sy) = (el.scroll_left() as f64 + px, el.scroll_top() as f64 + py);
            let k = next / base;
            el.set_scroll_left((sx * k - px) as i32);
            el.set_scroll_top((sy * k - py) as i32);
        }
        store.act(UiAction::Zoom { to: "fixed".into(), scale: Some(next) });
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
fn PageView(index: usize, width: f64, height: f64, scale: Memo<f64>, generation: Memo<u64>, visible: Memo<Vec<usize>>) -> impl IntoView {
    let store = store();
    let canvas = NodeRef::<html::Canvas>::new();
    let painted = RwSignal::new(None::<(u64, i64)>);
    let loading = RwSignal::new(true);
    let is_visible = Memo::new(move |_| visible.with(|v| v.contains(&index)));
    let css_w = move || width * scale.get();
    let css_h = move || height * scale.get();

    Effect::new(move |_| {
        if !is_visible.get() {
            return;
        }
        let dpr = bridge::device_pixel_ratio();
        let s = scale.get();
        let key = (generation.get(), (s * dpr * 1000.0).round() as i64);
        if painted.get_untracked() == Some(key) {
            return;
        }
        let Some(c) = canvas.get() else { return };
        loading.set(true);
        leptos::task::spawn_local(async move {
            let t0 = bridge::now_ms();
            match bridge::pixels("page", index, s * dpr, key.0).await {
                Ok(px) => match bridge::paint(&c, &px, width * s, height * s) {
                    Ok(()) => { bridge::log(format!("painted {index} in {:.0} ms", bridge::now_ms() - t0)); painted.set(Some(key)) },
                    Err(e) => bridge::log(format!("paint {index}: {e}")),
                },
                Err(e) => bridge::log(format!("pixels {index}: {e}")),
            }
            loading.set(false);
        });
    });

    let overlay = Memo::new(move |_| store.state.with(|s| s.view.overlays.iter().find(|o| o.page == index).cloned()));
    let dragging = RwSignal::new(false);
    let point = move |ev: &web_sys::PointerEvent| -> (f64, f64) {
        let target: web_sys::Element = ev.current_target().and_then(|t| t.dyn_into().ok()).expect("layer");
        let r = target.get_bounding_client_rect();
        let s = scale.get_untracked();
        ((ev.client_x() as f64 - r.left()) / s, (ev.client_y() as f64 - r.top()) / s)
    };
    let send = move |phase: &str, ev: &web_sys::PointerEvent| {
        let (x, y) = point(ev);
        store.act(UiAction::Pointer { phase: phase.into(), page: index, x, y, shift: ev.shift_key(), alt: ev.alt_key(), meta: ev.meta_key() });
    };

    view! {
        <div class="page" class:loading=move || loading.get() style=move || format!("width:{}px;height:{}px", css_w(), css_h())
            role="img" aria-label=format!("Page {}", index + 1)>
            <canvas node_ref=canvas aria-hidden="true"></canvas>
            <div class="layer"
                on:pointerdown=move |ev: web_sys::PointerEvent| {
                    if ev.button() != 0 { return; }
                    // Inputs inside the layer handle their own pointer.
                    if ev.target().and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok()).is_some_and(|t| t.tag_name() == "TEXTAREA" || t.tag_name() == "INPUT") { return; }
                    ev.prevent_default();
                    if let Some(el) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) { let _ = el.set_pointer_capture(ev.pointer_id()); }
                    dragging.set(true);
                    send("down", &ev);
                }
                on:pointermove=move |ev: web_sys::PointerEvent| if dragging.get_untracked() { send("move", &ev) }
                on:pointerup=move |ev: web_sys::PointerEvent| if dragging.get_untracked() { dragging.set(false); send("up", &ev) }
                on:pointercancel=move |ev: web_sys::PointerEvent| if dragging.get_untracked() { dragging.set(false); send("cancel", &ev) }>
                {move || overlay.get().map(|o| view! { <Overlay overlay=o scale=scale/> })}
            </div>
        </div>
    }
}

fn rect_style(r: &[f64; 4], s: f64) -> String {
    format!("left:{}px;top:{}px;width:{}px;height:{}px", r[0] * s, r[1] * s, (r[2] - r[0]).max(0.0) * s, (r[3] - r[1]).max(0.0) * s)
}

#[component]
fn Overlay(overlay: PageOverlay, scale: Memo<f64>) -> impl IntoView {
    let s = scale.get_untracked();
    let current = overlay.current_hit;
    view! {
        {overlay.hits.iter().enumerate().map(|(i, r)| view! {
            <div class="search-hit" class:current=Some(i) == current style=rect_style(r, s)></div>
        }).collect_view()}
        {overlay.selection.iter().map(|r| view! { <div class="text-sel" style=rect_style(r, s)></div> }).collect_view()}
        {overlay.targets.iter().map(|t| target_view(t, s)).collect_view()}
        {overlay.tint.iter().map(|r| view! { <div class="tint" style=rect_style(r, s)></div> }).collect_view()}
        {overlay.redactions.iter().map(|t| view! {
            <div class="redact-box" class:peek=t.label == "peek" style=rect_style(&t.rect, s)
                role="img" aria-label="Area to redact"></div>
        }).collect_view()}
        {overlay.editor.clone().map(|e| view! { <InlineEditor editor=e scale=s/> })}
        {overlay.selection.first().map(|r| view! { <SelectionBar left=r[0] * s top=r[1] * s/> })}
    }
}

/// §3.2: selecting text shows a small bar with what can be done with it.
#[component]
fn SelectionBar(left: f64, top: f64) -> impl IntoView {
    let store = store();
    let has_text = crate::select(|s| s.view.selected_text.is_some());
    let mark = move |op: &'static str| store.act(UiAction::Edit { op: op.into(), contents: Some(serde_json::json!("selection")) });
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

fn target_view(t: &Target, s: f64) -> impl IntoView + use<> {
    let class = format!("hit {}", t.kind);
    let handles = t.selected && t.movable;
    let r = t.rect;
    view! {
        <div class=class class:selected=t.selected style=rect_style(&r, s) title=t.label.clone()
            role=if t.kind == "link" { "link" } else { "presentation" } aria-label=t.label.clone()></div>
        {handles.then(|| {
            let (x0, y0, x1, y1) = (r[0] * s, r[1] * s, r[2] * s, r[3] * s);
            [(x0, y0), (x1, y0), (x0, y1), (x1, y1)].into_iter().map(|(x, y)| view! {
                <div class="handle" style=format!("left:{x}px;top:{y}px")></div>
            }).collect_view()
        })}
    }
}

#[component]
fn InlineEditor(editor: EditorView, scale: f64) -> impl IntoView {
    let store = store();
    let value = RwSignal::new(editor.value.clone());
    let area = NodeRef::<html::Textarea>::new();
    Effect::new(move |_| { if let Some(a) = area.get() { let _ = a.focus(); } });
    let r = editor.rect;
    let style = format!("{};font-size:{}px", rect_style(&r, scale), editor.font_px.max(9.0));
    let is_note = editor.kind == "note";
    let multiline = editor.multiline;
    let label = if !editor.label.is_empty() {
        editor.label.clone()
    } else {
        match editor.kind.as_str() { "note" => "Note text", "field" => "Field value", _ => "Text" }.to_string()
    };
    let options = editor.options.clone();
    let input = move |ev| {
        let v = event_target_value(&ev);
        value.set(v.clone());
        store.act(UiAction::EditorInput { value: v });
    };
    let keydown = move |ev: web_sys::KeyboardEvent| {
        match ev.key().as_str() {
            "Escape" => { ev.prevent_default(); ev.stop_propagation(); store.act(UiAction::Key { key: "Escape".into(), shift: false, alt: false, meta: false }); }
            // A one-line editor commits on Return; a multi-line one commits on ⌘Return.
            "Enter" if !is_note && (!multiline || ev.meta_key()) && !ev.shift_key() => { ev.prevent_default(); store.act(UiAction::EditorCommit); }
            "Tab" => { ev.prevent_default(); store.act(UiAction::Key { key: "Tab".into(), shift: ev.shift_key(), alt: false, meta: false }); }
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
                <div class="row"><span class="spacer"></span><small>"Esc to close"</small></div>
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
fn Thumb(index: usize, width: f64, height: f64, label: String, generation: Memo<u64>, shown: Memo<usize>) -> impl IntoView {
    let store = store();
    let canvas = NodeRef::<html::Canvas>::new();
    let item = NodeRef::<html::Button>::new();
    let seen = RwSignal::new(false);
    let w = 132.0_f64;
    let h = (height / width.max(1.0)) * w;
    Effect::new(move |_| {
        let Some(el) = item.get() else { return };
        let callback = wasm_bindgen::closure::Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
            let hit = entries.iter().any(|e| {
                e.dyn_into::<web_sys::IntersectionObserverEntry>().map(|e| e.is_intersecting()).unwrap_or(false)
            });
            if hit {
                seen.set(true);
            }
        });
        if let Ok(observer) = web_sys::IntersectionObserver::new(callback.as_ref().unchecked_ref()) {
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
        if shown.get() == index {
            if let Some(el) = item.get() {
                let options = web_sys::ScrollIntoViewOptions::new();
                options.set_block(web_sys::ScrollLogicalPosition::Nearest);
                el.scroll_into_view_with_scroll_into_view_options(&options);
            }
        }
    });
    let current = move || shown.get() == index;
    view! {
        <li>
            <button class="thumb" node_ref=item aria-current=move || current().then_some("page")
                tabindex=move || if current() { "0" } else { "-1" }
                aria-label=format!("Page {label}") on:click=move |_| store.act(UiAction::Goto { page: index })
                on:keydown=move |ev: web_sys::KeyboardEvent| {
                    let to = match ev.key().as_str() { "ArrowDown" | "ArrowRight" => Some(index + 1), "ArrowUp" | "ArrowLeft" => index.checked_sub(1), _ => None };
                    if let Some(to) = to {
                        ev.prevent_default();
                        ev.stop_propagation();
                        store.act(UiAction::Goto { page: to });
                        // Roving focus follows the current page.
                        if let Some(next) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.query_selector(&format!(".thumbs li:nth-child({}) button", to + 1)).ok().flatten()) {
                            if let Ok(next) = next.dyn_into::<web_sys::HtmlElement>() { let _ = next.focus(); }
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
    let pages = Memo::new(move |_| store.state.with(|s| s.view.doc.as_ref().map(|d| d.pages.clone()).unwrap_or_default()));
    let generation = Memo::new(move |_| store.state.with(|s| s.view.generation));
    let shown = Memo::new(move |_| store.state.with(|s| s.view.page));
    view! {
        <ul class="thumbs" aria-label="Pages">
            {move || pages.get().into_iter().enumerate().map(|(i, p)| view! {
                <Thumb index=i width=p.width height=p.height label=p.label.clone() generation=generation shown=shown/>
            }).collect_view()}
        </ul>
    }
}

#[component]
pub fn OutlineList() -> impl IntoView {
    let store = store();
    let rows = move || store.state.with(|s| s.view.outline.clone());
    view! {
        {move || {
            let rows = rows();
            if rows.is_empty() {
                view! { <p class="empty" style="color:var(--muted);padding:16px;text-align:center">"This document has no outline. Add bookmarks from Insert ▸ Bookmark…"</p> }.into_any()
            } else {
                view! {
                    <ul class="outline-list">
                        {rows.into_iter().map(|r| {
                            let page = r.page;
                            let id = r.id.clone();
                            view! {
                                <li style=format!("padding-left:{}px", r.depth * 12)>
                                    <button title=r.uri.clone().unwrap_or_default() on:click=move |_| {
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
