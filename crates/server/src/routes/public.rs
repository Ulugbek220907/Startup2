//! Customer-facing API: catalog, uploads, designs, pricing, orders.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use design::{DesignDoc, Side};
use domain::api::*;
use domain::order::{DeliveryMethod, OrderStatus};
use domain::pricing::{self, PricingError, Quote, QuoteInput};
use domain::{phone, Lang, Tiyin};
use uuid::Uuid;

use super::{public_order, random_code};
use crate::auth::{rate_limit, sha256_hex, ClientIp};
use crate::error::{AppError, AppResult};
use crate::render_svc;
use crate::state::AppState;
use crate::telegram;

const HOUR: i32 = 3600;

pub async fn catalog(State(state): State<AppState>) -> AppResult<Json<CatalogDto>> {
    let table = db::catalog::price_table(&state.pool).await?;
    let pickup = db::catalog::setting(&state.pool, "pickup_address").await?;
    Ok(Json(CatalogDto {
        brand: state.cfg.brand.clone(),
        garments: db::catalog::garments(&state.pool).await?,
        regions: db::catalog::regions(&state.pool).await?,
        price_tiers: table.tiers,
        quantity_discounts: table.discounts,
        pickup_address: pickup.and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default(),
        max_upload_bytes: state.cfg.max_upload_bytes as u64,
        max_quantity: pricing::MAX_QUANTITY,
        tryon_ttl_hours: state.cfg.tryon.ttl_hours as u32,
        telegram_bot: state.cfg.telegram.as_ref().and_then(|t| t.bot_username.clone()),
    }))
}

// -------------------------------------------------------------- assets

/// Store an image as a new asset (original + editor preview).
async fn store_asset(state: &AppState, img: image::DynamicImage) -> AppResult<AssetDto> {
    let (original, preview) = tokio::task::spawn_blocking(move || {
        let original = crate::uploads::encode_png(&img)?;
        // The editor and mockups only need a modest preview; the full
        // original is used for the print file.
        let small = if img.width().max(img.height()) > 1024 { img.thumbnail(1024, 1024) } else { img.clone() };
        let preview = crate::uploads::encode_png(&small)?;
        Ok::<_, crate::uploads::UploadError>((original, preview, img.width(), img.height()))
    })
    .await?
    .map(|(o, p, w, h)| ((o, w, h), p))
    .map_err(|e| AppError::bad_request(e.code(), "image could not be processed"))?;
    let (original, width, height) = original;

    let id = Uuid::new_v4();
    let storage_key = format!("assets/{id}/original.png");
    let preview_key = format!("assets/{id}/preview.png");
    let sha = sha256_hex(&original);
    let bytes = original.len() as i64;
    state.store.put(&storage_key, Bytes::from(original), "image/png").await?;
    state.store.put(&preview_key, Bytes::from(preview), "image/png").await?;
    db::designs::insert_asset(
        &state.pool,
        &db::designs::NewAsset {
            id,
            sha256: &sha,
            mime: "image/png",
            width: width as i32,
            height: height as i32,
            bytes,
            storage_key: &storage_key,
            preview_key: &preview_key,
        },
    )
    .await?;
    state.metrics.uploads.fetch_add(1, Ordering::Relaxed);
    Ok(AssetDto { id: id.to_string(), width, height, preview_url: format!("/api/assets/{id}/preview") })
}

/// `POST /api/assets` with the raw image bytes as the body.
pub async fn upload_asset(State(state): State<AppState>, ClientIp(ip): ClientIp, body: Bytes) -> AppResult<Json<AssetDto>> {
    rate_limit(&state, "upload", &ip, HOUR, 40).await?;
    if body.len() > state.cfg.max_upload_bytes {
        return Err(AppError::bad_request("upload_too_large", "file is too large"));
    }
    let img = crate::uploads::decode(body).await.map_err(|e| AppError::bad_request(e.code(), "image was rejected"))?;
    Ok(Json(store_asset(&state, img).await?))
}

