//! Privacy notice, data export/delete and the report / takedown form.

use domain::api::{DeletedDto, OkDto, PrivacyRequest, ReportKind, ReportRequest};
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api;
use crate::app::TopBar;
use crate::state::{ctx, store_get, t, tf};

/// Offer a JSON document as a file download.
fn download_json(name: &str, json: &str) {
    let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(json));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("application/json");
    let Ok(blob) = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts) else { return };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return };
    if let Some(a) =
        web_sys::window().and_then(|w| w.document()).and_then(|d| d.create_element("a").ok()).and_then(|e| e.dyn_into::<web_sys::HtmlAnchorElement>().ok())
    {
        a.set_href(&url);
        a.set_download(name);
        a.click();
    }
    let _ = web_sys::Url::revoke_object_url(&url);
}

/// Accept a share link (`https://…/d/abc123`) or a bare slug.
fn slug_from(input: &str) -> String {
    input.trim().trim_end_matches('/').rsplit('/').next().unwrap_or("").to_lowercase()
}

#[component]
pub fn LegalPage() -> impl IntoView {
    let ctx = ctx();
    let hours = move || ctx.catalog.with(|c| c.as_ref().map(|c| c.tryon_ttl_hours).unwrap_or(24)).to_string();

    // Proof of identity for export/delete.
    let code = RwSignal::new(String::new());
    let phone = RwSignal::new(String::new());
    let proof = move || PrivacyRequest { phone: format!("+998{}", phone.get_untracked()), code: code.get_untracked().trim().to_string() };
    let busy = RwSignal::new(false);
    let confirm_delete = RwSignal::new(false);

    let export = move |_| {
        busy.set(true);
        leptos::task::spawn_local(async move {
            let r = api::post::<_, serde_json::Value>("/api/me/export", &proof()).await;
            busy.set(false);
            match r {
                Ok(v) => download_json("qiyiqdoppi-data.json", &serde_json::to_string_pretty(&v).unwrap_or_default()),
                Err(e) if e.status == 404 => ctx.show_toast(t("track.not_found"), true),
                Err(e) => ctx.show_error(&e),
            }
        });
    };
    let delete = move |_| {
        // Two taps: the first arms the button, the second deletes.
        if !confirm_delete.get_untracked() {
            return confirm_delete.set(true);
        }
        busy.set(true);
        leptos::task::spawn_local(async move {
            let r = api::post::<_, DeletedDto>("/api/me/delete", &proof()).await;
            busy.set(false);
            confirm_delete.set(false);
            match r {
                Ok(_) => {
                    ctx.orders.set(vec![]);
                    crate::state::store_set("qd.orders", &Vec::<crate::state::SavedOrder>::new());
                    crate::state::store_set("qd.contact", &serde_json::json!({}));
                    ctx.show_toast(t("legal.delete_done"), false);
                }
                Err(e) if e.code == "active_orders" => ctx.show_toast(t("legal.delete_active"), true),
                Err(e) if e.status == 404 => ctx.show_toast(t("track.not_found"), true),
                Err(e) => ctx.show_error(&e),
            }
        });
    };

    // Report form. The editor stores the slug of a shared design here when
    // "Report" is tapped.
    let link = RwSignal::new(store_get::<String>("qd.report").unwrap_or_default());
    let reason = RwSignal::new(String::new());
    let contact = RwSignal::new(String::new());
    let trademark = RwSignal::new(false);
    let send_report = move |_| {
        let slug = slug_from(&link.get_untracked());
        if slug.is_empty() || reason.get_untracked().trim().chars().count() < 5 || (trademark.get_untracked() && contact.get_untracked().trim().is_empty()) {
            return ctx.show_toast(t("error.invalid_input"), true);
        }
        let req = ReportRequest {
            kind: if trademark.get_untracked() { ReportKind::Trademark } else { ReportKind::Report },
            reason: reason.get_untracked(),
            contact: Some(contact.get_untracked()).filter(|c| !c.trim().is_empty()),
        };
        busy.set(true);
        leptos::task::spawn_local(async move {
            let r = api::post::<_, OkDto>(&format!("/api/designs/{slug}/report"), &req).await;
            busy.set(false);
            match r {
                Ok(_) => {
                    reason.set(String::new());
                    link.set(String::new());
                    ctx.show_toast(t("legal.sent"), false);
                }
                Err(e) => ctx.show_error(&e),
            }
        });
    };

    view! {
        <TopBar title=Signal::derive(move || t("legal.title").to_string())/>
        <main class="page">
            <div class="card">
                <p>{move || t("legal.rights_notice")}</p>
                <p>{move || tf("legal.data", &[("hours", &hours())])}</p>
            </div>

            <div class="card">
                <p class="muted">{move || t("legal.proof_hint")}</p>
                <label class="field">
                    <span>{move || t("order.code")}</span>
                    <input class="input num" type="text" maxlength="12" placeholder="QD-XXXXXX" prop:value=move || code.get() on:input=move |ev| code.set(event_target_value(&ev).to_uppercase())/>
                </label>
                <label class="field">
                    <span>{move || t("order.phone")}</span>
                    <div class="phone">
                        <span class="prefix num">"+998"</span>
                        <input class="input num" type="tel" inputmode="numeric" maxlength="12" prop:value=move || phone.get() on:input=move |ev| phone.set(event_target_value(&ev))/>
                    </div>
                </label>
                <button class="btn btn-ghost" disabled=move || busy.get() on:click=export>{move || t("legal.export")}</button>
                <button class="btn btn-danger" disabled=move || busy.get() on:click=delete>
                    {move || if confirm_delete.get() { format!("{}?", t("legal.delete")) } else { t("legal.delete").to_string() }}
                </button>
            </div>

            <div class="card">
                <h3 class="section">{move || t("legal.report_title")}</h3>
                <label class="field">
                    <span>{move || t("editor.share_link")}</span>
                    <input class="input" type="text" maxlength="200" prop:value=move || link.get() on:input=move |ev| link.set(event_target_value(&ev))/>
                </label>
                <label class="check">
                    <input type="checkbox" prop:checked=move || trademark.get() on:change=move |ev| trademark.set(event_target_checked(&ev))/>
                    <span>{move || t("legal.takedown")}</span>
                </label>
                <label class="field">
                    <span>{move || t("legal.report_reason")}</span>
                    <textarea class="textarea" maxlength="1000" prop:value=move || reason.get() on:input=move |ev| reason.set(event_target_value(&ev))></textarea>
                </label>
                <label class="field">
                    <span>{move || t("legal.contact")}</span>
                    <input class="input" type="text" maxlength="200" prop:value=move || contact.get() on:input=move |ev| contact.set(event_target_value(&ev))/>
                </label>
                <button class="btn btn-ghost" disabled=move || busy.get() on:click=send_report>{move || t("legal.send")}</button>
            </div>
        </main>
    }
}
