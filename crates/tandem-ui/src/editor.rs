//! The editor window: top bar, tool strip, context bar, notices, and the
//! sidebar / pages / panel workspace (docs/rust-and-ux-plan.md §3.2).

use crate::icons::Icon;
use crate::store;
use crate::viewport::Viewport;
use leptos::html;
use leptos::prelude::*;
use tandem_protocol::ui::{Notice, UiAction};
use wasm_bindgen::JsCast;

/// The tool strip, in order: (id, label, icon, shortcut hint).
pub const TOOLS: &[(&str, &str, &str, &str)] = &[
    ("select", "Select", "select", "⌃⌘1"),
    ("highlight", "Highlight", "highlight", "⌃⌘2"),
    ("draw", "Draw", "draw", "⌃⌘3"),
    ("note", "Note", "note", "⌃⌘4"),
    ("text", "Text", "text", "⌃⌘5"),
    ("fill-sign", "Fill & Sign", "sign", "⌃⌘6"),
    ("redact", "Redact", "redact", "⌃⌘7"),
    ("pages", "Pages", "pages", "⌃⌘8"),
];

#[component]
pub fn Editor() -> impl IntoView {
    let sidebar_open = crate::select(|s| s.view.sidebar.open);
    let panel_open = crate::select(|s| s.view.panel.open);
    let pages_tool = crate::select(|s| s.view.tool.id == "pages");
    let comparing = crate::select(|s| s.view.compare.is_some());
    view! {
        <div class="app">
            <TopBar/>
            <ToolStrip/>
            <ContextBar/>
            <Notices/>
            <div class="workspace">
                {move || sidebar_open.get().then(|| view! { <Sidebar/> })}
                {move || if comparing.get() {
                    view! { <crate::screens::CompareView/> }.into_any()
                } else if pages_tool.get() {
                    view! { <crate::pages::PageGrid/> }.into_any()
                } else {
                    view! { <Viewport/> }.into_any()
                }}
                {move || panel_open.get().then(|| view! { <Panel/> })}
            </div>
        </div>
    }
}

