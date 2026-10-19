//! Dialogs. Each one is a view of the ported dialog in the core (its
//! elements by id, `DialogView::nodes`): typing sends `Input`/`Check`,
//! buttons send `Click`, Escape sends `DialogCancel`. The wording is new and
//! plain (UX audit X7); the logic behind every button is the Electron app's.

use crate::icons::Icon;
use crate::store;
use leptos::prelude::*;
use tandem_protocol::ui::{DialogView, NodeView, UiAction};
use wasm_bindgen::JsCast;

/// The open dialog, read reactively.
#[derive(Clone, Copy)]
struct Dlg(Memo<Option<DialogView>>);

impl Dlg {
    fn node(self, id: &str) -> NodeView {
        self.0.with(|d| {
            d.as_ref()
                .and_then(|d| d.nodes.get(id).cloned())
                .unwrap_or_default()
        })
    }
    fn text(self, id: &'static str) -> impl Fn() -> String + Copy + 'static {
        move || self.node(id).text
    }
}

fn click(id: &'static str) -> impl Fn(leptos::ev::MouseEvent) + Copy {
    move |_| store().act(UiAction::Click { id: id.into() })
}

/// A labelled text field bound to an element.
#[component]
fn Field(
    dlg: Dlg,
    id: &'static str,
    label: &'static str,
    #[prop(optional)] hint: &'static str,
    #[prop(optional)] kind: &'static str,
    #[prop(optional)] placeholder: &'static str,
    #[prop(optional)] autofocus: bool,
) -> impl IntoView {
    let store = store();
    let value = move || dlg.node(id).value;
    let kind = if kind.is_empty() { "text" } else { kind };
    let hint_id = format!("{id}-hint");
    view! {
        <label class="field" class:hidden=move || dlg.node(id).hidden>
            <span>{label}</span>
            <input class="input" type=kind id=format!("ui-{id}") placeholder=placeholder autocomplete="off" spellcheck="false"
                autofocus=autofocus aria-describedby=(!hint.is_empty()).then(|| hint_id.clone())
                disabled=move || dlg.node(id).disabled
                prop:value=value
                on:input=move |ev| store.act(UiAction::Input { id: id.into(), value: event_target_value(&ev) })/>
            {(!hint.is_empty()).then(|| view! { <small id=hint_id.clone()>{hint}</small> })}
        </label>
    }
}

#[component]
fn Area(dlg: Dlg, id: &'static str, label: &'static str) -> impl IntoView {
    let store = store();
    view! {
        <label class="field">
            <span>{label}</span>
            <textarea class="input" autofocus=true rows="6" prop:value=move || dlg.node(id).value
                on:input=move |ev| store.act(UiAction::Input { id: id.into(), value: event_target_value(&ev) })></textarea>
        </label>
    }
}

#[component]
fn Choice(
    dlg: Dlg,
    id: &'static str,
    label: &'static str,
    options: &'static [(&'static str, &'static str)],
    /// Hide the choice while the core disables it (it does not apply, e.g. Position for a
    /// watermark): a greyed control that never matters is noise.
    #[prop(optional)]
    hide_disabled: bool,
) -> impl IntoView {
    let store = store();
    view! {
        <label class="field" class:hidden=move || { let n = dlg.node(id); n.hidden || (hide_disabled && n.disabled) }>
            <span>{label}</span>
            <select class="input" disabled=move || dlg.node(id).disabled
                on:change=move |ev| store.act(UiAction::Input { id: id.into(), value: event_target_value(&ev) })>
                {options.iter().map(|(v, l)| view! { <option value=*v selected=move || dlg.node(id).value == *v>{*l}</option> }).collect_view()}
            </select>
        </label>
    }
}

#[component]
fn Check(
    dlg: Dlg,
    id: &'static str,
    label: &'static str,
    #[prop(optional)] hint: &'static str,
) -> impl IntoView {
    let store = store();
    view! {
        <label class="check" hidden=move || dlg.node(id).hidden>
            <input type="checkbox" prop:checked=move || dlg.node(id).checked disabled=move || dlg.node(id).disabled
                on:change=move |ev| store.act(UiAction::Check { id: id.into(), checked: event_target_checked(&ev) })/>
            <span>{label}{(!hint.is_empty()).then(|| view! { <br/><small style="color:var(--muted)">{hint}</small> })}</span>
        </label>
    }
}

#[component]
fn Btn(
    dlg: Dlg,
    id: &'static str,
    label: &'static str,
    #[prop(optional)] class: &'static str,
) -> impl IntoView {
    let class = if class.is_empty() {
        "btn".to_string()
    } else {
        format!("btn {class}")
    };
    view! {
        <button class=class hidden=move || dlg.node(id).hidden disabled=move || dlg.node(id).disabled on:click=click(id)>{label}</button>
    }
}

/// A status line from the core: a refusal or failure is drawn as an error and announced
/// at once (role=alert); anything else is a plain status. The core does not mark which is
/// which, so the wording decides (`plain` turns this off for summaries such as "0 failed").
#[component]
fn Status(dlg: Dlg, id: &'static str, #[prop(optional)] plain: bool) -> impl IntoView {
    view! {
        {move || {
            let t = dlg.node(id).text;
            (!t.is_empty()).then(|| {
                if !plain && looks_like_error(&t) {
                    view! { <p role="alert" class="selectable dialog-error"><Icon name="warning" class="sm"/><span>{t}</span></p> }.into_any()
                } else {
                    view! { <p role="status" class="selectable">{t}</p> }.into_any()
                }
            })
        }}
    }
}

/// Whether a dialog's status text reports a problem (see [`Status`]).
fn looks_like_error(text: &str) -> bool {
    let t = text.to_lowercase();
    [
        "could not",
        "couldn't",
        "can't",
        "cannot",
        "not valid",
        "invalid",
        "must ",
        "wrong",
        "incorrect",
        "failed",
        "isn't",
        "is not",
        "doesn't",
        "does not",
        "too large",
        "too big",
        "not allowed",
        "refused",
        "no certificate",
        "choose a",
        "choose the",
        "first choose",
        "unable",
        "error",
        "expired",
        "not a ",
    ]
    .iter()
    .any(|w| t.contains(w))
}

#[component]
pub fn Dialogs() -> impl IntoView {
    let store = store();
    let dialog = Memo::new(move |_| store.state.with(|s| s.view.dialog.clone()));
    let dlg = Dlg(dialog);
    let id = Memo::new(move |_| dialog.with(|d| d.as_ref().map(|d| d.id.clone())));
    view! {
        {move || id.get().map(|id| {
            let wide = matches!(id.as_str(), "help-dialog" | "verify-dialog" | "batch-dialog" | "redact-dialog");
            let cancel_id = id.clone();
            let body = dialog_body(&id, dlg);
            let panel = NodeRef::<leptos::html::Div>::new();
            // Focus moves into the dialog when it opens: its first field, else the dialog itself.
            Effect::new(move |_| {
                let Some(el) = panel.get() else { return };
                let first = el.query_selector("input:not([type=hidden]):not([disabled]), textarea, select").ok().flatten();
                match first.and_then(|f| f.dyn_into::<web_sys::HtmlElement>().ok()) {
                    Some(f) => { let _ = f.focus(); }
                    None => { let _ = el.focus(); }
                }
            });
            view! {
                <div class="scrim" on:keydown=move |ev: web_sys::KeyboardEvent| {
                    if ev.key() == "Escape" {
                        ev.prevent_default();
                        ev.stop_propagation();
                        store.act(UiAction::DialogCancel { id: cancel_id.clone() });
                    } else if ev.key() == "Tab" {
                        // Keep Tab inside the dialog.
                        if let Some(el) = panel.get_untracked() {
                            let list = el.query_selector_all("button:not([disabled]):not([hidden]), input:not([disabled]), select, textarea, summary, [tabindex='0']").ok();
                            let n = list.as_ref().map_or(0, |l| l.length());
                            if n > 0 {
                                let first = list.as_ref().and_then(|l| l.item(0));
                                let last = list.as_ref().and_then(|l| l.item(n - 1));
                                let active = web_sys::window().and_then(|w| w.document()).and_then(|d| d.active_element());
                                let at = |node: &Option<web_sys::Node>| node.as_ref().zip(active.as_ref()).is_some_and(|(a, b)| a.is_same_node(Some(b.as_ref())));
                                let wrap = if ev.shift_key() && at(&first) { last } else if !ev.shift_key() && at(&last) { first } else { None };
                                if let Some(target) = wrap.and_then(|t| t.dyn_into::<web_sys::HtmlElement>().ok()) {
                                    ev.prevent_default();
                                    let _ = target.focus();
                                }
                            }
                        }
                    }
                }>
                    <div class="dialog" class:wide=wide role="dialog" aria-modal="true" aria-labelledby="dialog-title" tabindex="-1" node_ref=panel>{body}</div>
                </div>
            }
        })}
    }
}

fn title(text: impl IntoView) -> impl IntoView {
    view! { <h2 id="dialog-title">{text}</h2> }
}

fn dialog_body(id: &str, d: Dlg) -> AnyView {
    match id {
        "note-dialog" => view! {
            {title("Note")}
            <Area dlg=d id="note-text" label="Note text"/>
            <div class="actions"><Btn dlg=d id="note-cancel" label="Cancel"/><Btn dlg=d id="note-save" label="Save" class="primary"/></div>
        }.into_any(),

        "help-dialog" => view! {
            // Close beside the title (the help is long; a second Close ends it).
            <div class="dialog-head">{title("Tandem PDF Help")}<Btn dlg=d id="help-close" label="Close"/></div>
            <Help/>
            <div class="actions"><Btn dlg=d id="help-close" label="Close" class="primary"/></div>
        }.into_any(),

        "redact-dialog" => view! {
            {title(d.text("redact-heading"))}
            <p>{d.text("redact-explain")}</p>
            {move || { let t = d.node("redact-shared").text; (!t.is_empty()).then(|| view! { <p class="notice warning" style="border-radius:9px">{t}</p> }) }}
            <label class="check">
                <input type="checkbox" prop:checked=move || d.node("redact-hidden").checked
                    on:change=move |ev| store().act(UiAction::Check { id: "redact-hidden".into(), checked: event_target_checked(&ev) })/>
                <span>"Also remove hidden information, other comments and form entries (author, document properties, attachments, bookmarks…) "
                    <span class="recommended">"Recommended"</span></span>
            </label>
            <details>
                <summary>"Details"</summary>
                <div style="padding:4px 0 0 24px">
                    <Check dlg=d id="redact-sanitize-info" label="Author, title and other document properties"/>
                    <Check dlg=d id="redact-sanitize-xmp" label="Extra document details saved by other apps"/>
                    <Check dlg=d id="redact-sanitize-attachments" label="Attached files"/>
                    <Check dlg=d id="redact-sanitize-outlines" label="Bookmarks"/>
                    <Check dlg=d id="redact-sanitize-annots" label="Other comments and notes, on every page"/>
                    <Check dlg=d id="redact-sanitize-links" label="Web and file addresses in links, and scripts that run when the file opens"/>
                    <Check dlg=d id="redact-sanitize-fields" label="Everything typed into form fields"/>
                    <Check dlg=d id="redact-sanitize-layers" label="Layer names"/>
                    <Check dlg=d id="redact-sanitize-struct" label="Descriptions written for screen readers"/>
                    <p>{d.text("redact-kept")}</p>
                    <Rows dlg=d head=&["Page", "Boxes"]/>
                </div>
            </details>
            <p style="color:var(--muted);font-size:12px">{d.text("redact-limit")}</p>
            <div class="actions"><Btn dlg=d id="redact-cancel" label="Cancel"/><Btn dlg=d id="redact-apply" label="Redact and save as…" class="redact"/></div>
        }.into_any(),

        "redact-find-dialog" => view! {
            {title("Find text to redact")}
            <p>"Adds a box over every place this text appears on the pages. Text inside pictures, form fields and comments is not found, so look over each page or draw boxes there."</p>
            <Field dlg=d id="redact-search-text" label="Text to find" kind="search" autofocus=true/>
            <div class="actions"><Btn dlg=d id="redact-find-cancel" label="Cancel"/><Btn dlg=d id="redact-search-go" label="Add boxes" class="primary"/></div>
        }.into_any(),

        "redact-pending-dialog" => view! {
            {title(d.text("redact-pending-heading"))}
            <p>{d.text("redact-pending-text")}</p>
            <div class="actions">
                <Btn dlg=d id="redact-pending-cancel" label="Cancel"/>
                <Btn dlg=d id="redact-pending-marks" label="Save marks only"/>
                <Btn dlg=d id="redact-pending-now" label="Redact now…" class="primary"/>
            </div>
        }.into_any(),

        "redact-ask-dialog" => view! {
            {title(d.text("redact-ask-heading"))}
            <p>{d.text("redact-ask-text")}</p>
            <div class="actions">
                <button class="btn" on:click=click("redact-ask-cancel")>"Cancel"</button>
                <button class="btn" hidden=move || d.node("redact-ask-alt").hidden on:click=click("redact-ask-alt")>{d.text("redact-ask-alt")}</button>
                <button class="btn primary" on:click=click("redact-ask-yes")>{d.text("redact-ask-yes")}</button>
            </div>
        }.into_any(),

        "stamp-icon-dialog" => view! {
            {title("Add a stamp")}
            <p>"Choose the stamp, then it is placed on the page; drag it where you want it."</p>
            <div role="radiogroup" aria-label="Stamp" class="grid2">
                {[("Draft", "Draft"), ("Approved", "Approved"), ("Confidential", "Confidential"), ("Final", "Final"), ("NotApproved", "Not approved"), ("Void", "Void")]
                    .into_iter().map(|(v, l)| {
                        let nid = format!("radio:stamp-icon:{v}");
                        let nid2 = nid.clone();
                        view! {
                            <label class="check"><input type="radio" name="stamp-icon" prop:checked=move || d.node(&nid).checked
                                on:change=move |_| store().act(UiAction::Check { id: nid2.clone(), checked: true })/>{l}</label>
                        }
                    }).collect_view()}
            </div>
            <div class="actions"><Btn dlg=d id="stamp-icon-cancel" label="Cancel"/><Btn dlg=d id="stamp-icon-save" label="Add stamp" class="primary"/></div>
        }.into_any(),

        "sigpad-dialog" => view! { <SignaturePad dlg=d/> }.into_any(),

        "stamp-dialog" => view! {
            {title(d.text("stamp-dialog-heading"))}
            <Choice dlg=d id="stamp-kind" label="Add" options=&[("watermark", "A watermark across each page"), ("header", "A header at the top"), ("footer", "A footer at the bottom")]/>
            <Field dlg=d id="stamp-text" label="Text" placeholder="Confidential — page {page} of {pages}"
                hint="You can use {page} (page number), {pages} (number of pages), {label} (page label) and {bates} (a running number that continues across documents)."/>
            <Choice dlg=d id="stamp-align" label="Position" options=&[("left", "Left"), ("right", "Right")] hide_disabled=true/>
            // The running-number fields only matter when the text uses {bates}.
            <div class="grid2" hidden=move || !d.node("stamp-text").value.contains("{bates}")>
                <Field dlg=d id="stamp-bates-start" label="Running number starts at" hint="Only used for {bates}."/>
                <Field dlg=d id="stamp-bates-pad" label="Number of digits" hint="6 gives 000001."/>
            </div>
            <Field dlg=d id="stamp-pages" label="Pages" placeholder="All pages" hint="Leave blank for every page, or list pages like 1-3, 5."/>
            <div class="actions"><Btn dlg=d id="stamp-cancel" label="Cancel"/><Btn dlg=d id="stamp-save" label="Add" class="primary"/></div>
        }.into_any(),

        "labels-dialog" => view! {
            {title("Page labels")}
            <p>"Page labels are the page numbers a reader shows, such as i, ii, iii for a preface."</p>
            <div class="grid2">
                <Field dlg=d id="labels-from" label="Starting at page" kind="number"/>
                <Choice dlg=d id="labels-style" label="Numbering" options=&[("D", "1, 2, 3"), ("r", "i, ii, iii"), ("R", "I, II, III"), ("a", "a, b, c"), ("A", "A, B, C"), ("", "No numbers")]/>
                <Field dlg=d id="labels-prefix" label="Text before the number" placeholder="For example A-"/>
                <Field dlg=d id="labels-start" label="First number" kind="number"/>
            </div>
            <div class="actions">
                <Btn dlg=d id="labels-clear" label="Remove labels from this page on"/>
                <span class="spacer"></span>
                <Btn dlg=d id="labels-cancel" label="Cancel"/><Btn dlg=d id="labels-save" label="Set labels" class="primary"/>
            </div>
        }.into_any(),

        "export-dialog" => view! {
            {title("Export")}
            <Choice dlg=d id="export-kind" label="Export" options=&[("png", "This page as a PNG picture"), ("jpeg", "This page as a JPEG picture"), ("txt", "All the text, as a text file")]/>
            <Choice dlg=d id="export-dpi" label="Picture quality" options=&[("72", "Screen (72 dpi)"), ("150", "Standard (150 dpi)"), ("300", "High, for printing (300 dpi)")] hide_disabled=true/>
            <div class="actions"><Btn dlg=d id="export-cancel" label="Cancel"/><Btn dlg=d id="export-start" label="Export…" class="primary"/></div>
        }.into_any(),

        "insert-dialog" => view! {
            {title("Insert pages")}
            <p>{d.text("insert-source")}</p>
            <InsertWhere dlg=d/>
            <Field dlg=d id="insert-range" label="Pages to take from that file" placeholder="All pages" hint="Leave blank for all, or list pages like 1-3, 5."/>
            <div class="actions"><Btn dlg=d id="insert-cancel" label="Cancel"/><Btn dlg=d id="insert-start" label="Insert" class="primary"/></div>
        }.into_any(),

        "extract-dialog" => view! {
            {title("Extract pages")}
            <p>"Saves the pages you choose as a new PDF. This document does not change."</p>
            <Field dlg=d id="extract-range" label="Pages" placeholder="For example 1-3, 5" autofocus=true/>
            <div class="actions"><Btn dlg=d id="extract-cancel" label="Cancel"/><Btn dlg=d id="extract-start" label="Extract…" class="primary"/></div>
        }.into_any(),

        "bookmark-dialog" => view! {
            {title(d.text("bookmark-dialog-heading"))}
            <Field dlg=d id="bookmark-title" label="Title" autofocus=true/>
            <div class="actions"><Btn dlg=d id="bookmark-cancel" label="Cancel"/><Btn dlg=d id="bookmark-save" label="Save" class="primary"/></div>
        }.into_any(),

        "link-dialog" => view! {
            {title(d.text("link-dialog-heading"))}
            <Choice dlg=d id="link-kind" label="Link to" options=&[("page", "A page in this document"), ("uri", "A web address")]/>
            {move || if d.node("link-kind").value == "uri" {
                view! { <Field dlg=d id="link-target" label="Web address" placeholder="https://" autofocus=true/> }.into_any()
            } else {
                view! { <Field dlg=d id="link-target" label="Page number" kind="number" autofocus=true/> }.into_any()
            }}
            <div class="actions"><Btn dlg=d id="link-cancel" label="Cancel"/><Btn dlg=d id="link-save" label="Save" class="primary"/></div>
        }.into_any(),

        "print-dialog" => view! {
            {title("Print")}
            <Field dlg=d id="print-range" label="Pages" placeholder="All pages" hint="Leave blank to print everything, or list pages like 1-3, 5."/>
            <Status dlg=d id="print-status"/>
            <div class="actions"><Btn dlg=d id="print-cancel" label="Cancel"/><Btn dlg=d id="print-start" label="Print…" class="primary"/></div>
        }.into_any(),

        "verify-dialog" => view! {
            {title("Signatures")}
            <p class="selectable">{d.text("verify-summary")}</p>
            {move || { let t = d.node("verify-certified").text; (!t.is_empty()).then(|| view! { <p>{t}</p> }) }}
            <VerifyRows dlg=d/>
            <h3 style="font-size:13px;margin:16px 0 4px">"Certificates you trust"</h3>
            <TrustedList dlg=d/>
            <div class="actions">
                <Btn dlg=d id="verify-trust-add" label="Trust a certificate…"/>
                <Btn dlg=d id="verify-trust-clear" label="Forget trusted certificates"/>
                <span class="spacer"></span>
                <Btn dlg=d id="verify-refresh" label="Check again"/>
                <Btn dlg=d id="verify-close" label="Done" class="primary"/>
            </div>
        }.into_any(),

        "batch-dialog" => view! {
            {title("Add to every PDF in a folder")}
            <p>"Adds the same header, footer or watermark to each PDF in a folder. The results go to a new folder next to them; the files themselves are not changed."</p>
            <div class="row">
                <span class="selectable" style="flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">
                    {move || { let v = d.node("batch-folder").value; if v.is_empty() { "No folder chosen".to_string() } else { v } }}
                </span>
                <Btn dlg=d id="batch-pick" label="Choose folder…"/>
            </div>
            <Choice dlg=d id="batch-kind" label="Add" options=&[("footer", "A footer at the bottom"), ("header", "A header at the top"), ("watermark", "A watermark across each page")]/>
            <Field dlg=d id="batch-text" label="Text" hint="You can use {page}, {pages}, {label} and {bates} (a running number that continues from file to file)."/>
            <Choice dlg=d id="batch-align" label="Position" options=&[("left", "Left"), ("right", "Right")] hide_disabled=true/>
            <div class="grid2" hidden=move || !d.node("batch-text").value.contains("{bates}")>
                <Field dlg=d id="batch-bates-start" label="Running number starts at"/>
                <Field dlg=d id="batch-bates-pad" label="Number of digits"/>
            </div>
            <Field dlg=d id="batch-pages" label="Pages" placeholder="All pages"/>
            <Status dlg=d id="batch-summary" plain=true/>
            <Rows dlg=d head=&["File", "Result"]/>
            <div class="actions">
                <Btn dlg=d id="batch-cancel" label="Cancel"/>
                <Btn dlg=d id="batch-close" label="Done" class="primary"/>
                <Btn dlg=d id="batch-start" label="Start" class="primary"/>
            </div>
        }.into_any(),

        "sign-choice-dialog" => view! {
            {title("Sign")}
            <div style="display:grid;gap:12px;margin-top:12px">
                <button class="card" on:click=click("sign-choice-picture")>
                    <Icon name="sign"/>
                    <strong>"Add my signature"</strong>
                    <small>"Draw it or use a picture of it, then click where it goes. Quick, but it does not prove who signed."</small>
                </button>
                <button class="card" on:click=click("sign-choice-certificate")>
                    <Icon name="verify"/>
                    <strong>"Sign digitally with a certificate"</strong>
                    <small>"Uses your certificate file. Anyone can check who signed and that nothing changed since. Saves a signed copy, which then opens."</small>
                </button>
            </div>
            <div class="actions"><Btn dlg=d id="sign-choice-cancel" label="Cancel"/></div>
        }.into_any(),

        "cert-sign-dialog" => view! {
            {title("Sign digitally")}
            <div class="row">
                <span class="selectable" style="flex:1">{move || { let t = d.node("cert-sign-file").text; if t.is_empty() { "No certificate chosen yet".into() } else { t } }}</span>
                <Btn dlg=d id="cert-sign-pick" label="Choose certificate…"/>
            </div>
            <Field dlg=d id="cert-sign-password" label="Certificate password" kind="password"/>
            {move || { let t = d.node("cert-sign-field").text; (!t.is_empty()).then(|| view! { <p>{t}</p> }) }}
            <Field dlg=d id="cert-sign-reason" label="Reason (optional)" placeholder="I approve this document"/>
            // Signing into a field: the field decides where the signature goes.
            <div hidden=move || !d.node("cert-sign-field").text.is_empty()>
            <Check dlg=d id="cert-sign-visible" label="Show the signature on the page"/>
            <div class="row" hidden=move || !d.node("cert-sign-visible").checked>
                <Btn dlg=d id="cert-sign-place" label="Drag on the page…"/>
                <small style="color:var(--muted)">"Draw the box where the signature goes; this dialog comes back."</small>
            </div>
            <details hidden=move || !d.node("cert-sign-visible").checked>
                <summary>"Where the signature goes"</summary>
                <div class="grid2">
                    <Field dlg=d id="cert-sign-page" label="Page" kind="number"/>
                    <span></span>
                    <Field dlg=d id="cert-sign-x" label="From the left (points)" kind="number"/>
                    <Field dlg=d id="cert-sign-y" label="From the top (points)" kind="number"/>
                    <Field dlg=d id="cert-sign-w" label="Width (points)" kind="number"/>
                    <Field dlg=d id="cert-sign-h" label="Height (points)" kind="number"/>
                </div>
                <small style="color:var(--muted)">"72 points are one inch. Or use “Drag on the page…”, or click an empty signature field on the page."</small>
            </details>
            </div>
            <Check dlg=d id="cert-sign-certify" label="Lock the document after signing" hint="Others can still fill in forms and add signatures, but nothing else."/>
            <Status dlg=d id="cert-sign-certify-reason"/>
            <Status dlg=d id="cert-sign-status"/>
            <div class="actions"><Btn dlg=d id="cert-sign-cancel" label="Cancel"/><Btn dlg=d id="cert-sign-go" label="Sign and save as…" class="primary"/></div>
        }.into_any(),

        "security-dialog" => view! { <Security dlg=d/> }.into_any(),

        other => {
            let other = other.to_string();
            view! {
                {title(d.text("dialog-title"))}
                <p>{format!("This window ({other}) has no layout yet.")}</p>
                <div class="actions"><button class="btn" on:click=move |_| store().act(UiAction::DialogCancel { id: other.clone() })>"Close"</button></div>
            }.into_any()
        }
    }
}

/// A read-only table of the dialog's rows.
#[component]
fn Rows(dlg: Dlg, head: &'static [&'static str]) -> impl IntoView {
    let rows = move || {
        dlg.0
            .with(|d| d.as_ref().map(|d| d.rows.clone()).unwrap_or_default())
    };
    view! {
        {move || {
            let rows = rows();
            (!rows.is_empty()).then(|| view! {
                <table class="data selectable">
                    <thead><tr>{head.iter().map(|h| view! { <th scope="col">{*h}</th> }).collect_view()}</tr></thead>
                    <tbody>{rows.into_iter().map(|r| view! { <tr>{r.into_iter().map(|c| view! { <td>{c}</td> }).collect_view()}</tr> }).collect_view()}</tbody>
                </table>
            })
        }}
    }
}

