//! Inline SVG icons (24x24, stroked). Drawn for this project.

use leptos::prelude::*;

fn svg(body: &'static str) -> impl IntoView {
    // `inner_html` is safe here: `body` is one of the constants below, never
    // user input.
    view! { <svg viewBox="0 0 24 24" aria-hidden="true" inner_html=body></svg> }
}

macro_rules! icons {
    ($($name:ident => $body:expr;)*) => {
        $(pub fn $name() -> impl IntoView { svg($body) })*
    };
}

icons! {
    back => r#"<path d="M15 5l-7 7 7 7"/>"#;
    undo => r#"<path d="M9 7L4 12l5 5"/><path d="M4 12h10a6 6 0 0 1 0 12"/>"#;
    redo => r#"<path d="M15 7l5 5-5 5"/><path d="M20 12H10a6 6 0 0 0 0 12"/>"#;
    share => r#"<circle cx="6" cy="12" r="2.5"/><circle cx="18" cy="6" r="2.5"/><circle cx="18" cy="18" r="2.5"/><path d="M8.2 10.9l7.6-3.8M8.2 13.1l7.6 3.8"/>"#;
    text => r#"<path d="M5 6V4h14v2M12 4v16M9 20h6"/>"#;
    image => r#"<rect x="3" y="4" width="18" height="16" rx="3"/><circle cx="9" cy="10" r="1.6"/><path d="M4 18l5-5 4 4 3-3 4 4"/>"#;
    brush => r#"<path d="M4 20c3 0 5-1 5-4a3 3 0 0 0-3-3c-2 0-2 3-2 7z"/><path d="M9 13L19 3l2 2-10 10"/>"#;
    shapes => r#"<rect x="3" y="12" width="9" height="9" rx="1.5"/><circle cx="16" cy="8" r="5"/>"#;
    sticker => r#"<path d="M12 3l2.6 5.6 6.1.7-4.5 4.2 1.2 6-5.4-3-5.4 3 1.2-6L3.3 9.3l6.1-.7z"/>"#;
    template => r#"<rect x="3" y="3" width="18" height="18" rx="3"/><path d="M3 9h18M9 9v12"/>"#;
    catalog => r#"<path d="M8 3L3 6l2 5 2-1v10h10V10l2 1 2-5-5-3a4 4 0 0 1-8 0z"/>"#;
    design => r#"<path d="M4 20l1-5L16 4l4 4L9 19z"/><path d="M14 6l4 4"/>"#;
    camera => r#"<path d="M4 8h3l2-3h6l2 3h3v11H4z"/><circle cx="12" cy="13" r="3.5"/>"#;
    bag => r#"<path d="M5 8h14l-1 12H6z"/><path d="M9 8V6a3 3 0 0 1 6 0v2"/>"#;
    list => r#"<path d="M8 6h12M8 12h12M8 18h12"/><circle cx="4" cy="6" r="1"/><circle cx="4" cy="12" r="1"/><circle cx="4" cy="18" r="1"/>"#;
    trash => r#"<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/>"#;
    copy => r#"<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a1 1 0 0 0-1-1H5a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h3"/>"#;
    flip => r#"<path d="M12 3v18M8 7l-5 5 5 5zM16 7l5 5-5 5z"/>"#;
    up => r#"<path d="M12 19V5M6 11l6-6 6 6"/>"#;
    down => r#"<path d="M12 5v14M6 13l6 6 6-6"/>"#;
    center => r#"<circle cx="12" cy="12" r="3"/><path d="M12 2v5M12 17v5M2 12h5M17 12h5"/>"#;
    swap => r#"<path d="M4 8h13l-3-3M20 16H7l3 3"/>"#;
    info => r#"<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5v.5"/>"#;
    check => r#"<path d="M5 12l5 5 9-10"/>"#;
}