#[component]
fn TopBar() -> impl IntoView {
    let store = store();
    let doc = move || store.state.with(|s| s.view.doc.clone());
    let name = move || doc().map(|d| d.name).unwrap_or_default();
    let save_state = move || doc().map(|d| d.save_state).unwrap_or_default();
    let can_undo = move || doc().is_some_and(|d| d.can_undo);
    let can_redo = move || doc().is_some_and(|d| d.can_redo);
    let undo_label = move || {
        doc()
            .and_then(|d| d.undo_label)
            .unwrap_or_else(|| "Undo".into())
    };
    let redo_label = move || {
        doc()
            .and_then(|d| d.redo_label)
            .unwrap_or_else(|| "Redo".into())
    };
    let can_save = move || doc().is_some_and(|d| d.can_save);
    let edited = move || {
        store.state.with(|s| {
            s.state
                .get("dirty")
                .and_then(|d| d.as_bool())
                .unwrap_or(false)
        })
    };
    let page = move || store.state.with(|s| s.view.page);
    let pages = move || doc().map(|d| d.page_count).unwrap_or(0);
    let search = move || store.state.with(|s| s.view.search.clone());
    let query = RwSignal::new(String::new());
    // Keep the field in step with the core (a menu or a script can search too).
    Effect::new(move |_| {
        let q = search().query;
        if q != query.get_untracked() {
            query.set(q);
        }
    });
    let page_input = RwSignal::new(String::new());
    Effect::new(move |_| page_input.set((page() + 1).to_string()));
    let sidebar_pressed = move || store.state.with(|s| s.view.sidebar.open.to_string());
    let panel_pressed = move || store.state.with(|s| s.view.panel.open.to_string());

    view! {
        <header class="topbar" data-tauri-drag-region>
            <button class="icon-btn pane" aria-label="Show or hide the sidebar" title="Sidebar (⌥⌘1)" aria-pressed=sidebar_pressed
                on:click=move |_| store.act(UiAction::Pane { which: "sidebar".into(), open: None, tab: None })>
                <Icon name="sidebar"/>
            </button>
            <button class="icon-btn" aria-label="Open a PDF" title="Open… (⌘O)" on:click=move |_| store.act(UiAction::Open { path: None })>
                <Icon name="open"/>
            </button>
            <div class="doc-title" data-tauri-drag-region>
                <strong title=name>{name}</strong>
                <small aria-live="polite">{save_state}</small>
            </div>
            <button class="icon-btn" aria-label=undo_label title=move || format!("{} (⌘Z)", undo_label()) disabled=move || !can_undo()
                on:click=move |_| store.act(UiAction::Edit { op: "undo".into(), contents: None })><Icon name="undo"/></button>
            <button class="icon-btn" aria-label=redo_label title=move || format!("{} (⇧⌘Z)", redo_label()) disabled=move || !can_redo()
                on:click=move |_| store.act(UiAction::Edit { op: "redo".into(), contents: None })><Icon name="redo"/></button>
            <div class="drag" data-tauri-drag-region></div>
            <span class="sep"></span>
            <form class="search" role="search" on:submit=move |ev| {
                ev.prevent_default();
                let s = search();
                if s.query == query.get_untracked() && s.count > 0 {
                    store.act(UiAction::SearchMove { by: 1 });
                } else {
                    store.act(UiAction::Search { query: query.get_untracked() });
                }
            }>
                <Icon name="search" class="sm"/>
                <input id="search-input" type="search" placeholder="Find in document" aria-label="Find in document" maxlength="256"
                    aria-keyshortcuts="Meta+F" autocomplete="off" spellcheck="false"
                    prop:value=move || query.get()
                    on:input=move |ev| query.set(event_target_value(&ev))
                    on:keydown=move |ev| {
                        if ev.key() == "Enter" && ev.shift_key() { ev.prevent_default(); store.act(UiAction::SearchMove { by: -1 }); }
                        if ev.key() == "Escape" && !query.get_untracked().is_empty() {
                            ev.prevent_default(); ev.stop_propagation(); query.set(String::new()); store.act(UiAction::Search { query: String::new() });
                        }
                    }/>
                <span class="count" aria-live="polite">{move || {
                    let s = search();
                    if s.busy { "Searching…".to_string() }
                    else if s.query.is_empty() { String::new() }
                    else if s.count == 0 { "No matches".to_string() }
                    else { format!("{} of {}{}", s.index.max(0) + 1, s.count, if s.truncated { "+" } else { "" }) }
                }}</span>
                <button type="button" class="icon-btn" aria-label="Previous match" title="Previous match (⇧↩)" hidden=move || search().query.is_empty() disabled=move || search().count == 0
                    on:click=move |_| store.act(UiAction::SearchMove { by: -1 })><Icon name="up" class="sm"/></button>
                <button type="button" class="icon-btn" aria-label="Next match" title="Next match (↩)" hidden=move || search().query.is_empty() disabled=move || search().count == 0
                    on:click=move |_| store.act(UiAction::SearchMove { by: 1 })><Icon name="down" class="sm"/></button>
            </form>
            <span class="sep"></span>
            <form class="pagebox" on:submit=move |ev| {
                ev.prevent_default();
                if let Ok(n) = page_input.get_untracked().trim().parse::<usize>() && n >= 1 {
                    store.act(UiAction::Goto { page: n - 1 });
                }
            }>
                <label class="sr-only" for="page-input">"Page"</label>
                <input id="page-input" class="input" inputmode="numeric" aria-label="Page number"
                    prop:value=move || page_input.get() on:input=move |ev| page_input.set(event_target_value(&ev))/>
                <span>{move || format!("/ {}", pages())}</span>
            </form>
            <ZoomControl/>
            <span class="sep"></span>
            <ShareMenu/>
            <button class=move || if edited() { "btn primary" } else { "btn" } disabled=move || !can_save() title="Save (⌘S)"
                on:click=move |_| store.act(UiAction::Save { how: "in-place".into() })>"Save"</button>
            <button class="icon-btn pane" aria-label="Show or hide the side panel" title="Panel (⌥⌘2)" aria-pressed=panel_pressed
                on:click=move |_| store.act(UiAction::Pane { which: "panel".into(), open: None, tab: None })>
                <Icon name="panel"/>
            </button>
        </header>
    }
}

