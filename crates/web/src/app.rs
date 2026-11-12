//! App shell: routes, bottom navigation, toast.

use leptos::prelude::*;

use crate::icons;
use crate::pages::{editor::EditorPage, home::HomePage, legal::LegalPage, order::OrderPage, track::TrackPage, tryon::TryOnPage};
use crate::router::{Route, Router};
use crate::state::{t, Ctx};

pub fn mount() {
    console_error_panic_hook::set_once();
    crate::platform::register_service_worker();
    crate::platform::remove_boot_splash();
    leptos::mount::mount_to_body(App);
    // After mounting, so the router is listening when a deep link arrives.
    crate::platform::init_native();
}

#[component]
fn App() -> impl IntoView {
    let ctx = Ctx::init();
    let router = Router::new();
    provide_context(router);

    view! {
        <div class="app">
            // The route signal decides which screen exists; switching routes
            // drops the old screen (and e.g. stops its camera).
            {move || match router.0.get() {
                Route::Home => view! { <HomePage/> }.into_any(),
                Route::Design(slug) => view! { <EditorPage garment=Some(slug) shared=None/> }.into_any(),
                Route::Shared(slug) => view! { <EditorPage garment=None shared=Some(slug)/> }.into_any(),
                Route::TryOn => view! { <TryOnPage/> }.into_any(),
                Route::Order => view! { <OrderPage/> }.into_any(),
                Route::Track(code) => view! { <TrackPage code=code/> }.into_any(),
                Route::Legal => view! { <LegalPage/> }.into_any(),
            }}
            <TabBar/>
            {move || ctx.toast.get().map(|(msg, bad)| view! { <div class="toast" class:bad=bad role="status">{msg}</div> })}
        </div>
    }
}

#[component]
fn TabBar() -> impl IntoView {
    let router = crate::router::use_router();
    let ctx = crate::state::ctx();
    // The design tab reopens the draft's garment, or the catalog if none.
    let design_route = move || match ctx.draft.with(|d| d.doc.as_ref().map(|doc| doc.garment.clone())) {
        Some(slug) => Route::Design(slug),
        None => Route::Home,
    };
    let tab = move |label: &'static str, icon: AnyView, is_on: fn(&Route) -> bool, target: Box<dyn Fn() -> Route>| {
        view! {
            <button class="tab" class:on=move || is_on(&router.0.get()) on:click=move |_| router.go(target())>
                {icon}
                <span>{move || t(label)}</span>
            </button>
        }
    };
    view! {
        <nav class="tabbar">
            {tab("nav.catalog", icons::catalog().into_any(), |r| matches!(r, Route::Home), Box::new(|| Route::Home))}
            {tab("nav.design", icons::design().into_any(), |r| matches!(r, Route::Design(_) | Route::Shared(_)), Box::new(design_route))}
            {tab("nav.tryon", icons::camera().into_any(), |r| matches!(r, Route::TryOn), Box::new(|| Route::TryOn))}
            {tab("nav.order", icons::bag().into_any(), |r| matches!(r, Route::Order), Box::new(|| Route::Order))}
            {tab("nav.track", icons::list().into_any(), |r| matches!(r, Route::Track(_) | Route::Legal), Box::new(|| Route::Track(None)))}
        </nav>
    }
}

/// Shared top bar with a back button and an optional right-hand slot.
#[component]
pub fn TopBar(#[prop(into)] title: Signal<String>, #[prop(optional)] children: Option<Children>) -> impl IntoView {
    let router = crate::router::use_router();
    view! {
        <header class="topbar">
            <button class="icon-btn" aria-label=move || t("common.back") on:click=move |_| router.back()>{icons::back()}</button>
            <h1>{move || title.get()}</h1>
            {children.map(|c| c())}
        </header>
    }
}