/// The verify dialog's results: each row's verdict (from `data.rowKinds`) gets its own icon
/// and colour, so a broken signature never looks like an untrusted one.
#[component]
fn VerifyRows(dlg: Dlg) -> impl IntoView {
    let rows = move || {
        dlg.0.with(|d| {
            d.as_ref()
                .map(|d| {
                    let kinds: Vec<String> = d.data["rowKinds"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .map(|k| k.as_str().unwrap_or("").to_string())
                                .collect()
                        })
                        .unwrap_or_default();
                    d.rows
                        .iter()
                        .enumerate()
                        .map(|(i, r)| (kinds.get(i).cloned().unwrap_or_default(), r.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
    };
    view! {
        {move || {
            let rows = rows();
            (!rows.is_empty()).then(|| view! {
                <table class="data selectable">
                    <thead><tr><th scope="col">"Signature"</th><th scope="col">"Signed by"</th><th scope="col">"Result"</th></tr></thead>
                    <tbody>{rows.into_iter().map(|(kind, r)| {
                        let (icon, word) = match kind.as_str() {
                            "valid" => ("check", "Valid"),
                            "untrusted" => ("warning", "Not trusted"),
                            "broken" => ("close", "Broken"),
                            "empty" => ("sign", "Not signed"),
                            _ => ("info", "Unknown"),
                        };
                        let class = format!("verdict {}", if kind.is_empty() { "unknown" } else { kind.as_str() });
                        let mut cells = r.into_iter();
                        let (a, b, c) = (cells.next().unwrap_or_default(), cells.next().unwrap_or_default(), cells.next().unwrap_or_default());
                        view! {
                            <tr>
                                <td>{a}</td><td>{b}</td>
                                <td><span class=class><Icon name=icon class="sm"/>{word}</span><br/>{c}</td>
                            </tr>
                        }
                    }).collect_view()}</tbody>
                </table>
            })
        }}
    }
}

/// The certificates the person trusts (`data.trusted`), or the core's empty sentence.
#[component]
fn TrustedList(dlg: Dlg) -> impl IntoView {
    let data = move || {
        dlg.0
            .with(|d| d.as_ref().map(|d| d.data.clone()).unwrap_or_default())
    };
    view! {
        {move || {
            let data = data();
            let names: Vec<String> = data["trusted"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if names.is_empty() {
                let empty = data["trustedEmpty"].as_str().map(str::to_string).unwrap_or_else(|| dlg.node("verify-trusted").text);
                view! { <p class="selectable" style="color:var(--muted)">{empty}</p> }.into_any()
            } else {
                view! { <ul class="trusted-list selectable">{names.into_iter().map(|n| view! { <li>{n}</li> }).collect_view()}</ul> }.into_any()
            }
        }}
    }
}

/// Where inserted pages go: before the first page, after this page, at the end, or after a page number.
#[component]
fn InsertWhere(dlg: Dlg) -> impl IntoView {
    let store = store();
    let pages = move || {
        store
            .state
            .with(|s| s.view.doc.as_ref().map(|d| d.page_count).unwrap_or(0))
    };
    let current = move || store.state.with(|s| s.view.page + 1);
    let value = move || dlg.node("insert-after").value;
    let set = move |v: String| {
        store.act(UiAction::Input {
            id: "insert-after".into(),
            value: v,
        })
    };
    let choice = move || {
        let v = value();
        if v == "0" {
            "first"
        } else if v == pages().to_string() {
            "end"
        } else if v == current().to_string() {
            "here"
        } else {
            "custom"
        }
    };
    view! {
        <div class="field" role="radiogroup" aria-label="Where">
            <span>"Where"</span>
            <label class="check"><input type="radio" name="ins" prop:checked=move || choice() == "first" on:change=move |_| set("0".into())/>"Before the first page"</label>
            <label class="check"><input type="radio" name="ins" prop:checked=move || choice() == "here" on:change=move |_| set(current().to_string())/>{move || format!("After this page (page {})", current())}</label>
            <label class="check"><input type="radio" name="ins" prop:checked=move || choice() == "end" on:change=move |_| set(pages().to_string())/>"At the end"</label>
            <label class="check"><input type="radio" name="ins" prop:checked=move || choice() == "custom" on:change=move |_| set("1".into())/>
                "After page "
                <input class="input" style="width:64px" type="number" min="0" prop:value=value on:input=move |ev| set(event_target_value(&ev))/>
            </label>
        </div>
    }
}

#[component]
fn Security(dlg: Dlg) -> impl IntoView {
    let store = store();
    let aes = move || dlg.node("security-mode-aes").checked;
    view! {
        {title("Security")}
        <p>{move || plain_security(&dlg.node("security-current").text)}</p>
        <div role="radiogroup" aria-label="Protection">
            <label class="check"><input type="radio" name="sec" prop:checked=move || dlg.node("security-mode-none").checked
                on:change=move |_| store.act(UiAction::Check { id: "security-mode-none".into(), checked: true })/>"No password"</label>
            <label class="check"><input type="radio" name="sec" prop:checked=aes
                on:change=move |_| store.act(UiAction::Check { id: "security-mode-aes".into(), checked: true })/>"Require a password to open the file"</label>
        </div>
        <div hidden=move || !aes()>
            <Field dlg=dlg id="security-user" label="Password to open" kind="password" hint="Letters, digits and common punctuation work everywhere; other characters may not open in every app."/>
            <Field dlg=dlg id="security-owner" label="Password to change these settings (optional)" kind="password" hint="Leave blank to use the same password."/>
            <div class="field"><span>"People who open the file may"</span>
                <Check dlg=dlg id="security-perm-print" label="Print"/>
                <Check dlg=dlg id="security-perm-copy" label="Copy text"/>
                <Check dlg=dlg id="security-perm-edit" label="Change pages"/>
                <Check dlg=dlg id="security-perm-annotate" label="Add comments"/>
                <Check dlg=dlg id="security-perm-form" label="Fill in forms"/>
            </div>
        </div>
        <p class="hint">"The new protection is written into the file the next time you save it."</p>
        <Status dlg=dlg id="security-status"/>
        <div class="actions"><Btn dlg=dlg id="security-cancel" label="Cancel"/><Btn dlg=dlg id="security-apply" label="Apply" class="primary"/></div>
    }
}

/// The security dialog's "Currently: …" line in plain words. The core keeps its own text
/// (STATE pins it); only what is shown changes.
fn plain_security(text: &str) -> String {
    let Some(rest) = text.strip_prefix("Currently: ") else {
        return text.to_string();
    };
    if rest.starts_with("no encryption") {
        return "Right now anyone can open this file.".into();
    }
    let weak = rest.contains("weak");
    let owner = rest.contains("owner password");
    let mut out = String::from("Right now this file needs a password to open");
    out.push_str(if weak {
        ", with old, weak protection."
    } else {
        "."
    });
    if owner {
        out.push_str(" You opened it with the password that can change these settings.");
    }
    out
}

/// The signature pad: strokes are drawn here and sent to the core when saved.
#[component]
fn SignaturePad(dlg: Dlg) -> impl IntoView {
    use wasm_bindgen::JsCast;
    let store = store();
    let canvas = NodeRef::<leptos::html::Canvas>::new();
    let strokes = RwSignal::new(Vec::<Vec<[f64; 2]>>::new());
    let drawing = RwSignal::new(false);
    let ctx = move || -> Option<web_sys::CanvasRenderingContext2d> {
        canvas
            .get_untracked()?
            .get_context("2d")
            .ok()??
            .dyn_into()
            .ok()
    };
    let point = move |ev: &web_sys::PointerEvent| -> [f64; 2] {
        let el: web_sys::Element = ev
            .current_target()
            .and_then(|t| t.dyn_into().ok())
            .expect("pad");
        let r = el.get_bounding_client_rect();
        [
            ev.client_x() as f64 - r.left(),
            ev.client_y() as f64 - r.top(),
        ]
    };
    let redraw = move || {
        let (Some(c), Some(el)) = (ctx(), canvas.get_untracked()) else {
            return;
        };
        let dpr = crate::bridge::device_pixel_ratio();
        let (w, h) = (el.client_width() as f64, el.client_height() as f64);
        el.set_width((w * dpr) as u32);
        el.set_height((h * dpr) as u32);
        let _ = c.scale(dpr, dpr);
        c.set_line_width(2.2);
        c.set_line_cap("round");
        c.set_line_join("round");
        c.set_stroke_style_str("#1b2a52");
        for s in strokes.get_untracked() {
            c.begin_path();
            for (i, p) in s.iter().enumerate() {
                if i == 0 {
                    c.move_to(p[0], p[1])
                } else {
                    c.line_to(p[0], p[1])
                }
            }
            c.stroke();
        }
    };
    Effect::new(move |_| {
        if canvas.get().is_some() {
            redraw();
        }
    });
    let has_strokes = move || !strokes.with(|s| s.is_empty());
    view! {
        {title("Your signature")}
        <p>"Draw your signature with the trackpad or mouse. It is a picture kept on this Mac, not a certificate signature: it does not prove who signed."</p>
        <canvas class="sigpad" node_ref=canvas role="img" aria-label="Signature drawing area"
            on:pointerdown=move |ev: web_sys::PointerEvent| {
                ev.prevent_default();
                if let Some(el) = ev.current_target().and_then(|t| t.dyn_into::<web_sys::Element>().ok()) { let _ = el.set_pointer_capture(ev.pointer_id()); }
                drawing.set(true);
                strokes.update(|s| s.push(vec![point(&ev)]));
            }
            on:pointermove=move |ev: web_sys::PointerEvent| if drawing.get_untracked() {
                let p = point(&ev);
                strokes.update(|s| if let Some(last) = s.last_mut() { last.push(p) });
                if let Some(c) = ctx() {
                    let s = strokes.get_untracked();
                    if let Some(last) = s.last() && last.len() >= 2 {
                        let a = last[last.len() - 2];
                        c.begin_path(); c.move_to(a[0], a[1]); c.line_to(p[0], p[1]); c.stroke();
                    }
                }
            }
            on:pointerup=move |_| drawing.set(false)></canvas>
        // The pad's own tools sit under it; the dialog's answers stay on their own row.
        <div class="row wrap sigpad-tools">
            <button class="btn" on:click=move |_| { strokes.set(vec![]); redraw(); store.act(UiAction::Click { id: "sigpad-clear".into() }) }>"Clear"</button>
            <Btn dlg=dlg id="sigpad-image" label="Use a picture…"/>
            <span class="spacer"></span>
            <Btn dlg=dlg id="sigpad-remove" label="Remove saved signature" class="quiet"/>
        </div>
        <Status dlg=dlg id="sigpad-status"/>
        <div class="actions">
            <Btn dlg=dlg id="sigpad-cancel" label="Cancel"/>
            <button class="btn primary" disabled=move || !has_strokes() && dlg.node("sigpad-save").disabled on:click=move |_| {
                let (w, h) = canvas.get_untracked().map(|c| (c.client_width() as f64, c.client_height() as f64)).unwrap_or((0.0, 0.0));
                let s = strokes.get_untracked();
                if !s.is_empty() { store.act(UiAction::SignaturePad { strokes: s, width: w, height: h }); }
                store.act(UiAction::Click { id: "sigpad-save".into() });
            }>"Save"</button>
        </div>
    }
}

/// Help in plain words (replaces the old capability matrix, UX audit X7).
#[component]
fn Help() -> impl IntoView {
    view! {
        <div class="help">
            <h3>"Tools"</h3>
            <dl>
                <dt>"Select"</dt><dd>"Drag across text to select it; a small bar offers Highlight, Underline, Strikethrough, Copy and Redact. Click a link to follow it (with any other tool, ⌘-click it)."</dd>
                <dt>"Highlight"</dt><dd>"Drag across text to highlight, underline or strike it through."</dd>
                <dt>"Draw"</dt><dd>"Draw freehand, or drag to make a line, rectangle or ellipse. Stays on until you press Esc."</dd>
                <dt>"Note"</dt><dd>"Click anywhere to leave a note and type it at once."</dd>
                <dt>"Text"</dt><dd>"Click existing text to change it, or empty space to add text. Click a picture or a drawing to move, resize, rotate or replace it."</dd>
                <dt>"Fill & Sign"</dt><dd>"Click a form field to type in it; Tab moves to the next one. Sign… adds your signature or signs with a certificate."</dd>
                <dt>"Redact"</dt><dd>"Drag a box over anything you want removed. Nothing is removed until you choose Redact; the result is saved as a new file."</dd>
                <dt>"Pages"</dt><dd>"Rotate, move, delete, insert and extract pages."</dd>
            </dl>
            <p>"Esc always takes you back to Select. Every change can be undone with ⌘Z."</p>
            <h3>"Changing existing text"</h3>
            <p>"Most text can be changed in place. Some cannot: when a file includes only part of a font, only the letters already used on that page can be typed; text drawn as a picture, rotated text and right-to-left scripts cannot be changed yet. When text cannot be changed, Tandem PDF says why and leaves the page untouched."</p>
            <h3>"Forms"</h3>
            <p>"Fill in text boxes, checkboxes, lists and choices with Fill & Sign. To create or change form fields, open the side panel's Fields tab and choose “Create or change form fields”."</p>
            <h3>"Signatures"</h3>
            <p>"“Add my signature” places a picture of your signature. “Sign digitally” uses a certificate file (.p12, .pfx or .pem) so others can check who signed and that nothing changed since. Verify Signatures tells you, in plain words, whether each signature is intact and whether you trust the person who signed."</p>
            <h3>"Redaction"</h3>
            <p>"Everything inside a box is removed from the file: text, pictures, drawings, comments and form fields. Text inside a picture needs a box drawn over it. Files up to 12.5 MB can be redacted. A signed file is redacted into an unsigned copy."</p>
            <h3>"Headers, footers and running numbers"</h3>
            <p>"Page ▸ Header and Footer adds text to every page, with the page number or a running number that continues across the documents you open. File ▸ Batch adds the same text to every PDF in a folder."</p>
            <h3>"Keyboard"</h3>
            <dl>
                <dt>"⌘O, ⌘N, ⌘S, ⇧⌘S"</dt><dd>"Open, New, Save, Save As"</dd>
                <dt>"⌘Z, ⇧⌘Z"</dt><dd>"Undo, Redo"</dd>
                <dt>"⌘F, ⌘G, ⇧⌘G"</dt><dd>"Find, Find Next, Find Previous"</dd>
                <dt>"⌘+, ⌘−, ⌘0, ⌘9, ⌘8"</dt><dd>"Zoom in, zoom out, actual size, fit width, fit page"</dd>
                <dt>"⌃⌘1 … ⌃⌘8"</dt><dd>"Select, Highlight, Draw, Note, Text, Fill & Sign, Redact, Pages"</dd>
                <dt>"Esc"</dt><dd>"Close what is open, then back to Select"</dd>
                <dt>"⌥⌘1, ⌥⌘2"</dt><dd>"Show or hide the sidebar, the side panel"</dd>
                <dt>"⌥⌘← ⌥⌘→"</dt><dd>"Previous page, next page"</dd>
                <dt>"Delete, arrows"</dt><dd>"Delete or nudge what is selected on the page (⇧ for bigger steps)"</dd>
                <dt>"Tab"</dt><dd>"Next form field while filling in"</dd>
                <dt>"⌘?"</dt><dd>"This help"</dd>
            </dl>
            <h3>"Private by design"</h3>
            <p>"Tandem PDF works offline. Nothing you open is uploaded, and scripts inside PDF files are never run."</p>
        </div>
    }
}