/// A drop-down menu's keyboard and pointer behaviour (WAI-ARIA menu button): opening
/// focuses the checked item (or the first), ↑ ↓ Home End move, Esc and Tab close and
/// return to the button, a click anywhere else closes.
fn menu_behaviour(open: RwSignal<bool>, wrap: NodeRef<html::Div>, menu: NodeRef<html::Div>) {
    Effect::new(move |_| {
        if open.get()
            && let Some(m) = menu.get()
        {
            let items = menu_items(&m);
            let checked = items
                .iter()
                .position(|i| i.get_attribute("aria-checked").as_deref() == Some("true"));
            if let Some(item) = items.get(checked.unwrap_or(0)) {
                let _ = item.focus();
            }
        }
    });
    let outside = window_event_listener(leptos::ev::pointerdown, move |ev| {
        if !open.get_untracked() {
            return;
        }
        let inside = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Node>().ok())
            .zip(wrap.get_untracked())
            .is_some_and(|(t, w)| w.contains(Some(&t)));
        if !inside {
            open.set(false);
        }
    });
    on_cleanup(move || outside.remove());
}

fn menu_items(menu: &web_sys::HtmlElement) -> Vec<web_sys::HtmlElement> {
    let mut out = Vec::new();
    if let Ok(list) = menu.query_selector_all("[role^=menuitem]") {
        for i in 0..list.length() {
            if let Some(el) = list
                .item(i)
                .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
            {
                out.push(el);
            }
        }
    }
    out
}

/// Keys inside an open drop-down menu.
fn menu_keydown(ev: web_sys::KeyboardEvent, open: RwSignal<bool>, button: NodeRef<html::Button>) {
    let Some(menu) = ev
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok())
    else {
        return;
    };
    let items = menu_items(&menu);
    let active = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element());
    let at = items
        .iter()
        .position(|i| active.as_ref().is_some_and(|a| a.is_same_node(Some(i))));
    let n = items.len();
    let go = |i: usize| {
        if let Some(item) = items.get(i) {
            let _ = item.focus();
        }
    };
    match ev.key().as_str() {
        "ArrowDown" if n > 0 => {
            ev.prevent_default();
            ev.stop_propagation();
            go(at.map_or(0, |i| (i + 1) % n));
        }
        "ArrowUp" if n > 0 => {
            ev.prevent_default();
            ev.stop_propagation();
            go(at.map_or(n - 1, |i| (i + n - 1) % n));
        }
        "Home" if n > 0 => {
            ev.prevent_default();
            ev.stop_propagation();
            go(0);
        }
        "End" if n > 0 => {
            ev.prevent_default();
            ev.stop_propagation();
            go(n - 1);
        }
        "Escape" => {
            ev.prevent_default();
            ev.stop_propagation();
            open.set(false);
            if let Some(b) = button.get_untracked() {
                let _ = b.focus();
            }
        }
        "Tab" => open.set(false),
        _ => {}
    }
}