/// `POST /api/assets/{id}/remove-bg`: returns a new asset without background.
pub async fn remove_background(State(state): State<AppState>, ClientIp(ip): ClientIp, Path(id): Path<Uuid>) -> AppResult<Json<AssetDto>> {
    rate_limit(&state, "removebg", &ip, HOUR, 20).await?;
    let row = db::designs::asset(&state.pool, id).await?.ok_or_else(AppError::not_found)?;
    let bytes = state.store.get(&row.storage_key).await?.ok_or_else(AppError::not_found)?;
    let img = crate::uploads::decode(bytes).await.map_err(|e| AppError::bad_request(e.code(), "image was rejected"))?;
    let out = state.bg.remove(img.to_rgba8()).await.map_err(AppError::internal)?;
    Ok(Json(store_asset(&state, image::DynamicImage::ImageRgba8(out)).await?))
}

fn png_response(bytes: Bytes, cache: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, cache),
            // Stops browsers from guessing a different type for user content.
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response()
}

pub async fn asset_preview(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Response> {
    let row = db::designs::asset(&state.pool, id).await?.ok_or_else(AppError::not_found)?;
    let bytes = state.store.get(&row.preview_key).await?.ok_or_else(AppError::not_found)?;
    // Asset ids are never reused, so the preview can be cached forever.
    Ok(png_response(bytes, "public, max-age=31536000, immutable"))
}

// ------------------------------------------------------------- designs

fn validated(state: &AppState, doc: &DesignDoc) -> AppResult<()> {
    design::validate::validate(doc, state.cfg.allow_foil).map_err(|e| AppError::bad_request("invalid_design", e.to_string()))
}

fn doc_json(doc: &DesignDoc) -> AppResult<serde_json::Value> {
    serde_json::to_value(doc).map_err(AppError::internal)
}

pub async fn create_design(State(state): State<AppState>, ClientIp(ip): ClientIp, Json(req): Json<SaveDesignRequest>) -> AppResult<Json<DesignSavedDto>> {
    rate_limit(&state, "design", &ip, HOUR, 120).await?;
    validated(&state, &req.doc)?;
    let slug = random_code(10)?.to_lowercase();
    let edit_token = crate::auth::random_token(24)?;
    db::designs::insert_design(&state.pool, &slug, &sha256_hex(edit_token.as_bytes()), req.doc.version as i32, &doc_json(&req.doc)?, &req.doc.garment).await?;
    Ok(Json(DesignSavedDto { slug, edit_token }))
}

async fn visible_design(state: &AppState, slug: &str) -> AppResult<db::designs::DesignRow> {
    let row = db::designs::design_by_slug(&state.pool, slug).await?.ok_or_else(AppError::not_found)?;
    // A design taken down by moderation is gone as far as the public goes.
    if row.moderation_status == "rejected" {
        return Err(AppError::not_found());
    }
    Ok(row)
}

pub async fn get_design(State(state): State<AppState>, Path(slug): Path<String>) -> AppResult<Json<DesignDto>> {
    let row = visible_design(&state, &slug).await?;
    let doc = design::migrate(row.doc).map_err(AppError::internal)?;
    Ok(Json(DesignDto { slug: row.slug, doc }))
}

pub async fn update_design(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    headers: HeaderMap,
    Json(req): Json<SaveDesignRequest>,
) -> AppResult<Json<OkDto>> {
    validated(&state, &req.doc)?;
    let token = headers.get("x-edit-token").and_then(|v| v.to_str().ok()).ok_or_else(|| AppError::forbidden("edit token required"))?;
    let updated = db::designs::update_design(&state.pool, &slug, &sha256_hex(token.as_bytes()), &doc_json(&req.doc)?, &req.doc.garment).await?;
    if !updated {
        // Same answer for "no such design" and "wrong token".
        return Err(AppError::forbidden("design cannot be edited with this token"));
    }
    Ok(Json(OkDto { ok: true }))
}

pub async fn render_design(State(state): State<AppState>, ClientIp(ip): ClientIp, Path(slug): Path<String>) -> AppResult<Json<JobDto>> {
    rate_limit(&state, "render", &ip, HOUR, 60).await?;
    let row = visible_design(&state, &slug).await?;
    let id = db::jobs::enqueue(&state.pool, db::jobs::RENDER_MOCKUP, &serde_json::json!({ "design_id": row.id }), 0, 3).await?;
    state.job_notify.notify_one();
    Ok(Json(JobDto { id: id.to_string(), status: JobState::Queued }))
}

pub async fn design_mockup(State(state): State<AppState>, Path((slug, side)): Path<(String, String)>) -> AppResult<Response> {
    let side = Side::parse(side.trim_end_matches(".png")).ok_or_else(AppError::not_found)?;
    let row = visible_design(&state, &slug).await?;
    let bytes = state.store.get(&format!("mockups/{}/{}.png", row.id, side.as_str())).await?.ok_or_else(AppError::not_found)?;
    // Designs can be edited, so let clients revalidate.
    Ok(png_response(bytes, "no-cache"))
}

pub async fn job_status(State(state): State<AppState>, Path(id): Path<Uuid>) -> AppResult<Json<JobDto>> {
    let status = db::jobs::status(&state.pool, id).await?.ok_or_else(AppError::not_found)?;
    let status = match status.as_str() {
        "queued" => JobState::Queued,
        "running" => JobState::Running,
        "done" => JobState::Done,
        _ => JobState::Failed,
    };
    Ok(Json(JobDto { id: id.to_string(), status }))
}

fn clean_text(s: &str, max_chars: usize) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\n').take(max_chars).collect::<String>().trim().to_string()
}

