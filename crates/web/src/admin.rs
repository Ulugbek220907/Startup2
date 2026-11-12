//! Admin page: orders, print files, moderation, reports, prices, audit log.
//!
//! A separate WASM bundle served at `/admin/`, so customers never download
//! it. Routing uses the URL hash (`#/orders/<id>`), which is also what the
//! QR code on the production sheet links to.

use domain::api::*;
use domain::order::OrderStatus;
use domain::pricing::{AreaTier, QuantityDiscount};
use domain::Tiyin;
use leptos::prelude::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::{self, ApiFail};
use crate::state::{t, Ctx};

pub fn mount() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(Admin);
}

#[derive(Clone, Debug, PartialEq)]
enum Page {
    Orders,
    Order(String),
    Moderation,
    Reports,
    Prices,
    Audit,
}

fn parse_hash(hash: &str) -> Page {
    let mut parts = hash.trim_start_matches('#').trim_matches('/').split('/');
    match (parts.next(), parts.next()) {
        (Some("orders"), Some(id)) if !id.is_empty() => Page::Order(id.to_string()),
        (Some("moderation"), _) => Page::Moderation,
        (Some("reports"), _) => Page::Reports,
        (Some("prices"), _) => Page::Prices,
        (Some("audit"), _) => Page::Audit,
        _ => Page::Orders,
    }
}

fn go(hash: &str) {
    if let Some(w) = web_sys::window() {
        let _ = w.location().set_hash(hash);
    }
}

/// Shared admin state: the session (with its CSRF token) and a toast.
#[derive(Clone, Copy)]
struct Adm {
    me: RwSignal<Option<AdminMeDto>>,
    ctx: Ctx,
}

