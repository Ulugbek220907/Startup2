//! Order request: size, quantity, delivery, contact details, server price.

use design::{Color, Side};
use domain::api::{CreateOrderRequest, DeliveryChoice, OrderCreatedDto, QuoteRequest};
use domain::order::DeliveryMethod;
use domain::pricing::Quote;
use domain::Tiyin;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::app::TopBar;
use crate::router::{use_router, Route};
use crate::state::{ctx, store_get, store_set, t};
use crate::{api, draw, icons, platform};

/// Contact details remembered on this device for the next order.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Contact {
    phone: String,
    name: String,
    region: String,
    address: String,
}

fn money(v: Tiyin) -> String {
    v.format_sum()
}

#[component]
pub fn OrderPage() -> impl IntoView {
    let ctx = ctx();
    let router = use_router();
    let saved: Contact = store_get("qd.contact").unwrap_or_default();
    let phone = RwSignal::new(saved.phone);
    let name = RwSignal::new(saved.name);
    let region = RwSignal::new(saved.region);
    let address = RwSignal::new(saved.address);
    let notes = RwSignal::new(String::new());
    let pickup = RwSignal::new(false);
    let rights = RwSignal::new(false);
    let sending = RwSignal::new(false);
    let quote = RwSignal::new(None::<Result<Quote, String>>);
    let created = RwSignal::new(None::<OrderCreatedDto>);
    let thumb_ref = NodeRef::<leptos::html::Canvas>::new();

    let has_design = move || ctx.draft.with(|d| d.doc.as_ref().is_some_and(|doc| !doc.is_empty()));
    let delivery = move || DeliveryChoice {
        method: if pickup.get() { DeliveryMethod::Pickup } else { DeliveryMethod::Delivery },
        region: (!pickup.get() && !region.get().is_empty()).then(|| region.get()),
    };

    // Default size: the middle of the garment's size list.
    Effect::new(move |_| {
        if let Some(g) = ctx.garment() {
            if ctx.size.get_untracked().is_none_or(|s| !g.sizes.contains(&s)) {
                ctx.size.set(g.sizes.get(g.sizes.len() / 2).cloned());
            }
        }
    });

    // Ask the server for the price whenever an input that affects it
    // changes. `seq` discards answers that arrive out of order.
    let seq = StoredValue::new(0u32);
    Effect::new(move |_| {
        let Some(doc) = ctx.draft.with(|d| d.doc.clone()).filter(|d| !d.is_empty()) else { return };
        let req = QuoteRequest { doc, quantity: ctx.quantity.get(), delivery: delivery() };
        if req.delivery.method == DeliveryMethod::Delivery && req.delivery.region.is_none() {
            // No region yet: price without delivery so a number is shown.
            let req = QuoteRequest { delivery: DeliveryChoice { method: DeliveryMethod::Pickup, region: None }, ..req };
            fetch_quote(req, seq, quote);
        } else {
            fetch_quote(req, seq, quote);
        }
    });

    Effect::new(move |_| {
        ctx.redraw.track();
        let (Some(canvas), Some(g)) = (thumb_ref.get(), ctx.garment()) else { return };
        ctx.draft.with(|d| {
            if let Some(doc) = d.doc.as_ref() {
                let color = g.color(&doc.color).and_then(|c| Color::parse(&c.hex)).unwrap_or(Color::BLACK);
                draw::paint_full(&canvas, 96.0, g.kind, Side::Front, color, g.area(Side::Front), &doc.front);
            }
        });
    });

    let submit = move |_| {
        if sending.get_untracked() {
            return;
        }
        let Some(doc) = ctx.draft.with_untracked(|d| d.doc.clone()) else { return };
        // Check locally first for instant feedback; the server checks again.
        let fail = |key: &str| ctx.show_toast(t(key), true);
        let full_phone = format!("+998{}", phone.get_untracked());
        if domain::phone::normalize(&full_phone).is_err() {
            return fail("error.invalid_phone");
        }
        let Some(size) = ctx.size.get_untracked() else { return fail("error.size_required") };
        let choice = delivery();
        if choice.method == DeliveryMethod::Delivery {
            if choice.region.is_none() {
                return fail("error.region_required");
            }
            if address.get_untracked().trim().chars().count() < 5 {
                return fail("error.address_required");
            }
        }
        if !rights.get_untracked() {
            return fail("error.rights_required");
        }
        store_set(
            "qd.contact",
            &Contact { phone: phone.get_untracked(), name: name.get_untracked(), region: region.get_untracked(), address: address.get_untracked() },
        );
        // Read every field now: after an `await` the screen (and its
        // signals) may be gone.
        let opt = |s: String| Some(s.trim().to_string()).filter(|s| !s.is_empty());
        let mut req = CreateOrderRequest {
            doc,
            design_slug: None,
            size,
            quantity: ctx.quantity.get_untracked(),
            phone: full_phone.clone(),
            customer_name: opt(name.get_untracked()),
            delivery: choice,
            address: opt(address.get_untracked()),
            notes: opt(notes.get_untracked()),
            lang: ctx.lang.get_untracked(),
            accept_rights: true,
        };
        sending.set(true);
        leptos::task::spawn_local(async move {
            // Saving the design is best-effort: the order carries its own copy.
            req.design_slug = ctx.ensure_saved().await.ok();
            let result = api::post::<_, OrderCreatedDto>("/api/orders", &req).await;
            sending.set(false);
            match result {
                Ok(order) => {
                    platform::haptic("success");
                    ctx.remember_order(&order.code, &full_phone);
                    created.set(Some(order));
                    if let Some(w) = web_sys::window() {
                        w.scroll_to_with_x_and_y(0.0, 0.0);
                    }
                }
                Err(e) => ctx.show_error(&e),
            }
        });
    };

    let line = |label: &'static str, value: Signal<String>| view! { <div class="line"><span>{move || t(label)}</span><span class="num">{move || value.get()}</span></div> };
    let q = move || quote.get().and_then(|r| r.ok());

    view! {
        <TopBar title=Signal::derive(move || t("order.title").to_string())/>
        {move || match created.get() {
            Some(order) => {
                let code = order.code.clone();
                let bot = ctx.catalog.with(|c| c.as_ref().and_then(|c| c.telegram_bot.clone()));
                view! {
                    <main class="page">
                        <div class="card center">
                            <h2 class="title">{move || t("order.sent_title")}</h2>
                            <p class="muted">{move || t("order.code")}</p>
                            <div class="code num">{order.code.clone()}</div>
                            <div class="line total"><span>{move || t("common.total")}</span><span class="num">{money(order.quote.total)}" "{move || t("common.sum")}</span></div>
                            <p class="muted">{move || t("order.payment_cod")}</p>
                            <p>{move || t("order.confirm_note")}</p>
                        </div>
                        <button class="btn btn-primary" on:click=move |_| router.go(Route::Track(Some(code.clone())))>{move || t("order.track")}</button>
                        {bot.map(|b| view! { <a class="btn btn-ghost" href=format!("https://t.me/{b}") target="_blank" rel="noopener">{move || t("order.tg_notify")}</a> })}
                        <button class="btn btn-ghost" on:click=move |_| router.go(Route::Home)>{move || t("order.new_design")}</button>
                    </main>
                }
                    .into_any()
            }
            None => view! {
                <main class="page has-bar">
                    <Show
                        when=has_design
                        fallback=move || view! {
                            <div class="note">{move || t("order.empty_design")}</div>
                            <button class="btn btn-primary" on:click=move |_| router.go(Route::Home)>{move || t("home.start")}</button>
                        }
                    >
                        <div class="card">
                            <div class="row">
                                <canvas class="thumb" node_ref=thumb_ref></canvas>
                                <div class="grow stack">
                                    <h3 class="section">{move || ctx.garment().map(|g| g.name.get(ctx.lang.get()).to_string())}</h3>
                                    <div class="stepper">
                                        <button aria-label="-" on:click=move |_| ctx.quantity.update(|q| *q = q.saturating_sub(1).max(1))>"−"</button>
                                        <span class="value num">{move || ctx.quantity.get()}</span>
                                        <button aria-label="+" on:click=move |_| ctx.quantity.update(|q| *q = (*q + 1).min(domain::pricing::MAX_QUANTITY))>"+"</button>
                                        <span class="muted">{move || t("common.pcs")}</span>
                                    </div>
                                </div>
                            </div>
                            <div class="field">
                                <span>{move || t("common.size")}</span>
                                <div class="chips wrap">
                                    {move || ctx.garment().map(|g| g.sizes).unwrap_or_default().into_iter().map(|s| {
                                        let (a, b) = (s.clone(), s.clone());
                                        view! { <button class="chip" class:on=move || ctx.size.get().as_deref() == Some(a.as_str()) on:click=move |_| ctx.size.set(Some(b.clone()))>{s}</button> }
                                    }).collect_view()}
                                </div>
                            </div>
                        </div>

                        <div class="seg">
                            <button class:on=move || !pickup.get() on:click=move |_| pickup.set(false)>{move || t("order.delivery")}</button>
                            <button class:on=move || pickup.get() on:click=move |_| pickup.set(true)>{move || t("order.pickup")}</button>
                        </div>
                        <Show
                            when=move || !pickup.get()
                            fallback=move || view! {
                                <div class="note">
                                    <div class="muted small">{move || t("order.pickup_address")}</div>
                                    {move || ctx.catalog.with(|c| c.as_ref().map(|c| c.pickup_address.get(ctx.lang.get()).to_string()))}
                                </div>
                            }
                        >
                            <label class="field">
                                <span>{move || t("order.region")}</span>
                                <select class="select" on:change=move |ev| region.set(event_target_value(&ev))>
                                    <option value="" selected=move || region.get().is_empty()>{move || t("order.region_choose")}</option>
                                    {move || ctx.catalog.with(|c| c.as_ref().map(|c| c.regions.clone()).unwrap_or_default()).into_iter().map(|r| {
                                        let slug = r.slug.clone();
                                        view! { <option value=r.slug.clone() selected=move || region.get() == slug>{move || r.name.get(ctx.lang.get()).to_string()}</option> }
                                    }).collect_view()}
                                </select>
                            </label>
                            <label class="field">
                                <span>{move || t("order.address")}</span>
                                <input class="input" type="text" maxlength="300" autocomplete="street-address" placeholder=move || t("order.address_ph") prop:value=move || address.get() on:input=move |ev| address.set(event_target_value(&ev))/>
                            </label>
                        </Show>

                        <label class="field">
                            <span>{move || t("order.phone")}</span>
                            <div class="phone">
                                <span class="prefix num">"+998"</span>
                                <input
                                    class="input num"
                                    type="tel"
                                    inputmode="numeric"
                                    autocomplete="tel-national"
                                    maxlength="12"
                                    placeholder="90 123 45 67"
                                    prop:value=move || phone.get()
                                    on:input=move |ev| phone.set(event_target_value(&ev).chars().filter(|c| c.is_ascii_digit() || *c == ' ').collect())
                                />
                            </div>
                        </label>
                        {move || ctx.telegram.get().and_then(|me| me.phone).filter(|p| p.len() == 13 && phone.get() != p[4..]).map(|p| {
                            view! { <button class="btn btn-ghost btn-sm" on:click=move |_| phone.set(p[4..].to_string())>{move || t("order.use_tg_phone")}</button> }
                        })}
                        <label class="field">
                            <span>{move || t("order.name")}</span>
                            <input class="input" type="text" maxlength="80" autocomplete="given-name" prop:value=move || name.get() on:input=move |ev| name.set(event_target_value(&ev))/>
                        </label>
                        <label class="field">
                            <span>{move || t("order.notes")}</span>
                            <textarea class="textarea" maxlength="500" prop:value=move || notes.get() on:input=move |ev| notes.set(event_target_value(&ev))></textarea>
                        </label>

                        <div class="card">
                            <div class="lines">
                                {line("order.garment_price", Signal::derive(move || q().map(|q| money(q.unit_garment)).unwrap_or_default()))}
                                {line("order.print_price", Signal::derive(move || q().map(|q| money(q.unit_print)).unwrap_or_default()))}
                                <Show when=move || q().is_some_and(|q| q.quantity > 1)>
                                    {line("common.quantity", Signal::derive(move || q().map(|q| format!("× {}", q.quantity)).unwrap_or_default()))}
                                </Show>
                                <Show when=move || q().is_some_and(|q| q.discount.0 > 0)>
                                    {line("order.discount", Signal::derive(move || q().map(|q| format!("−{}", money(q.discount))).unwrap_or_default()))}
                                </Show>
                                {line("order.delivery", Signal::derive(move || match q() {
                                    Some(q) if q.delivery_fee.0 > 0 => money(q.delivery_fee),
                                    Some(_) if pickup.get() => t("order.free").to_string(),
                                    _ => "—".to_string(),
                                }))}
                                <div class="line total"><span>{move || t("common.total")}</span><span class="num">{move || q().map(|q| money(q.total)).unwrap_or_else(|| "…".into())}" "{move || t("common.sum")}</span></div>
                            </div>
                            <p class="muted small">{move || t("order.payment_cod")}</p>
                        </div>
                        {move || quote.get().and_then(|r| r.err()).map(|key| view! { <div class="note bad">{move || t(&key)}</div> })}

                        <label class="check">
                            <input type="checkbox" prop:checked=move || rights.get() on:change=move |ev| rights.set(event_target_checked(&ev))/>
                            <span>{move || t("order.rights")}</span>
                        </label>
                        <p class="muted small">{move || t("order.confirm_note")}</p>
                    </Show>
                </main>
                <Show when=has_design>
                    <div class="sticky-bar">
                        <button class="btn btn-primary btn-block" disabled=move || sending.get() on:click=submit>
                            {icons::check()}
                            {move || if sending.get() { t("order.sending") } else { t("order.submit") }}
                        </button>
                    </div>
                </Show>
            }
                .into_any(),
        }}
    }
}

fn fetch_quote(req: QuoteRequest, seq: StoredValue<u32>, out: RwSignal<Option<Result<Quote, String>>>) {
    seq.update_value(|n| *n += 1);
    let mine = seq.get_value();
    leptos::task::spawn_local(async move {
        // Short pause so tapping "+" several times sends one request.
        crate::pages::tryon::sleep_ms(250).await;
        // `try_`: the order screen may have been closed while we waited.
        if seq.try_get_value() != Some(mine) {
            return;
        }
        let result = api::post::<_, Quote>("/api/price/quote", &req).await;
        if seq.try_get_value() == Some(mine) {
            out.set(Some(result.map_err(|e| e.key())));
        }
    });
}