fn opt_text(s: &Option<String>, max_chars: usize) -> Option<String> {
    s.as_deref().map(|v| clean_text(v, max_chars)).filter(|v| !v.is_empty())
}

pub async fn report_design(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Path(slug): Path<String>,
    Json(req): Json<ReportRequest>,
) -> AppResult<Json<OkDto>> {
    rate_limit(&state, "report", &ip, HOUR, 10).await?;
    let reason = clean_text(&req.reason, 1000);
    let contact = opt_text(&req.contact, 200);
    if reason.chars().count() < 5 {
        return Err(AppError::invalid("please describe the problem"));
    }
    // A takedown claim is only actionable if we can reach the claimant.
    if req.kind == ReportKind::Trademark && contact.is_none() {
        return Err(AppError::invalid("contact details are required for a rights claim"));
    }
    let row = visible_design(&state, &slug).await?;
    let kind = match req.kind {
        ReportKind::Report => "report",
        ReportKind::Trademark => "trademark",
    };
    let mut tx = state.pool.begin().await?;
    db::designs::insert_report(&mut *tx, row.id, kind, &reason, contact.as_deref()).await?;
    db::designs::flag_design(&mut *tx, row.id, &format!("reported: {kind}")).await?;
    db::jobs::enqueue(&mut *tx, db::jobs::RENDER_MOCKUP, &serde_json::json!({ "design_id": row.id }), 0, 3).await?;
    db::jobs::enqueue(&mut *tx, db::jobs::NOTIFY_ADMIN, &serde_json::json!({ "report_design_slug": row.slug, "kind": kind }), 0, 5).await?;
    tx.commit().await?;
    state.job_notify.notify_one();
    Ok(Json(OkDto { ok: true }))
}

// ------------------------------------------------------------- pricing

pub struct Priced {
    pub quote: Quote,
    pub garment: GarmentDto,
    pub color: ColorDto,
    pub region: Option<RegionDto>,
}

/// Price a design from database prices only. Nothing price-related is ever
/// taken from the request.
pub async fn price(state: &AppState, doc: &DesignDoc, quantity: u32, delivery: &DeliveryChoice) -> AppResult<Priced> {
    validated(state, doc)?;
    let garments = db::catalog::garments(&state.pool).await?;
    let garment = garments.into_iter().find(|g| g.slug == doc.garment).ok_or_else(|| AppError::bad_request("invalid_design", "unknown garment"))?;
    let color = garment.color(&doc.color).cloned().ok_or_else(|| AppError::bad_request("invalid_design", "unknown colour"))?;

    let (region, table_fee) = match delivery.method {
        DeliveryMethod::Pickup => (None, Tiyin::ZERO),
        DeliveryMethod::Delivery => {
            let slug = delivery.region.as_deref().ok_or_else(|| AppError::bad_request("region_required", "choose a region"))?;
            let region = db::catalog::regions(&state.pool)
                .await?
                .into_iter()
                .find(|r| r.slug == slug)
                .ok_or_else(|| AppError::bad_request("region_required", "unknown region"))?;
            let fee = region.delivery_fee;
            (Some(region), fee)
        }
    };
    let delivery_fee = match &region {
        Some(r) => state.delivery.fee(&r.slug, table_fee).await.map_err(AppError::internal)?,
        None => Tiyin::ZERO,
    };

    let table = db::catalog::price_table(&state.pool).await?;
    let input = QuoteInput { garment_base: garment.base_price, side_areas_cm2: render_svc::side_areas_cm2(doc, &garment.areas), quantity, delivery_fee };
    let quote = pricing::quote(&input, &table).map_err(|e| match e {
        PricingError::InvalidQuantity => AppError::bad_request("invalid_quantity", e.to_string()),
        PricingError::EmptyDesign => AppError::bad_request("empty_design", e.to_string()),
        other => AppError::internal(other),
    })?;
    Ok(Priced { quote, garment, color, region })
}

