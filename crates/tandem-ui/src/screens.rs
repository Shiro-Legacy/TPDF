//! The password screen and the Compare view.

use crate::bridge;
use crate::icons::Icon;
use crate::{select, store};
use leptos::html;
use leptos::prelude::*;
use tandem_protocol::ui::UiAction;

/// §3.3 Open: "a clean password screen: 'x.pdf' is password-protected, a field and Open".
#[component]
pub fn PasswordScreen() -> impl IntoView {
    let store = store();
    let info = select(|s| s.view.password.clone().unwrap_or_default());
    let value = RwSignal::new(String::new());
    let input = NodeRef::<html::Input>::new();
    Effect::new(move |_| {
        if let Some(i) = input.get() {
            let _ = i.focus();
        }
    });
    let submit = move || {
        let password = value.get_untracked();
        value.set(String::new());
        store.act(UiAction::Unlock { password });
    };
    view! {
        <main class="start" aria-labelledby="pw-title">
            <div class="start-inner pw-inner">
                <div class="lock-badge"><Icon name="lock" class="lg"/></div>
                <h1 id="pw-title" class="pw-title">{move || format!("“{}” is password\u{2011}protected", info.get().name)}</h1>
                <p class="lede" style="font-size:14px">"Type its password to open it. Once it is open you can change it like any other file."</p>
                <form on:submit=move |ev| { ev.prevent_default(); submit(); }>
                    <label class="field">
                        <span>"Password"</span>
                        <input class="input" type="password" node_ref=input autocomplete="off" maxlength="1024"
                            aria-invalid=move || (!info.get().error.is_empty()).to_string() aria-describedby="pw-error"
                            prop:value=move || value.get() on:input=move |ev| value.set(event_target_value(&ev))/>
                    </label>
                    <p id="pw-error" role="alert" class="pw-error">{move || info.get().error}</p>
                    <p class="pw-hint" hidden=move || info.get().error.is_empty()>
                        "Passwords are case-sensitive: check Caps Lock and try again. If you do not know it, ask the person who sent you the file."
                    </p>
                    <div class="row">
                        <button type="button" class="btn" on:click=move |_| store.act(UiAction::Open { path: None })>"Open another file…"</button>
                        <span class="spacer"></span>
                        <button type="submit" class="btn primary">"Open"</button>
                    </div>
                </form>
            </div>
        </main>
    }
}

fn paint_into(canvas: NodeRef<html::Canvas>, kind: &'static str, generation: u64, css_width: f64) {
    leptos::task::spawn_local(async move {
        let Some(c) = canvas.get_untracked() else {
            return;
        };
        if let Ok(px) = bridge::pixels(kind, 0, 1.0, generation).await
            && px.width > 0
        {
            let h = css_width * px.height as f64 / px.width as f64;
            let _ = bridge::paint(&c, &px, css_width, h);
        }
    });
}

/// Compare with another PDF (⌥⌘K): the two pages side by side, the pixels that differ
/// in red, and the lines found on one side only.
#[component]
pub fn CompareView() -> impl IntoView {
    let store = store();
    let cmp = select(|s| s.view.compare.clone().unwrap_or_default());
    let a = NodeRef::<html::Canvas>::new();
    let b = NodeRef::<html::Canvas>::new();
    let o = NodeRef::<html::Canvas>::new();
    Effect::new(move |_| {
        let g = cmp.get().generation;
        if a.get().is_some() {
            paint_into(a, "compare-a", g, 420.0);
            paint_into(b, "compare-b", g, 420.0);
            paint_into(o, "compare-overlay", g, 420.0);
        }
    });
    view! {
        <section class="compare" aria-label="Compare">
            <div class="contextbar" role="toolbar" aria-label="Compare">
                <span class="chip plain">{move || format!("Comparing with {}", cmp.get().name_b)}</span>
                <label class="check"><input type="checkbox" prop:checked=move || cmp.get().show_diff
                    on:change=move |ev| store.act(UiAction::Check { id: "compare-show-diff".into(), checked: event_target_checked(&ev) })/>"Show differences in red"</label>
                <label class="row">"Page of the other file"
                    <input class="input" type="number" min="1" max=move || cmp.get().pages_b.max(1).to_string() style="width:64px" prop:value=move || (cmp.get().page_b + 1).to_string()
                        on:change=move |ev| store.act(UiAction::Input { id: "compare-page-b".into(), value: event_target_value(&ev) })/>
                    {move || format!("of {}", cmp.get().pages_b)}
                </label>
                <span class="spacer"></span>
                <button class="btn" on:click=move |_| store.act(UiAction::Click { id: "compare-close".into() })>"Close compare"</button>
            </div>
            <div class="compare-body">
                <p class="selectable compare-summary" role="status">{move || cmp.get().summary}</p>
                <div class="compare-panes">
                    <figure>
                        <figcaption>{move || format!("{} · page {}", cmp.get().name_a, cmp.get().page_a + 1)}</figcaption>
                        <canvas node_ref=a class="paper"></canvas>
                    </figure>
                    <figure>
                        <figcaption>{move || format!("{} · page {}", cmp.get().name_b, cmp.get().page_b + 1)}</figcaption>
                        <div class="compare-b">
                            <canvas node_ref=b class="paper"></canvas>
                            <canvas node_ref=o class="overlay" hidden=move || !cmp.get().show_diff></canvas>
                        </div>
                    </figure>
                </div>
                {move || {
                    let rows = cmp.get().rows;
                    if rows.is_empty() {
                        view! { <p class="empty" style="color:var(--muted)">"The text of these two pages is the same."</p> }.into_any()
                    } else {
                        view! {
                            <table class="data selectable" aria-label="Lines only on one side">
                                <thead><tr><th scope="col">"Where"</th><th scope="col">"Line"</th></tr></thead>
                                <tbody>{rows.into_iter().map(|r| view! { <tr><td>{r.detail}</td><td>{r.title}</td></tr> }).collect_view()}</tbody>
                            </table>
                        }.into_any()
                    }
                }}
            </div>
        </section>
    }
}
