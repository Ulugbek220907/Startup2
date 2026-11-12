//! Order tracking by order number + phone.

use domain::api::{OrderLookupRequest, OrderPublicDto};
use domain::order::OrderStatus;
use leptos::prelude::*;

use crate::api;
use crate::app::TopBar;
use crate::router::{use_router, Route};
use crate::state::{ctx, t};

/// `2026-11-11T14:05:00Z` -> `11.11 19:05` in Tashkent time (UTC+5, no DST).
fn short_time(rfc3339: &str) -> String {
    let ms = js_sys::Date::parse(rfc3339);
    if ms.is_nan() {
        return String::new();
    }
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms + 5.0 * 3600.0 * 1000.0));
    format!("{:02}.{:02} {:02}:{:02}", d.get_utc_date(), d.get_utc_month() + 1, d.get_utc_hours(), d.get_utc_minutes())
}

#[component]
pub fn TrackPage(code: Option<String>) -> impl IntoView {
    let ctx = ctx();
    let router = use_router();
    let code_sig = RwSignal::new(code.clone().unwrap_or_default());
    let phone = RwSignal::new(String::new());
    let order = RwSignal::new(None::<OrderPublicDto>);
    let busy = RwSignal::new(false);
    let not_found = RwSignal::new(false);

    let lookup = move |code: String, phone_full: String| {
        busy.set(true);
        not_found.set(false);
        leptos::task::spawn_local(async move {
            let req = OrderLookupRequest { code: code.clone(), phone: phone_full.clone() };
            let r = api::post::<_, OrderPublicDto>("/api/orders/lookup", &req).await;
            busy.set(false);
            match r {
                Ok(o) => {
                    ctx.remember_order(&o.code, &phone_full);
                    order.set(Some(o));
                }
                Err(e) if e.status == 404 || e.code == "invalid_phone" => not_found.set(true),
                Err(e) => ctx.show_error(&e),
            }
        });
    };

    // Opened from the order confirmation: the phone is already known.
    if let Some(c) = code {
        if let Some(saved) = ctx.orders.with_untracked(|o| o.iter().find(|x| x.code == c).cloned()) {
            lookup(saved.code, saved.phone);
        }
    }

    let timeline = move |o: OrderPublicDto| {
        let at = |s: OrderStatus| o.timeline.iter().rev().find(|e| e.status == s).map(|e| short_time(&e.at));
        let cancelled = o.status == OrderStatus::Cancelled;
        let reached = OrderStatus::FLOW.iter().position(|s| *s == o.status);
        let mut items = Vec::new();
        for (i, s) in OrderStatus::FLOW.into_iter().enumerate() {
            let time = at(s);
            // After a cancellation only the steps that really happened show.
            if cancelled && time.is_none() {
                continue;
            }
            let done = cancelled || reached.is_some_and(|r| i < r) || (o.status == OrderStatus::Delivered && s == OrderStatus::Delivered);
            let now = !cancelled && reached == Some(i) && !done;
            items.push(view! { <li class:done=done class:now=now><span>{move || t(s.i18n_key())}</span><time>{time}</time></li> }.into_any());
        }
        if cancelled {
            let time = at(OrderStatus::Cancelled);
            items.push(view! { <li class="cancelled"><span>{move || t("status.cancelled")}</span><time>{time}</time></li> }.into_any());
        }
        view! { <ul class="timeline">{items}</ul> }
    };

    view! {
        <TopBar title=Signal::derive(move || t("track.title").to_string())/>
        <main class="page">
            {move || order.get().map(|o| {
                let total = o.quote.total.format_sum();
                view! {
                    <div class="card">
                        <div class="spread">
                            <div class="code num" style="font-size:28px">{o.code.clone()}</div>
                            <button class="link-btn" on:click=move |_| order.set(None)>{move || t("common.close")}</button>
                        </div>
                        <p class="muted">{format!("{} · {} · {} · {} ", o.garment_name, o.color, o.size, o.quantity)}{move || t("common.pcs")}</p>
                        <div class="line total"><span>{move || t("common.total")}</span><span class="num">{total}" "{move || t("common.sum")}</span></div>
                    </div>
                    <div class="card">{timeline(o)}</div>
                }
            })}
            <Show when=move || order.get().is_none()>
                <div class="card">
                    <label class="field">
                        <span>{move || t("order.code")}</span>
                        <input class="input num" type="text" maxlength="12" autocapitalize="characters" placeholder="QD-XXXXXX" prop:value=move || code_sig.get() on:input=move |ev| code_sig.set(event_target_value(&ev).to_uppercase())/>
                    </label>
                    <label class="field">
                        <span>{move || t("order.phone")}</span>
                        <div class="phone">
                            <span class="prefix num">"+998"</span>
                            <input class="input num" type="tel" inputmode="numeric" maxlength="12" placeholder="90 123 45 67" prop:value=move || phone.get() on:input=move |ev| phone.set(event_target_value(&ev))/>
                        </div>
                    </label>
                    <button class="btn btn-primary" disabled=move || busy.get() on:click=move |_| lookup(code_sig.get_untracked().trim().to_string(), format!("+998{}", phone.get_untracked()))>
                        {move || t("track.find")}
                    </button>
                    <Show when=move || not_found.get()>
                        <div class="note bad">{move || t("track.not_found")}</div>
                    </Show>
                </div>
                <Show when=move || !ctx.orders.with(|o| o.is_empty())>
                    <h3 class="section">{move || t("track.my_orders")}</h3>
                    <div class="chips wrap">
                        {move || ctx.orders.get().into_iter().map(|o| {
                            let label = o.code.clone();
                            view! { <button class="chip num" on:click=move |_| lookup(o.code.clone(), o.phone.clone())>{label}</button> }
                        }).collect_view()}
                    </div>
                </Show>
            </Show>
            <button class="link-btn" on:click=move |_| router.go(Route::Legal)>{move || t("legal.title")}</button>
        </main>
    }
}