pub async fn quote(State(state): State<AppState>, ClientIp(ip): ClientIp, Json(req): Json<QuoteRequest>) -> AppResult<Json<Quote>> {
    rate_limit(&state, "quote", &ip, 60, 60).await?;
    Ok(Json(price(&state, &req.doc, req.quantity, &req.delivery).await?.quote))
}

// -------------------------------------------------------------- orders

pub async fn create_order(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Json(req): Json<CreateOrderRequest>,
) -> AppResult<(StatusCode, Json<OrderCreatedDto>)> {
    rate_limit(&state, "order-ip", &ip, HOUR, 20).await?;
    let phone = phone::normalize(&req.phone).map_err(|e| AppError::bad_request("invalid_phone", e.to_string()))?;
    rate_limit(&state, "order-phone", &phone, HOUR, 6).await?;
    if !req.accept_rights {
        return Err(AppError::bad_request("rights_required", "the rights statement must be accepted"));
    }

    let priced = price(&state, &req.doc, req.quantity, &req.delivery).await?;
    if !priced.garment.sizes.contains(&req.size) {
        return Err(AppError::bad_request("size_required", "choose a size"));
    }
    let name = opt_text(&req.customer_name, 80);
    let notes = opt_text(&req.notes, 500);
    let address = opt_text(&req.address, 300);
    if req.delivery.method == DeliveryMethod::Delivery && address.as_deref().is_none_or(|a| a.chars().count() < 5) {
        return Err(AppError::bad_request("address_required", "enter the delivery address"));
    }

    // Cheap text check now, so an obviously bad design is flagged before
    // anyone can confirm the order; image checks run in the background.
    let blocklist = db::catalog::blocklist(&state.pool).await?;
    let text_hit = domain::moderation::find_blocked(&req.doc.all_text(), blocklist.iter().map(String::as_str)).map(str::to_string);

    let doc_value = doc_json(&req.doc)?;
    let quote_value = serde_json::to_value(priced.quote).map_err(AppError::internal)?;
    let areas_value = serde_json::to_value(&priced.garment.areas).map_err(AppError::internal)?;
    let lang = req.lang;
    let garment_name = priced.garment.name.get(lang).to_string();
    let color_name = priced.color.name.get(lang).to_string();

    // Order codes are random; on the (unlikely) collision try a new one.
    for _ in 0..5 {
        let code = format!("QD-{}", random_code(6)?);
        let mut tx = state.pool.begin().await?;
        let customer_id = db::orders::upsert_customer_by_phone(&mut tx, &phone, name.as_deref(), lang.code()).await?;

        // Reuse the saved design when the order is for exactly that
        // document; otherwise freeze a new design row for this order.
        let existing = match &req.design_slug {
            Some(slug) => db::designs::design_by_slug(&state.pool, slug).await?.filter(|d| d.doc == doc_value && d.moderation_status != "rejected"),
            None => None,
        };
        let design_id = match existing {
            Some(d) => d.id,
            None => {
                let slug = random_code(10)?.to_lowercase();
                // Nobody holds this token: the order's design cannot be edited.
                let token_hash = sha256_hex(crate::auth::random_token(24)?.as_bytes());
                db::designs::insert_design(&mut *tx, &slug, &token_hash, req.doc.version as i32, &doc_value, &req.doc.garment).await?
            }
        };
        if let Some(term) = &text_hit {
            db::designs::flag_design(&mut *tx, design_id, &format!("blocked word: {term}")).await?;
        }

        let order = db::orders::NewOrder {
            code: &code,
            customer_id,
            phone: &phone,
            customer_name: name.as_deref(),
            delivery_method: req.delivery.method.as_str(),
            region_slug: priced.region.as_ref().map(|r| r.slug.as_str()),
            address: address.as_deref(),
            notes: notes.as_deref(),
            lang: lang.code(),
            subtotal: priced.quote.subtotal.0,
            discount: priced.quote.discount.0,
            delivery_fee: priced.quote.delivery_fee.0,
            total: priced.quote.total.0,
            quote: &quote_value,
        };
        let item = db::orders::NewItem {
            design_id,
            design_doc: &doc_value,
            garment_slug: &priced.garment.slug,
            garment_kind: priced.garment.kind.as_str(),
            garment_name: &garment_name,
            color_slug: &priced.color.slug,
            color_name: &color_name,
            color_hex: &priced.color.hex,
            size: &req.size,
            quantity: req.quantity as i32,
            finish: "standard",
            unit_price: priced.quote.unit_price.0,
            areas: &areas_value,
        };
        let order_id = match db::orders::insert_order(&mut tx, &order, &item).await {
            Ok(id) => id,
            Err(e) if db::is_unique_violation(&e) => continue, // dropping `tx` rolls back
            Err(e) => return Err(e.into()),
        };
        // Jobs are queued in the same transaction as the order: either both
        // exist or neither does.
        let payload = serde_json::json!({ "order_id": order_id });
        db::jobs::enqueue(&mut *tx, db::jobs::MODERATE_DESIGN, &serde_json::json!({ "design_id": design_id }), 0, 5).await?;
        db::jobs::enqueue(&mut *tx, db::jobs::RENDER_ORDER_FILES, &payload, 0, 4).await?;
        db::jobs::enqueue(&mut *tx, db::jobs::NOTIFY_ADMIN, &payload, 0, 6).await?;
        db::jobs::enqueue(&mut *tx, db::jobs::NOTIFY_CUSTOMER, &serde_json::json!({ "order_id": order_id, "event": "received" }), 0, 6).await?;
        tx.commit().await?;

        state.job_notify.notify_waiters();
        state.metrics.orders_created.fetch_add(1, Ordering::Relaxed);
        tracing::info!(order = %code, phone = %phone::mask(&phone), total = priced.quote.total.0, "order created");
        return Ok((StatusCode::CREATED, Json(OrderCreatedDto { code, status: OrderStatus::New, quote: priced.quote })));
    }
    Err(AppError::internal("could not allocate an order code"))
}