/// Zoom: − [ 125% ▾ ] +, with Fit width / Fit page / Actual size and steps in the menu.
#[component]
fn ZoomControl() -> impl IntoView {
    let store = store();
    let open = RwSignal::new(false);
    let wrap = NodeRef::<html::Div>::new();
    let menu = NodeRef::<html::Div>::new();
    let button = NodeRef::<html::Button>::new();
    menu_behaviour(open, wrap, menu);
    let label = move || {
        store.state.with(|s| match s.view.zoom.mode.as_str() {
            "fit-width" => "Fit width".to_string(),
            "fit-page" => "Fit page".to_string(),
            _ => format!("{}%", (s.view.zoom.scale * 100.0).round() as i64),
        })
    };
    let mode = move || store.state.with(|s| s.view.zoom.mode.clone());
    let fixed_at = move |p: u32| {
        store.state.with(|s| {
            !s.view.zoom.mode.starts_with("fit")
                && (s.view.zoom.scale * 100.0 - p as f64).abs() < 0.5
        })
    };
    let set = move |to: &'static str, scale: Option<f64>| {
        open.set(false);
        if let Some(b) = button.get_untracked() {
            let _ = b.focus();
        }
        store.act(UiAction::Zoom {
            to: to.into(),
            scale,
        });
    };
    view! {
        <div class="zoom" node_ref=wrap>
            <button class="icon-btn" aria-label="Zoom out" title="Zoom out (⌘−)" on:click=move |_| store.act(UiAction::Zoom { to: "out".into(), scale: None })><Icon name="minus" class="sm"/></button>
            <button class="btn quiet" node_ref=button aria-haspopup="menu" aria-expanded=move || open.get().to_string() aria-label=move || format!("Zoom: {}", label())
                on:click=move |_| open.update(|o| *o = !*o)
                on:keydown=move |ev| if ev.key() == "ArrowDown" { ev.prevent_default(); ev.stop_propagation(); open.set(true) }>{label}<Icon name="down" class="sm"/></button>
            <button class="icon-btn" aria-label="Zoom in" title="Zoom in (⌘+)" on:click=move |_| store.act(UiAction::Zoom { to: "in".into(), scale: None })><Icon name="plus" class="sm"/></button>
            {move || open.get().then(|| view! {
                <div class="menu" role="menu" aria-label="Zoom" node_ref=menu style="right:0;top:34px" on:keydown=move |ev| menu_keydown(ev, open, button)>
                    <button role="menuitemradio" tabindex="-1" aria-checked=move || (mode() == "fit-width").to_string() on:click=move |_| set("fit-width", None)>
                        <span class="mark" aria-hidden="true"></span>"Fit width"<span class="kbd">"⌘9"</span></button>
                    <button role="menuitemradio" tabindex="-1" aria-checked=move || (mode() == "fit-page").to_string() on:click=move |_| set("fit-page", None)>
                        <span class="mark" aria-hidden="true"></span>"Fit page"<span class="kbd">"⌘8"</span></button>
                    <button role="menuitem" tabindex="-1" on:click=move |_| set("actual", None)>
                        <span class="mark" aria-hidden="true"></span>"Actual size"<span class="kbd">"⌘0"</span></button>
                    <hr/>
                    {[50u32, 75, 100, 125, 150, 200, 300, 400].into_iter().map(|p| view! {
                        <button role="menuitemradio" tabindex="-1" aria-checked=move || fixed_at(p).to_string() on:click=move |_| set("fixed", Some(p as f64 / 100.0))>
                            <span class="mark" aria-hidden="true"></span>{format!("{p}%")}</button>
                    }).collect_view()}
                </div>
            })}
        </div>
    }
}