impl Adm {
    /// A state-changing call; adds the CSRF token.
    async fn send<B: Serialize, T: DeserializeOwned>(&self, method: &str, path: &str, body: &B) -> Result<T, ApiFail> {
        let csrf = self.me.with_untracked(|m| m.as_ref().map(|m| m.csrf_token.clone())).unwrap_or_default();
        let r = api::request(method, path, body, &[("x-csrf-token", &csrf)]).await;
        self.check(&r);
        r
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ApiFail> {
        let r = api::get(path).await;
        self.check(&r);
        r
    }

    /// A 401 means the session expired: show the login form again.
    fn check<T>(&self, r: &Result<T, ApiFail>) {
        match r {
            Err(e) if e.status == 401 => self.me.set(None),
            Err(e) => self.ctx.show_error(e),
            Ok(_) => {}
        }
    }
}

fn local_time(rfc3339: &str) -> String {
    let ms = js_sys::Date::parse(rfc3339);
    if ms.is_nan() {
        return rfc3339.to_string();
    }
    // Tashkent time, UTC+5 all year.
    let d = js_sys::Date::new(&JsValue::from_f64(ms + 5.0 * 3600.0 * 1000.0));
    format!("{:02}.{:02}.{} {:02}:{:02}", d.get_utc_date(), d.get_utc_month() + 1, d.get_utc_full_year(), d.get_utc_hours(), d.get_utc_minutes())
}

#[component]
fn Admin() -> impl IntoView {
    let ctx = Ctx::init();
    let adm = Adm { me: RwSignal::new(None), ctx };
    let checked = RwSignal::new(false);
    let page = RwSignal::new(parse_hash(&web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default()));

    let on_hash = Closure::<dyn FnMut()>::new(move || {
        page.set(parse_hash(&web_sys::window().and_then(|w| w.location().hash().ok()).unwrap_or_default()));
    });
    if let Some(w) = web_sys::window() {
        let _ = w.add_event_listener_with_callback("hashchange", on_hash.as_ref().unchecked_ref());
    }
    on_hash.forget(); // lives as long as the page

    // Is there already a valid session cookie?
    leptos::task::spawn_local(async move {
        if let Ok(me) = api::get::<AdminMeDto>("/api/admin/me").await {
            adm.me.set(Some(me));
        }
        checked.set(true);
    });

    let logout = move |_| {
        leptos::task::spawn_local(async move {
            let _ = adm.send::<_, OkDto>("POST", "/api/admin/logout", &serde_json::json!({})).await;
            adm.me.set(None);
        });
    };
    let nav = move |label: &'static str, hash: &'static str, is: fn(&Page) -> bool| {
        view! { <button class="chip" class:on=move || is(&page.get()) on:click=move |_| go(hash)>{move || t(label)}</button> }
    };

    view! {
        <div class="admin-app">
            <Show when=move || checked.get()>
                <Show when=move || adm.me.with(|m| m.is_some()) fallback=move || view! { <Login adm=adm/> }>
                    <nav class="admin-nav">
                        <span class="brand">{move || t("admin.title")}</span>
                        {nav("admin.orders", "#/orders", |p| matches!(p, Page::Orders | Page::Order(_)))}
                        {nav("admin.moderation", "#/moderation", |p| matches!(p, Page::Moderation))}
                        {nav("admin.reports", "#/reports", |p| matches!(p, Page::Reports))}
                        {nav("admin.prices", "#/prices", |p| matches!(p, Page::Prices))}
                        {nav("admin.audit", "#/audit", |p| matches!(p, Page::Audit))}
                        <span class="spacer"></span>
                        <button class="link-btn" on:click=logout>{move || t("admin.sign_out")}</button>
                    </nav>
                    {move || match page.get() {
                        Page::Orders => view! { <Orders adm=adm/> }.into_any(),
                        Page::Order(id) => view! { <OrderDetail adm=adm id=id/> }.into_any(),
                        Page::Moderation => view! { <Moderation adm=adm/> }.into_any(),
                        Page::Reports => view! { <Reports adm=adm/> }.into_any(),
                        Page::Prices => view! { <Prices adm=adm/> }.into_any(),
                        Page::Audit => view! { <Audit adm=adm/> }.into_any(),
                    }}
                </Show>
            </Show>
            {move || ctx.toast.get().map(|(msg, bad)| view! { <div class="toast" class:bad=bad role="status">{msg}</div> })}
        </div>
    }
}

#[component]
fn Login(adm: Adm) -> impl IntoView {
    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        leptos::task::spawn_local(async move {
            let req = LoginRequest { username: username.get_untracked(), password: password.get_untracked() };
            let r = api::post::<_, AdminMeDto>("/api/admin/login", &req).await;
            busy.set(false);
            password.set(String::new());
            match r {
                Ok(me) => adm.me.set(Some(me)),
                Err(e) => adm.ctx.show_error(&e),
            }
        });
    };
    view! {
        <form class="card login" on:submit=submit>
            <h2 class="title">{move || t("admin.title")}</h2>
            <label class="field">
                <span>{move || t("admin.username")}</span>
                <input class="input" type="text" autocomplete="username" prop:value=move || username.get() on:input=move |ev| username.set(event_target_value(&ev))/>
            </label>
            <label class="field">
                <span>{move || t("admin.password")}</span>
                <input class="input" type="password" autocomplete="current-password" prop:value=move || password.get() on:input=move |ev| password.set(event_target_value(&ev))/>
            </label>
            <button class="btn btn-primary" type="submit" disabled=move || busy.get()>{move || t("admin.sign_in")}</button>
        </form>
    }
}

fn status_pill(s: OrderStatus) -> impl IntoView {
    view! { <span class=format!("pill {}", s.as_str())>{move || t(s.i18n_key())}</span> }
}

