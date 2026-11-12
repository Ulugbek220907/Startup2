//! A very small client-side router on the History API.
//!
//! The app has six screens, so a routing library would cost more bytes than
//! this file. The current route is a signal; `navigate` pushes a history
//! entry and updates it; the browser's back button fires `popstate`.

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Home,
    /// `/design/<garment slug>` starts or continues a design for a garment.
    Design(String),
    /// `/d/<share slug>` opens a shared design.
    Shared(String),
    TryOn,
    Order,
    /// `/track` or `/track/<code>`.
    Track(Option<String>),
    Legal,
}

impl Route {
    pub fn parse(path: &str) -> Route {
        let mut parts = path.trim_matches('/').split('/').filter(|s| !s.is_empty());
        match (parts.next(), parts.next()) {
            (Some("design"), Some(slug)) => Route::Design(slug.to_string()),
            (Some("d"), Some(slug)) => Route::Shared(slug.to_string()),
            (Some("tryon"), _) => Route::TryOn,
            (Some("order"), _) => Route::Order,
            (Some("track"), code) => Route::Track(code.map(str::to_string)),
            (Some("legal"), _) => Route::Legal,
            _ => Route::Home,
        }
    }

    pub fn path(&self) -> String {
        match self {
            Route::Home => "/".into(),
            Route::Design(s) => format!("/design/{s}"),
            Route::Shared(s) => format!("/d/{s}"),
            Route::TryOn => "/tryon".into(),
            Route::Order => "/order".into(),
            Route::Track(None) => "/track".into(),
            Route::Track(Some(c)) => format!("/track/{c}"),
            Route::Legal => "/legal".into(),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Router(pub RwSignal<Route>);

impl Router {
    pub fn new() -> Self {
        let path = web_sys::window().and_then(|w| w.location().pathname().ok()).unwrap_or_default();
        let route = RwSignal::new(Route::parse(&path));
        // `Closure::forget` leaks the listener on purpose: it lives as long
        // as the page does.
        let on_pop = Closure::<dyn FnMut()>::new(move || {
            let path = web_sys::window().and_then(|w| w.location().pathname().ok()).unwrap_or_default();
            route.set(Route::parse(&path));
        });
        if let Some(w) = web_sys::window() {
            let _ = w.add_event_listener_with_callback("popstate", on_pop.as_ref().unchecked_ref());
        }
        on_pop.forget();
        Router(route)
    }

    pub fn go(&self, to: Route) {
        if self.0.get_untracked() == to {
            return;
        }
        if let Some(w) = web_sys::window() {
            if let Ok(h) = w.history() {
                let _ = h.push_state_with_url(&JsValue::NULL, "", Some(&to.path()));
            }
            w.scroll_to_with_x_and_y(0.0, 0.0);
        }
        self.0.set(to);
    }

    pub fn back(&self) {
        let went = web_sys::window().and_then(|w| w.history().ok()).filter(|h| h.length().unwrap_or(0) > 1).map(|h| h.back().is_ok());
        if went != Some(true) {
            self.go(Route::Home);
        }
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

pub fn use_router() -> Router {
    expect_context::<Router>()
}