#[component]
fn ShareMenu() -> impl IntoView {
    let store = store();
    let open = RwSignal::new(false);
    let wrap = NodeRef::<html::Div>::new();
    let menu = NodeRef::<html::Div>::new();
    let button = NodeRef::<html::Button>::new();
    menu_behaviour(open, wrap, menu);
    let item = move |label: &'static str, action: UiAction| {
        view! {
            <button role="menuitem" tabindex="-1" on:click=move |_| {
                open.set(false);
                if let Some(b) = button.get_untracked() { let _ = b.focus(); }
                store.act(action.clone());
            }>{label}</button>
        }
    };
    view! {
        <div style="position:relative" node_ref=wrap>
            <button class="btn" node_ref=button aria-haspopup="menu" aria-expanded=move || open.get().to_string()
                on:click=move |_| open.update(|o| *o = !*o)
                on:keydown=move |ev| if ev.key() == "ArrowDown" { ev.prevent_default(); ev.stop_propagation(); open.set(true) }>
                <Icon name="share" class="sm"/>"Share"<Icon name="down" class="sm"/>
            </button>
            {move || open.get().then(|| view! {
                <div class="menu" role="menu" aria-label="Share" node_ref=menu style="right:0;top:34px" on:keydown=move |ev| menu_keydown(ev, open, button)>
                    {item("Save As…", UiAction::Save { how: "as".into() })}
                    {item("Save a Copy…", UiAction::Save { how: "copy".into() })}
                    <hr/>
                    {item("Export…", UiAction::Edit { op: "export".into(), contents: None })}
                    {item("Print…", UiAction::Print)}
                    <hr/>
                    {item("Protect with a Password…", UiAction::Edit { op: "security".into(), contents: None })}
                    {item("Compare with…", UiAction::Edit { op: "compare".into(), contents: None })}
                </div>
            })}
        </div>
    }
}

#[component]
fn ToolStrip() -> impl IntoView {
    let store = store();
    let current = move || store.state.with(|s| s.view.tool.id.clone());
    let availability = move |id: &str| {
        store
            .state
            .with(|s| s.view.tool.available.iter().find(|a| a.id == id).cloned())
    };
    view! {
        <nav class="toolstrip" aria-label="Tools">
            {TOOLS.iter().enumerate().map(|(i, (id, label, icon, keys))| {
                let id = id.to_string();
                let id2 = id.clone();
                let id3 = id.clone();
                let id4 = id.clone();
                let pressed = move || (current() == id2).to_string();
                let enabled = move || availability(&id3).is_none_or(|a| a.enabled);
                let title = move || {
                    let reason = availability(&id4).and_then(|a| a.reason);
                    match reason { Some(r) => format!("{label} — {r}"), None => format!("{label} ({keys})") }
                };
                let badge = { let id = id.clone(); move || availability(&id).and_then(|a| a.badge).filter(|b| *b > 0) };
                view! {
                    {(i == 1 || i == 5).then(|| view! { <span class="strip-sep" aria-hidden="true"></span> })}
                    <span class="tool-wrap">
                        <button class="tool" aria-pressed=pressed disabled=move || !enabled() title=title
                            on:click=move |_| store.act(UiAction::Tool { id: id.clone(), variant: None })>
                            <Icon name=*icon class="lg"/>
                            <span>{*label}</span>
                        </button>
                        {move || badge().map(|b| view! { <span class="badge" aria-label=format!("{b} pending")>{b}</span> })}
                    </span>
                }
            }).collect_view()}
        </nav>
    }
}

#[component]
fn ContextBar() -> impl IntoView {
    let store = store();
    let tool = crate::select(|s| s.view.tool.clone());
    view! {
        {move || {
            let t = tool.get();
            let chip = t.chip.clone();
            let id = t.id.clone();
            (id != "select" || chip.is_some()).then(|| view! {
                <div class="contextbar" role="toolbar" aria-label="Tool options">
                    {chip.map(|c| view! {
                        <span class="chip" role="status">{c}
                            <button on:click=move |_| store.act(UiAction::Key { key: "Escape".into(), shift: false, alt: false, meta: false })>"Done"</button>
                        </span>
                    })}
                    <ToolOptions id=id.clone()/>
                </div>
            })
        }}
    }
}