#[component]
fn Orders(adm: Adm) -> impl IntoView {
    let status = RwSignal::new(None::<OrderStatus>);
    let search = RwSignal::new(String::new());
    let page = RwSignal::new(0i64);
    let list = RwSignal::new(None::<AdminOrderList>);

    let load = move || {
        let mut path = format!("/api/admin/orders?page={}", page.get_untracked());
        if let Some(s) = status.get_untracked() {
            path.push_str(&format!("&status={}", s.as_str()));
        }
        let q = search.get_untracked();
        if !q.trim().is_empty() {
            path.push_str(&format!("&q={}", String::from(js_sys::encode_uri_component(q.trim()))));
        }
        leptos::task::spawn_local(async move {
            if let Ok(l) = adm.get::<AdminOrderList>(&path).await {
                list.set(Some(l));
            }
        });
    };
    Effect::new(move |_| {
        status.track();
        page.track();
        load();
    });

    view! {
        <div class="stack">
            <div class="chips wrap">
                <button class="chip" class:on=move || status.get().is_none() on:click=move |_| { page.set(0); status.set(None) }>{move || t("admin.all")}</button>
                {OrderStatus::ALL.into_iter().map(|s| view! {
                    <button class="chip" class:on=move || status.get() == Some(s) on:click=move |_| { page.set(0); status.set(Some(s)) }>{move || t(s.i18n_key())}</button>
                }).collect_view()}
            </div>
            <form class="row" on:submit=move |ev: leptos::ev::SubmitEvent| { ev.prevent_default(); page.set(0); load(); }>
                <input class="input grow" type="search" placeholder=move || t("admin.search_ph") prop:value=move || search.get() on:input=move |ev| search.set(event_target_value(&ev))/>
                <button class="btn btn-ghost btn-sm" type="submit">{move || t("track.find")}</button>
            </form>
            <div class="table-wrap">
                <table class="table">
                    <thead><tr>
                        <th>{move || t("admin.order")}</th><th>{move || t("admin.created")}</th><th>{move || t("admin.customer")}</th>
                        <th>{move || t("admin.garment")}</th><th>{move || t("common.total")}</th><th></th>
                    </tr></thead>
                    <tbody>
                        {move || list.get().map(|l| l.orders).unwrap_or_default().into_iter().map(|o| {
                            let id = o.id.clone();
                            let needs_review = !o.moderation.allows_production();
                            view! {
                                <tr class="click" on:click=move |_| go(&format!("#/orders/{id}"))>
                                    <td class="num"><b>{o.code}</b></td>
                                    <td class="num">{local_time(&o.created_at)}</td>
                                    <td>{o.customer_name.unwrap_or_default()}<div class="muted num">{domain::phone::format_display(&o.phone)}</div></td>
                                    <td>{format!("{} · {} · {}", o.garment_name, o.size, o.quantity)}</td>
                                    <td class="num">{o.total.format_sum()}</td>
                                    <td>
                                        {status_pill(o.status)}
                                        {needs_review.then(|| view! { " "<span class="pill flagged">{move || t("admin.needs_review")}</span> })}
                                    </td>
                                </tr>
                            }
                        }).collect_view()}
                    </tbody>
                </table>
            </div>
            <Show when=move || list.with(|l| l.as_ref().is_some_and(|l| l.orders.is_empty()))>
                <p class="muted">{move || t("admin.empty")}</p>
            </Show>
            <div class="row">
                <button class="btn btn-ghost btn-sm" disabled=move || page.get() == 0 on:click=move |_| page.update(|p| *p -= 1)>"‹"</button>
                <span class="muted num">{move || list.with(|l| l.as_ref().map(|l| format!("{} / {}", page.get() + 1, (l.total + 29) / 30)).unwrap_or_default())}</span>
                <button class="btn btn-ghost btn-sm" disabled=move || list.with(|l| l.as_ref().is_none_or(|l| (page.get() + 1) * 30 >= l.total)) on:click=move |_| page.update(|p| *p += 1)>"›"</button>
            </div>
        </div>
    }
}

fn file_label(kind: &str) -> &'static str {
    match kind {
        "print_front" => "admin.file_print_front",
        "print_back" => "admin.file_print_back",
        "mockup_front" => "admin.file_mockup_front",
        "mockup_back" => "admin.file_mockup_back",
        _ => "admin.file_sheet",
    }
}

