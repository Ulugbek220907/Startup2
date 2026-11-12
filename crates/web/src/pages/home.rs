//! Catalog: pick a garment to design.

use design::{Color, Side};
use domain::Lang;
use leptos::prelude::*;

use crate::draw;
use crate::router::{use_router, Route};
use crate::state::{ctx, t, tf};

#[component]
pub fn HomePage() -> impl IntoView {
    let ctx = ctx();
    let router = use_router();

    // Cheapest possible price: garment plus the smallest print tier.
    let min_print = move || ctx.catalog.with(|c| c.as_ref().and_then(|c| c.price_tiers.iter().map(|t| t.price.0).min()).unwrap_or(0));

    view! {
        <header class="topbar">
            <h1 style="padding-left:8px">{move || ctx.catalog.with(|c| c.as_ref().map(|c| c.brand.clone()).unwrap_or_else(|| "QiyiqDoʻppi".into()))}</h1>
            <div class="lang-row" role="group" aria-label=move || t("common.language")>
                {Lang::ALL
                    .into_iter()
                    .map(|l| {
                        view! {
                            <button class="chip" style="min-height:44px;padding:0 12px" class:on=move || ctx.lang.get() == l on:click=move |_| ctx.set_lang(l)>
                                {l.code().to_uppercase()}
                            </button>
                        }
                    })
                    .collect_view()}
            </div>
        </header>
        <main class="page">
            <h2 class="title">{move || t("home.title")}</h2>
            <Show when=move || ctx.catalog_error.get()>
                <div class="note bad">{move || t("error.network")}</div>
                <button class="btn btn-ghost" on:click=move |_| ctx.load_catalog()>{move || t("common.retry")}</button>
            </Show>
            <Show when=move || ctx.catalog.with(|c| c.is_none()) && !ctx.catalog_error.get()>
                <p class="muted">{move || t("common.loading")}</p>
            </Show>
            <div class="garments">
                {move || {
                    ctx.catalog
                        .get()
                        .map(|c| c.garments)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|g| {
                            let color = g.colors.first().and_then(|c| Color::parse(&c.hex)).unwrap_or(Color::BLACK);
                            let art = draw::garment_url(g.kind, Side::Front, color);
                            let slug = g.slug.clone();
                            let price = domain::Tiyin(g.base_price.0 + min_print()).format_sum();
                            let name = g.name.clone();
                            view! {
                                <button class="garment" on:click=move |_| router.go(Route::Design(slug.clone()))>
                                    <div class="art"><img src=art alt="" loading="lazy"/></div>
                                    <div class="name">{move || name.get(ctx.lang.get()).to_string()}</div>
                                    <div class="from num">{move || tf("home.from_price", &[("price", &price)])}</div>
                                </button>
                            }
                        })
                        .collect_view()
                }}
            </div>
            <button class="link-btn" on:click=move |_| router.go(Route::Legal)>{move || t("legal.title")}</button>
        </main>
    }
}