fn normalise_code(code: &str) -> String {
    code.trim().to_uppercase().replace(' ', "")
}

/// Find the order for a code + phone pair. Both must match, and the same
/// error is returned whichever one is wrong.
async fn order_for(state: &AppState, code: &str, phone_raw: &str) -> AppResult<db::orders::OrderRow> {
    let phone = phone::normalize(phone_raw).map_err(|e| AppError::bad_request("invalid_phone", e.to_string()))?;
    db::orders::order_by_code_and_phone(&state.pool, &normalise_code(code), &phone).await?.ok_or_else(AppError::not_found)
}

pub async fn lookup_order(State(state): State<AppState>, ClientIp(ip): ClientIp, Json(req): Json<OrderLookupRequest>) -> AppResult<Json<OrderPublicDto>> {
    // Tight limit: this endpoint is the one that could be used to guess codes.
    rate_limit(&state, "lookup", &ip, 600, 20).await?;
    let row = order_for(&state, &req.code, &req.phone).await?;
    let log = db::orders::status_log(&state.pool, row.id).await?;
    Ok(Json(public_order(&row, &log)?))
}

// ------------------------------------------------------------- privacy

pub async fn export_data(State(state): State<AppState>, ClientIp(ip): ClientIp, Json(req): Json<PrivacyRequest>) -> AppResult<Json<serde_json::Value>> {
    rate_limit(&state, "privacy", &ip, HOUR, 10).await?;
    let proof = order_for(&state, &req.code, &req.phone).await?;
    let phone = proof.phone.clone().unwrap_or_default();
    let customer = db::orders::customer_by_phone(&state.pool, &phone).await?;
    let orders = db::orders::orders_by_phone(&state.pool, &phone, 500).await?;
    let orders: Vec<serde_json::Value> = orders
        .iter()
        .map(|o| {
            serde_json::json!({
                "code": o.code, "status": o.status, "created_at": db::ts(o.created_at), "name": o.customer_name,
                "delivery_method": o.delivery_method, "region": o.region_slug, "address": o.address, "notes": o.notes,
                "garment": o.garment_name, "color": o.color_name, "size": o.size, "quantity": o.quantity,
                "total_tiyin": o.total_tiyin, "design": o.design_doc,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "phone": phone,
        "name": customer.as_ref().and_then(|c| c.name.clone()),
        "language": customer.as_ref().map(|c| c.lang.clone()),
        "telegram_linked": customer.as_ref().is_some_and(|c| c.telegram_id.is_some()),
        "orders": orders,
    })))
}