/// The armed tool's options (variant, colour, width).
#[component]
fn ToolOptions(id: String) -> impl IntoView {
    let store = store();
    let variant = move || {
        store
            .state
            .with(|s| s.view.tool.variant.clone().unwrap_or_default())
    };
    let seg = move |tool: &'static str, value: &'static str, label: &'static str| {
        view! {
            <button aria-pressed=move || (variant() == value).to_string()
                on:click=move |_| store.act(UiAction::Tool { id: tool.into(), variant: Some(value.into()) })>{label}</button>
        }
    };
    match id.as_str() {
        "highlight" => view! {
            <div class="segmented" role="group" aria-label="Kind of mark">
                {seg("highlight", "highlight", "Highlight")}
                {seg("highlight", "underline", "Underline")}
                {seg("highlight", "strike", "Strikethrough")}
            </div>
        }.into_any(),
        "draw" => view! {
            <div class="segmented" role="group" aria-label="Shape">
                {seg("draw", "pen", "Pen")}
                {seg("draw", "line", "Line")}
                {seg("draw", "rect", "Rectangle")}
                {seg("draw", "ellipse", "Ellipse")}
            </div>
        }.into_any(),
        "fill-sign" => view! {
            <div class="row">
                <button class="btn" on:click=move |_| store.act(UiAction::Edit { op: "sign-choose".into(), contents: None })>
                    <Icon name="sign" class="sm"/>"Sign…"
                </button>
                <span class="kbd" style="border:0">"Click a field to type in it. Tab moves to the next one."</span>
            </div>
        }.into_any(),
        "redact" => view! {
            <div class="row">
                <button class="btn" on:click=move |_| store.act(UiAction::Edit { op: "redact-search".into(), contents: None })>"Find text to redact…"</button>
            </div>
        }.into_any(),
        _ => ().into_any(),
    }
}

#[component]
fn Notices() -> impl IntoView {
    let store = store();
    let banner = crate::select(|s| s.view.banner.clone());
    let notice = crate::select(|s| s.view.notice.clone());
    let render = move |n: Notice, dismiss: bool| {
        let role = if n.kind == "error" { "alert" } else { "status" };
        view! {
            <div class=format!("notice {}", n.kind) role=role>
                <Icon name=match n.kind.as_str() { "error" | "warning" => "warning", "success" => "check", _ => "info" }/>
                <p>{n.text.clone()}</p>
                <div class="actions">
                    {n.actions.iter().cloned().map(|(id, label)| view! {
                        <button class="btn" on:click=move |_| store.act(UiAction::Notice { action: id.clone() })>{label}</button>
                    }).collect_view()}
                    {dismiss.then(|| view! {
                        <button class="icon-btn" aria-label="Dismiss" on:click=move |_| store.act(UiAction::Notice { action: "dismiss".into() })>
                            <Icon name="close" class="sm"/>
                        </button>
                    })}
                </div>
            </div>
        }
    };
    view! {
        {move || banner.get().map(|b| render(b, false))}
        {move || notice.get().map(|n| render(n, true))}
    }
}

#[component]
fn Sidebar() -> impl IntoView {
    let store = store();
    let tab = crate::select(|s| s.view.sidebar.tab.clone());
    let tab = move || tab.get();
    let set_tab = move |t: &'static str| {
        store.act(UiAction::Pane {
            which: "sidebar".into(),
            open: Some(true),
            tab: Some(t.into()),
        })
    };
    view! {
        <aside class="sidebar" aria-label="Pages and outline">
            <div class="tabs">
                <div class="segmented" role="tablist">
                    <button role="tab" aria-selected=move || (tab() != "outline").to_string() on:click=move |_| set_tab("pages")>"Pages"</button>
                    <button role="tab" aria-selected=move || (tab() == "outline").to_string() on:click=move |_| set_tab("outline")>"Outline"</button>
                </div>
            </div>
            <div class="scroll">
                {move || if tab() == "outline" { view! { <crate::viewport::OutlineList/> }.into_any() } else { view! { <crate::viewport::Thumbnails/> }.into_any() }}
            </div>
        </aside>
    }
}

#[component]
fn Panel() -> impl IntoView {
    view! { <crate::panel::PanelBody/> }
}