#[component]
fn OrderDetail(adm: Adm, id: String) -> impl IntoView {
    let detail = RwSignal::new(None::<AdminOrderDetail>);
    let note = RwSignal::new(String::new());
    let id = StoredValue::new(id);
    let load = move || {
        leptos::task::spawn_local(async move {
            if let Ok(d) = adm.get::<AdminOrderDetail>(&format!("/api/admin/orders/{}", id.get_value())).await {
                // Files are rendered in the background: check again shortly.
                if d.files_status == "pending" {
                    set_timeout(move || reload(adm, id, detail), std::time::Duration::from_secs(3));
                }
                detail.set(Some(d));
            }
        });
    };
    load();
    let change = move |to: OrderStatus| {
        leptos::task::spawn_local(async move {
            let req = StatusChangeRequest { to, note: Some(note.get_untracked()).filter(|n| !n.trim().is_empty()) };
            if adm.send::<_, OkDto>("POST", &format!("/api/admin/orders/{}/status", id.get_value()), &req).await.is_ok() {
                note.set(String::new());
                load();
            }
        });
    };
    let rerender = move |_| {
        leptos::task::spawn_local(async move {
            if adm.send::<_, OkDto>("POST", &format!("/api/admin/orders/{}/rerender", id.get_value()), &serde_json::json!({})).await.is_ok() {
                load();
            }
        });
    };

    view! {
        <button class="link-btn" on:click=move |_| go("#/orders")>"‹ "{move || t("admin.orders")}</button>
        {move || detail.get().map(|d| {
            let q = d.quote;
            // Front first.
            let mut mockups: Vec<String> = d.files.iter().filter(|f| f.kind.starts_with("mockup")).map(|f| f.url.clone()).collect();
            mockups.reverse();
            let region = d.region.clone().map(|slug| {
                adm.ctx.catalog.with(|c| c.as_ref().and_then(|c| c.regions.iter().find(|r| r.slug == slug)).map(|r| r.name.get(adm.ctx.lang.get()).to_string())).unwrap_or(slug)
            });
            let downloads: Vec<OrderFileDto> = d.files.iter().filter(|f| !f.kind.starts_with("mockup")).cloned().collect();
            let needs_review = !d.row.moderation.allows_production();
            view! {
                <div class="cols">
                    <div class="stack">
                        <div class="card">
                            <div class="spread"><div class="code num">{d.row.code.clone()}</div>{status_pill(d.row.status)}</div>
                            <div class="muted num">{local_time(&d.row.created_at)}</div>
                            <div class="lines">
                                <div class="line"><span>{move || t("admin.garment")}</span><span>{format!("{} · {}", d.row.garment_name, d.color)}</span></div>
                                <div class="line"><span>{move || t("common.size")}</span><span><b>{d.row.size.clone()}</b></span></div>
                                <div class="line"><span>{move || t("common.quantity")}</span><span class="num"><b>{d.row.quantity}</b></span></div>
                                <div class="line"><span>{move || t("admin.customer")}</span><span>{d.row.customer_name.clone().unwrap_or_default()}" "<span class="num">{domain::phone::format_display(&d.row.phone)}</span></span></div>
                                <div class="line"><span>{move || t("order.delivery")}</span><span>{region.unwrap_or_else(|| t("order.pickup").to_string())}</span></div>
                                {d.address.clone().map(|a| view! { <div class="line"><span>{move || t("order.address")}</span><span>{a}</span></div> })}
                                {d.notes.clone().map(|n| view! { <div class="line"><span>{move || t("order.notes")}</span><span>{n}</span></div> })}
                                <div class="line"><span>{move || t("order.discount")}</span><span class="num">{q.discount.format_sum()}</span></div>
                                <div class="line"><span>{move || t("order.delivery")}</span><span class="num">{q.delivery_fee.format_sum()}</span></div>
                                <div class="line total"><span>{move || t("common.total")}</span><span class="num">{q.total.format_sum()}</span></div>
                            </div>
                        </div>
                        <div class="card">
                            <h3 class="section">{move || t("admin.change_status")}</h3>
                            {needs_review.then(|| view! {
                                <div class="note warn">{move || t("admin.needs_review")}{d.moderation_reason.clone().map(|r| format!(": {r}"))}</div>
                                <button class="link-btn" on:click=move |_| go("#/moderation")>{move || t("admin.moderation")}</button>
                            })}
                            <input class="input" type="text" maxlength="500" placeholder=move || t("admin.note") prop:value=move || note.get() on:input=move |ev| note.set(event_target_value(&ev))/>
                            <div class="row wrap">
                                {d.allowed_next.iter().copied().map(|s| {
                                    let class = if s == OrderStatus::Cancelled { "btn btn-danger btn-sm" } else { "btn btn-primary btn-sm" };
                                    view! { <button class=class on:click=move |_| change(s)>{move || t(s.i18n_key())}</button> }
                                }).collect_view()}
                            </div>
                            <h3 class="section">{move || t("admin.history")}</h3>
                            <div class="lines">
                                {d.log.iter().map(|l| view! {
                                    <div class="line"><span class="num">{local_time(&l.at)}" · "{l.actor.clone()}</span><span>{status_pill(l.to)}{l.note.clone().map(|n| format!(" {n}"))}</span></div>
                                }).collect_view()}
                            </div>
                        </div>
                    </div>
                    <div class="stack">
                        <div class="card">
                            <h3 class="section">{move || t("admin.files")}</h3>
                            {match d.files_status.as_str() {
                                "pending" => view! { <div class="note">{move || t("admin.files_pending")}</div> }.into_any(),
                                "failed" => view! { <div class="note bad">{move || t("admin.files_failed")}</div> }.into_any(),
                                _ => ().into_any(),
                            }}
                            <div class="mock-pair">{mockups.into_iter().map(|u| view! { <img src=u alt=""/> }).collect_view()}</div>
                            {downloads.into_iter().map(|f| {
                                let label = file_label(&f.kind);
                                view! { <a class="btn btn-ghost btn-sm" href=f.url download="">{move || t(label)}<span class="muted num">{format!("{:.1} MB", f.bytes as f64 / 1_048_576.0)}</span></a> }
                            }).collect_view()}
                            <button class="link-btn" on:click=rerender>{move || t("admin.retry_files")}</button>
                        </div>
                        <div class="card">
                            <h3 class="section">{move || t("admin.design_text")}</h3>
                            <p style="white-space:pre-wrap">{d.doc.all_text()}</p>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}

fn reload(adm: Adm, id: StoredValue<String>, detail: RwSignal<Option<AdminOrderDetail>>) {
    leptos::task::spawn_local(async move {
        // The page may have been left meanwhile; the signal is then gone.
        let (Some(_), Some(order_id)) = (detail.try_get_untracked(), id.try_get_value()) else { return };
        if let Ok(d) = adm.get::<AdminOrderDetail>(&format!("/api/admin/orders/{order_id}")).await {
            if d.files_status == "pending" {
                set_timeout(move || reload(adm, id, detail), std::time::Duration::from_secs(3));
            }
            let _ = detail.try_set(Some(d));
        }
    });
}

#[component]
fn Moderation(adm: Adm) -> impl IntoView {
    let items = RwSignal::new(None::<Vec<ModerationItemDto>>);
    let load = move || {
        leptos::task::spawn_local(async move {
            if let Ok(v) = adm.get("/api/admin/moderation").await {
                items.set(Some(v));
            }
        });
    };
    load();
    let decide = move |design_id: String, approve: bool| {
        leptos::task::spawn_local(async move {
            let req = ModerationDecision { approve, note: None };
            if adm.send::<_, OkDto>("POST", &format!("/api/admin/designs/{design_id}/moderation"), &req).await.is_ok() {
                load();
            }
        });
    };
    view! {
        <div class="stack">
            {move || items.get().map(|list| {
                if list.is_empty() {
                    return view! { <p class="muted">{move || t("admin.empty")}</p> }.into_any();
                }
                list.into_iter().map(|m| {
                    let (a, b) = (m.design_id.clone(), m.design_id.clone());
                    view! {
                        <div class="card">
                            <div class="row" style="align-items:flex-start">
                                {m.preview_url.clone().map(|u| view! { <img src=u alt="" style="width:180px;border-radius:14px;background:#2b2d34"/> })}
                                <div class="grow stack">
                                    <div class="note warn">{m.reason.clone().unwrap_or_default()}</div>
                                    {m.order_code.clone().map(|c| view! { <div class="num"><b>{c}</b></div> })}
                                    <p style="white-space:pre-wrap">{m.text.clone()}</p>
                                    <div class="row">
                                        <button class="btn btn-primary btn-sm" on:click=move |_| decide(a.clone(), true)>{move || t("admin.approve")}</button>
                                        <button class="btn btn-danger btn-sm" on:click=move |_| decide(b.clone(), false)>{move || t("admin.reject")}</button>
                                    </div>
                                </div>
                            </div>
                        </div>
                    }
                }).collect_view().into_any()
            })}
        </div>
    }
}

#[component]
fn Reports(adm: Adm) -> impl IntoView {
    let items = RwSignal::new(None::<Vec<ReportDto>>);
    let load = move || {
        leptos::task::spawn_local(async move {
            if let Ok(v) = adm.get("/api/admin/reports").await {
                items.set(Some(v));
            }
        });
    };
    load();
    let decide = move |id: String, remove: bool| {
        leptos::task::spawn_local(async move {
            if adm.send::<_, OkDto>("POST", &format!("/api/admin/reports/{id}"), &ReportDecision { remove, note: None }).await.is_ok() {
                load();
            }
        });
    };
    view! {
        <div class="stack">
            {move || items.get().map(|list| {
                if list.is_empty() {
                    return view! { <p class="muted">{move || t("admin.empty")}</p> }.into_any();
                }
                list.into_iter().map(|r| {
                    let (a, b) = (r.id.clone(), r.id.clone());
                    let open = r.status == "open";
                    let kind = if r.kind == ReportKind::Trademark { "legal.takedown" } else { "legal.report_title" };
                    view! {
                        <div class="card">
                            <div class="spread"><b>{move || t(kind)}</b><span class="pill">{r.status.clone()}</span></div>
                            <div class="muted num">{local_time(&r.created_at)}" · "{r.design_slug.clone().unwrap_or_default()}</div>
                            <p style="white-space:pre-wrap">{r.reason.clone()}</p>
                            {r.contact.clone().map(|c| view! { <p class="muted">{c}</p> })}
                            <img src=format!("/api/admin/designs/{}/preview", r.design_id) alt="" style="width:180px;border-radius:14px;background:#2b2d34"/>
                            {open.then(|| view! {
                                <div class="row">
                                    <button class="btn btn-danger btn-sm" on:click=move |_| decide(a.clone(), true)>{move || t("admin.remove_design")}</button>
                                    <button class="btn btn-ghost btn-sm" on:click=move |_| decide(b.clone(), false)>{move || t("admin.dismiss")}</button>
                                </div>
                            })}
                        </div>
                    }
                }).collect_view().into_any()
            })}
        </div>
    }
}

/// Whole soʻm in the form, tiyin in the API.
fn sum_input(value: Tiyin, on_change: impl Fn(Tiyin) + 'static) -> impl IntoView {
    view! {
        <input type="number" min="0" step="1000" class="num" prop:value=value.to_sum_floor().to_string() on:change=move |ev| {
            let v: i64 = event_target_value(&ev).parse().unwrap_or(0);
            on_change(Tiyin::from_sum(v.max(0)).unwrap_or(Tiyin::ZERO));
        }/>
    }
}

fn num_input<T: std::str::FromStr + ToString + Copy + 'static>(value: T, step: &'static str, on_change: impl Fn(T) + 'static) -> impl IntoView {
    view! {
        <input type="number" min="0" step=step class="num" prop:value=value.to_string() on:change=move |ev| {
            if let Ok(v) = event_target_value(&ev).parse::<T>() {
                on_change(v);
            }
        }/>
    }
}

#[component]
fn Prices(adm: Adm) -> impl IntoView {
    let cfg = RwSignal::new(None::<PricingConfigDto>);
    let blocklist = RwSignal::new(String::new());
    leptos::task::spawn_local(async move {
        if let Ok(c) = adm.get::<PricingConfigDto>("/api/admin/pricing").await {
            cfg.set(Some(c));
        }
        if let Ok(b) = adm.get::<BlocklistDto>("/api/admin/blocklist").await {
            blocklist.set(b.terms.join("\n"));
        }
    });
    // Edit a copy in place; nothing is sent until "Save".
    let edit = move |f: Box<dyn FnOnce(&mut PricingConfigDto)>| {
        cfg.update(|c| {
            if let Some(c) = c {
                f(c);
            }
        })
    };
    let save = move |_| {
        let Some(c) = cfg.get_untracked() else { return };
        leptos::task::spawn_local(async move {
            if adm.send::<_, OkDto>("PUT", "/api/admin/pricing", &c).await.is_ok() {
                adm.ctx.show_toast(t("admin.saved"), false);
            }
        });
    };
    let save_blocklist = move |_| {
        let terms: Vec<String> = blocklist.get_untracked().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        leptos::task::spawn_local(async move {
            if adm.send::<_, OkDto>("PUT", "/api/admin/blocklist", &BlocklistDto { terms }).await.is_ok() {
                adm.ctx.show_toast(t("admin.saved"), false);
            }
        });
    };

    view! {
        // Rebuilt after structural changes (rows added/removed); typing in a
        // field only fires on `change`, so focus is not lost mid-edit.
        {move || cfg.get().map(|c| view! {
            <div class="cols">
                <div class="stack">
                    <div class="card">
                        <h3 class="section">{move || t("admin.garment_prices")}</h3>
                        <table class="table"><tbody>
                            {c.garments.iter().enumerate().map(|(i, g)| view! {
                                <tr><td>{g.name.clone()}</td><td>{sum_input(g.base_price, move |v| edit(Box::new(move |c| c.garments[i].base_price = v)))}</td></tr>
                            }).collect_view()}
                        </tbody></table>
                    </div>
                    <div class="card">
                        <h3 class="section">{move || t("admin.print_tiers")}</h3>
                        <table class="table">
                            <thead><tr><th>{move || t("admin.max_area")}</th><th>{move || t("admin.price")}</th><th></th></tr></thead>
                            <tbody>
                                {c.tiers.iter().enumerate().map(|(i, tier)| view! {
                                    <tr>
                                        <td>{num_input(tier.max_area_cm2, "50", move |v: u32| edit(Box::new(move |c| c.tiers[i].max_area_cm2 = v)))}</td>
                                        <td>{sum_input(tier.price, move |v| edit(Box::new(move |c| c.tiers[i].price = v)))}</td>
                                        <td><button class="link-btn" on:click=move |_| edit(Box::new(move |c| { c.tiers.remove(i); }))>"×"</button></td>
                                    </tr>
                                }).collect_view()}
                            </tbody>
                        </table>
                        <button class="link-btn" on:click=move |_| edit(Box::new(|c| {
                            let next = c.tiers.iter().map(|t| t.max_area_cm2).max().unwrap_or(0) + 500;
                            c.tiers.push(AreaTier { max_area_cm2: next, price: Tiyin::ZERO });
                        }))>{move || t("admin.add_row")}</button>
                    </div>
                    <div class="card">
                        <h3 class="section">{move || t("admin.discounts")}</h3>
                        <table class="table">
                            <thead><tr><th>{move || t("admin.min_qty")}</th><th>{move || t("admin.percent")}</th><th></th></tr></thead>
                            <tbody>
                                {c.discounts.iter().enumerate().map(|(i, d)| view! {
                                    <tr>
                                        <td>{num_input(d.min_qty, "1", move |v: u32| edit(Box::new(move |c| c.discounts[i].min_qty = v)))}</td>
                                        // Shown in percent, stored in basis points.
                                        <td>{num_input(d.percent_bp as f32 / 100.0, "0.5", move |v: f32| edit(Box::new(move |c| c.discounts[i].percent_bp = (v * 100.0).round().clamp(0.0, 10_000.0) as u32)))}</td>
                                        <td><button class="link-btn" on:click=move |_| edit(Box::new(move |c| { c.discounts.remove(i); }))>"×"</button></td>
                                    </tr>
                                }).collect_view()}
                            </tbody>
                        </table>
                        <button class="link-btn" on:click=move |_| edit(Box::new(|c| {
                            let next = c.discounts.iter().map(|d| d.min_qty).max().unwrap_or(1) + 10;
                            c.discounts.push(QuantityDiscount { min_qty: next, percent_bp: 0 });
                        }))>{move || t("admin.add_row")}</button>
                    </div>
                    <div class="card">
                        <h3 class="section">{move || t("admin.print_areas")}</h3>
                        <p class="muted small">{move || t("admin.position")}</p>
                        <div class="table-wrap"><table class="table">
                            <thead><tr><th></th><th>{move || t("admin.width_cm")}</th><th>{move || t("admin.height_cm")}</th><th>"x"</th><th>"y"</th><th>"w"</th><th>"h"</th></tr></thead>
                            <tbody>
                                {c.areas.iter().enumerate().map(|(i, a)| {
                                    let p = a.area;
                                    view! {
                                        <tr>
                                            <td>{format!("{} {}", a.garment, p.side.as_str())}</td>
                                            <td>{num_input(p.width_cm, "1", move |v: f32| edit(Box::new(move |c| c.areas[i].area.width_cm = v)))}</td>
                                            <td>{num_input(p.height_cm, "1", move |v: f32| edit(Box::new(move |c| c.areas[i].area.height_cm = v)))}</td>
                                            <td>{num_input(p.mock_x, "0.005", move |v: f32| edit(Box::new(move |c| c.areas[i].area.mock_x = v)))}</td>
                                            <td>{num_input(p.mock_y, "0.005", move |v: f32| edit(Box::new(move |c| c.areas[i].area.mock_y = v)))}</td>
                                            <td>{num_input(p.mock_w, "0.005", move |v: f32| edit(Box::new(move |c| c.areas[i].area.mock_w = v)))}</td>
                                            <td>{num_input(p.mock_h, "0.005", move |v: f32| edit(Box::new(move |c| c.areas[i].area.mock_h = v)))}</td>
                                        </tr>
                                    }
                                }).collect_view()}
                            </tbody>
                        </table></div>
                    </div>
                    <button class="btn btn-primary" on:click=save>{move || t("common.save")}</button>
                </div>
                <div class="stack">
                    <div class="card">
                        <h3 class="section">{move || t("admin.delivery_fees")}</h3>
                        <table class="table"><tbody>
                            {c.regions.iter().enumerate().map(|(i, r)| view! {
                                <tr><td>{r.name.uz.clone()}</td><td>{sum_input(r.delivery_fee, move |v| edit(Box::new(move |c| c.regions[i].delivery_fee = v)))}</td></tr>
                            }).collect_view()}
                        </tbody></table>
                    </div>
                    <div class="card">
                        <h3 class="section">{move || t("admin.blocklist")}</h3>
                        <p class="muted small">{move || t("admin.blocklist_hint")}</p>
                        <textarea class="textarea" rows="8" prop:value=move || blocklist.get() on:input=move |ev| blocklist.set(event_target_value(&ev))></textarea>
                        <button class="btn btn-ghost btn-sm" on:click=save_blocklist>{move || t("common.save")}</button>
                    </div>
                </div>
            </div>
        })}
    }
}

#[component]
fn Audit(adm: Adm) -> impl IntoView {
    let rows = RwSignal::new(Vec::<AuditEntryDto>::new());
    leptos::task::spawn_local(async move {
        if let Ok(v) = adm.get("/api/admin/audit").await {
            rows.set(v);
        }
    });
    view! {
        <div class="table-wrap">
            <table class="table">
                <tbody>
                    {move || rows.get().into_iter().map(|r| view! {
                        <tr>
                            <td class="num">{local_time(&r.at)}</td>
                            <td>{r.actor}</td>
                            <td><b>{r.action}</b></td>
                            <td>{format!("{} {}", r.entity, r.entity_id.unwrap_or_default())}</td>
                            <td class="muted small" style="max-width:360px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap">{if r.detail == serde_json::json!({}) { String::new() } else { r.detail.to_string() }}</td>
                        </tr>
                    }).collect_view()}
                </tbody>
            </table>
        </div>
    }
}