pub async fn delete_data(State(state): State<AppState>, ClientIp(ip): ClientIp, Json(req): Json<PrivacyRequest>) -> AppResult<Json<DeletedDto>> {
    rate_limit(&state, "privacy", &ip, HOUR, 10).await?;
    let proof = order_for(&state, &req.code, &req.phone).await?;
    let phone = proof.phone.clone().unwrap_or_default();
    // An order that still has to be made and delivered needs its contact
    // details; it must be finished or cancelled first.
    if db::orders::count_active_orders(&state.pool, &phone).await? > 0 {
        return Err(AppError::new(StatusCode::CONFLICT, "active_orders", "active orders must be finished or cancelled first"));
    }
    let mut tx = state.pool.begin().await?;
    let n = db::orders::anonymise_customer(&mut tx, &phone).await?;
    db::auth::audit(&mut *tx, "customer", "privacy_delete", "customer", None, &serde_json::json!({ "orders_anonymised": n }), Some(&ip)).await?;
    tx.commit().await?;
    Ok(Json(DeletedDto { deleted: true }))
}

// ------------------------------------------------------------ telegram

/// `POST /api/telegram/me`: verify Mini App initData and return the user's
/// language, linked phone and orders.
pub async fn telegram_me(State(state): State<AppState>, ClientIp(ip): ClientIp, Json(req): Json<TelegramAuthRequest>) -> AppResult<Json<TelegramMeDto>> {
    rate_limit(&state, "tg", &ip, 60, 30).await?;
    let tg = state.cfg.telegram.as_ref().ok_or_else(|| AppError::new(StatusCode::SERVICE_UNAVAILABLE, "telegram_unavailable", "telegram is not configured"))?;
    let user = telegram::verify_init_data(&req.init_data, &tg.bot_token, chrono::Utc::now().timestamp()).map_err(|_| AppError::unauthorized())?;
    let detected = Lang::detect(user.language_code.as_deref().unwrap_or("uz"));
    db::orders::upsert_telegram_user(&state.pool, user.id, None, user.first_name.as_deref(), detected.code()).await?;
    let customer = db::orders::customer_by_telegram(&state.pool, user.id).await?;
    let phone = customer.as_ref().and_then(|c| c.phone.clone());
    let orders = match &phone {
        Some(p) => db::orders::orders_by_phone(&state.pool, p, 30).await?,
        None => vec![],
    };
    Ok(Json(TelegramMeDto {
        lang: customer.as_ref().and_then(|c| Lang::parse(&c.lang)).unwrap_or(detected),
        phone,
        first_name: user.first_name,
        orders: orders
            .iter()
            .filter_map(|o| {
                Some(OrderSummaryDto {
                    code: o.code.clone(),
                    status: OrderStatus::parse(&o.status)?,
                    created_at: db::ts(o.created_at),
                    total: Tiyin(o.total_tiyin),
                })
            })
            .collect(),
    }))
}

/// Shared by the try-on route: the mockup for a saved design.
pub async fn design_garment_mockup(state: &AppState, design: &db::designs::DesignRow, side: Side, width: u32) -> AppResult<Vec<u8>> {
    let doc = design::migrate(design.doc.clone()).map_err(AppError::internal)?;
    let garments = db::catalog::garments(&state.pool).await?;
    let garment = garments.iter().find(|g| g.slug == doc.garment).ok_or_else(AppError::not_found)?;
    let color = garment.color(&doc.color).and_then(|c| design::Color::parse(&c.hex)).ok_or_else(AppError::not_found)?;
    let area = garment.area(side).ok_or_else(AppError::not_found)?;
    let assets = render_svc::load_assets(state, &doc).await?;
    render_svc::mockup_png(state, Arc::new(doc), Arc::new(assets), side, garment.kind, color, render_svc::geom(area), width).await
}
