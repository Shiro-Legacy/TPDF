//! The app's icons: simple 24×24 line drawings made for Tandem PDF (no
//! third-party icon set, so nothing to license). Stroke colour follows the
//! text colour; every icon is decorative (`aria-hidden`), its button carries
//! the label.

use leptos::prelude::*;

/// SVG markup (inside a 24×24 viewBox) for an icon name.
pub fn path(name: &str) -> &'static str {
    match name {
        "open" => r#"<path d="M3 7.5V18a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V9.5a2 2 0 0 0-2-2h-7l-2-2.5H5a2 2 0 0 0-2 2Z"/>"#,
        "new" => r#"<path d="M6 3h8l4 4v14H6Z"/><path d="M14 3v4h4"/><path d="M12 11v6M9 14h6"/>"#,
        "images" => r#"<rect x="3" y="5" width="18" height="14" rx="2"/><circle cx="9" cy="10" r="1.6"/><path d="m21 16-5-5-8 8"/>"#,
        "folder" => r#"<path d="M3 7.5V18a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V9.5a2 2 0 0 0-2-2h-7l-2-2.5H5a2 2 0 0 0-2 2Z"/>"#,
        "save" => r#"<path d="M5 3h11l3 3v13a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2Z"/><path d="M8 3v5h7V3"/><rect x="8" y="13" width="8" height="5" rx="1"/>"#,
        "share" => r#"<path d="M12 3v12"/><path d="m8 7 4-4 4 4"/><path d="M5 12v7a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-7"/>"#,
        "print" => r#"<path d="M7 9V3h10v6"/><rect x="3" y="9" width="18" height="8" rx="2"/><path d="M7 14h10v7H7Z"/>"#,
        "undo" => r#"<path d="M9 14 4 9l5-5"/><path d="M4 9h10a6 6 0 0 1 0 12h-3"/>"#,
        "redo" => r#"<path d="m15 14 5-5-5-5"/><path d="M20 9H10a6 6 0 0 0 0 12h3"/>"#,
        "search" => r#"<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2"/>"#,
        "left" => r#"<path d="m15 18-6-6 6-6"/>"#,
        "right" => r#"<path d="m9 18 6-6-6-6"/>"#,
        "up" => r#"<path d="m18 15-6-6-6 6"/>"#,
        "down" => r#"<path d="m6 9 6 6 6-6"/>"#,
        "close" => r#"<path d="M18 6 6 18M6 6l12 12"/>"#,
        "plus" => r#"<path d="M12 5v14M5 12h14"/>"#,
        "minus" => r#"<path d="M5 12h14"/>"#,
        "sidebar" => r#"<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/>"#,
        "panel" => r#"<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M15 4v16"/>"#,
        "select" => r#"<path d="M5 3.5 18 11l-6 1.6L9 19Z"/><path d="m12.5 13 4.5 6"/>"#,
        "highlight" => r#"<path d="m9 15 6-11 4 2.2-6 11Z"/><path d="M9 15 7.5 19l3.5-1.8"/><path d="M4 21h16" stroke-width="2.4" opacity=".55"/>"#,
        "draw" => r#"<path d="M4 20c2.5-3 4-6.5 7-6.5s2.5 4 5 4 3-2.5 4-4.5"/><path d="m14 4 4 4-7.5 7.5H6.5v-4Z"/>"#,
        "note" => r#"<path d="M4 5a1 1 0 0 1 1-1h14a1 1 0 0 1 1 1v10l-5 5H5a1 1 0 0 1-1-1Z"/><path d="M15 20v-5h5"/><path d="M8 9h8M8 12.5h5"/>"#,
        "text" => r#"<path d="M5 6V4h14v2"/><path d="M12 4v16"/><path d="M9 20h6"/>"#,
        "sign" => r#"<path d="M3 17c2-4 4.5-9 6-9s-1 8 1 8 3.5-5 5-5-.5 4 1 4 2-1 4-2"/><path d="M3 21h18"/>"#,
        "redact" => r#"<rect x="3.5" y="7" width="17" height="10" rx="1.5" fill="currentColor"/>"#,
        "pages" => r#"<rect x="4" y="3" width="7" height="8" rx="1"/><rect x="13" y="3" width="7" height="8" rx="1"/><rect x="4" y="13" width="7" height="8" rx="1"/><rect x="13" y="13" width="7" height="8" rx="1"/>"#,
        "rotate" => r#"<path d="M20 12a8 8 0 1 1-2.3-5.6"/><path d="M20 4v4.5h-4.5"/>"#,
        "rotate-left" => r#"<path d="M4 12a8 8 0 1 0 2.3-5.6"/><path d="M4 4v4.5h4.5"/>"#,
        "trash" => r#"<path d="M4 7h16"/><path d="M9 7V4h6v3"/><path d="M6 7l1 13h10l1-13"/>"#,
        "lock" => r#"<rect x="5" y="10.5" width="14" height="10" rx="2"/><path d="M8 10.5V8a4 4 0 0 1 8 0v2.5"/>"#,
        "check" => r#"<path d="m5 12.5 4.5 4.5L19 7.5"/>"#,
        "verify" => r#"<path d="M12 3 4.5 6v5.5c0 4.6 3.2 8.4 7.5 9.5 4.3-1.1 7.5-4.9 7.5-9.5V6Z"/><path d="m8.5 12 2.5 2.5 4.5-5"/>"#,
        "compare" => r#"<rect x="3" y="4" width="8" height="16" rx="1.5"/><rect x="13" y="4" width="8" height="16" rx="1.5"/><path d="M6 9h2M6 12h2M16 9h2M16 12h2"/>"#,
        "batch" => r#"<path d="M7 7V4h13v13h-3"/><rect x="4" y="7" width="13" height="13" rx="1.5"/>"#,
        "info" => r#"<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5v.5"/>"#,
        "warning" => r#"<path d="M12 4 2.8 19.5h18.4Z"/><path d="M12 10v4.5M12 17v.5"/>"#,
        "help" => r#"<circle cx="12" cy="12" r="9"/><path d="M9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.7.3-1 1-1 1.7v.5M12 17v.5"/>"#,
        "zoom-in" => r#"<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2M11 8v6M8 11h6"/>"#,
        "zoom-out" => r#"<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2M8 11h6"/>"#,
        "link" => r#"<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/>"#,
        "bookmark" => r#"<path d="M6 3h12v18l-6-4-6 4Z"/>"#,
        "layers" => r#"<path d="m12 3 9 5-9 5-9-5Z"/><path d="m3 13 9 5 9-5"/>"#,
        "comment" => r#"<path d="M4 5h16v11H9l-5 4Z"/>"#,
        "square" => r#"<rect x="4" y="5" width="16" height="14" rx="1"/>"#,
        "circle" => r#"<ellipse cx="12" cy="12" rx="8.5" ry="7"/>"#,
        "line" => r#"<path d="M5 19 19 5"/>"#,
        "ink" => r#"<path d="M4 17c3-6 5-9 7-9s0 9 3 9 3-6 6-9"/>"#,
        "stamp" => r#"<path d="M9 12V8a3 3 0 1 1 6 0v4"/><path d="M5 12h14v4H5Z"/><path d="M4 20h16"/>"#,
        "eye" => r#"<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12Z"/><circle cx="12" cy="12" r="3"/>"#,
        "form" => r#"<rect x="3" y="5" width="18" height="5" rx="1"/><rect x="3" y="14" width="18" height="5" rx="1"/><path d="M6 7.5h4M6 16.5h7"/>"#,
        "export" => r#"<path d="M14 3h7v7"/><path d="M21 3 11 13"/><path d="M19 14v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7a2 2 0 0 1 2-2h5"/>"#,
        "more" => r#"<circle cx="5.5" cy="12" r="1.2"/><circle cx="12" cy="12" r="1.2"/><circle cx="18.5" cy="12" r="1.2"/>"#,
        "pdf" => r#"<path d="M6 3h8l4 4v14H6Z"/><path d="M14 3v4h4"/>"#,
        "watermark" => r#"<path d="M6 3h8l4 4v14H6Z"/><path d="m8.5 17 7-7" opacity=".6"/>"#,
        "copy" => r#"<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3"/>"#,
        "scissors" => r#"<circle cx="6" cy="7" r="2.5"/><circle cx="6" cy="17" r="2.5"/><path d="M8 8.5 20 18M8 15.5 20 6"/>"#,
        "paste" => r#"<path d="M9 4h6v3H9Z"/><path d="M9 5.5H6v15h12v-15h-3"/>"#,
        _ => r#"<circle cx="12" cy="12" r="2"/>"#,
    }
}

/// An inline SVG icon, hidden from assistive technology.
#[component]
pub fn Icon(#[prop(into)] name: String, #[prop(optional, into)] class: Option<String>) -> impl IntoView {
    let class = match class {
        Some(extra) => format!("icon {extra}"),
        None => "icon".to_string(),
    };
    view! { <svg class=class viewBox="0 0 24 24" aria-hidden="true" focusable="false" inner_html=path(&name)></svg> }
}
