//! Admin API. Every handler except `login` takes an [`AdminSession`], which
//! enforces the session cookie and (for writes) the CSRF token.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use design::Side;
use domain::api::*;
use domain::order::{check_transition, DeliveryMethod, ModerationStatus, OrderStatus, TransitionError};
use domain::Lang;
use serde::Deserialize;
use uuid::Uuid;

use super::{admin_row, parse_quote, parse_status};
use crate::auth::{self, rate_limit, AdminSession, ClientIp};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

// ---------------------------------------------------------------- auth

pub async fn login(State(state): State<AppState>, ClientIp(ip): ClientIp, Json(req): Json<LoginRequest>) -> AppResult<Response> {
    // Limits per IP and per username slow down password guessing from one
    // address and distributed guessing against one account.
    rate_limit(&state, "login-ip", &ip, 900, 10).await?;
    let username: String = req.username.trim().chars().take(64).collect();
    rate_limit(&state, "login-user", &username.to_lowercase(), 900, 10).await?;

    let admin = db::auth::admin_by_username(&state.pool, &username).await?;
    // Verify against a dummy hash when the user does not exist, so response
    // time does not reveal which usernames are real.
    const DUMMY: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$Zm9vYmFyZm9vYmFyZm9vYmFyZm9vYmFyZm9vYmFyZm8";
    let hash = admin.as_ref().map(|a| a.password_hash.clone()).unwrap_or_else(|| DUMMY.to_string());
    let password = req.password;
    let ok = tokio::task::spawn_blocking(move || auth::verify_password(&password, &hash)).await?;
    let Some(admin) = admin.filter(|_| ok) else {
        db::auth::audit(&state.pool, &username, "login_failed", "admin", None, &serde_json::json!({}), Some(&ip)).await?;
        return Err(AppError::new(StatusCode::UNAUTHORIZED, "wrong_login", "wrong username or password"));
    };

    let token = auth::random_token(32)?;
    let csrf = auth::random_token(24)?;
    db::auth::insert_session(&state.pool, admin.id, &auth::sha256_hex(token.as_bytes()), &csrf, state.cfg.session_ttl_hours).await?;
    db::auth::audit(&state.pool, &admin.username, "login", "admin", None, &serde_json::json!({}), Some(&ip)).await?;
    let cookie = auth::session_cookie(&token, state.cfg.session_ttl_hours as i64 * 3600, state.cfg.cookie_secure);
    Ok(([(header::SET_COOKIE, cookie)], Json(AdminMeDto { username: admin.username, csrf_token: csrf })).into_response())
}

pub async fn logout(State(state): State<AppState>, session: AdminSession) -> AppResult<Response> {
    db::auth::delete_session(&state.pool, &session.token_hash).await?;
    let cookie = auth::session_cookie("", 0, state.cfg.cookie_secure);
    Ok(([(header::SET_COOKIE, cookie)], Json(OkDto { ok: true })).into_response())
}

pub async fn me(session: AdminSession) -> Json<AdminMeDto> {
    Json(AdminMeDto { username: session.username, csrf_token: session.csrf_token })
}

// -------------------------------------------------------------- orders

#[derive(Deserialize)]
pub struct ListParams {
    pub status: Option<String>,
    pub q: Option<String>,
    #[serde(default)]
    pub page: i64,
}

const PAGE_SIZE: i64 = 30;

pub async fn list_orders(State(state): State<AppState>, _s: AdminSession, Query(p): Query<ListParams>) -> AppResult<Json<AdminOrderList>> {
    let status = match p.status.as_deref().filter(|s| !s.is_empty()) {
        None => None,
        Some(s) => Some(OrderStatus::parse(s).ok_or_else(|| AppError::invalid("unknown status"))?.as_str()),
    };
    let filter = db::orders::OrderFilter { status, search: p.q.as_deref(), limit: PAGE_SIZE, offset: p.page.max(0) * PAGE_SIZE };
    let (rows, total) = db::orders::list_orders(&state.pool, &filter).await?;
    Ok(Json(AdminOrderList { orders: rows.iter().map(admin_row).collect::<AppResult<_>>()?, total }))
}

async fn load_order(state: &AppState, id: Uuid) -> AppResult<db::orders::OrderRow> {
    db::orders::order_by_id(&state.pool, id).await?.ok_or_else(AppError::not_found)
}

pub async fn get_order(State(state): State<AppState>, _s: AdminSession, Path(id): Path<Uuid>) -> AppResult<Json<AdminOrderDetail>> {
    let row = load_order(&state, id).await?;
    let status = parse_status(&row.status)?;
    let files = db::orders::files(&state.pool, id).await?;
    let log = db::orders::status_log(&state.pool, id).await?;
    Ok(Json(AdminOrderDetail {
        row: admin_row(&row)?,
        color: row.color_name.clone(),
        delivery_method: DeliveryMethod::parse(&row.delivery_method).unwrap_or(DeliveryMethod::Pickup),
        region: row.region_slug.clone(),
        address: row.address.clone(),
        notes: row.notes.clone(),
        lang: Lang::parse(&row.lang).unwrap_or_default(),
        quote: parse_quote(&row.quote)?,
        files: files
            .into_iter()
            .map(|f| OrderFileDto { url: format!("/api/admin/orders/{id}/files/{}", f.kind), kind: f.kind, mime: f.mime, bytes: f.bytes })
            .collect(),
        files_status: row.files_status.clone(),
        log: log
            .into_iter()
            .filter_map(|l| {
                Some(StatusLogDto {
                    from: l.from_status.as_deref().and_then(OrderStatus::parse),
                    to: OrderStatus::parse(&l.to_status)?,
                    actor: l.actor,
                    note: l.note,
                    at: db::ts(l.created_at),
                })
            })
            .collect(),
        allowed_next: status.next_allowed().to_vec(),
        design_id: row.design_id.to_string(),
        moderation_reason: row.moderation_reason.clone(),
        doc: design::migrate(row.design_doc).map_err(AppError::internal)?,
    }))
}

pub async fn change_status(
    State(state): State<AppState>,
    session: AdminSession,
    ClientIp(ip): ClientIp,
    Path(id): Path<Uuid>,
    Json(req): Json<StatusChangeRequest>,
) -> AppResult<Json<OkDto>> {
    let row = load_order(&state, id).await?;
    let from = parse_status(&row.status)?;
    let moderation = ModerationStatus::parse(&row.moderation_status).unwrap_or(ModerationStatus::Pending);
    check_transition(from, req.to, moderation).map_err(|e| match e {
        TransitionError::NeedsModeration => AppError::new(StatusCode::CONFLICT, "needs_moderation", e.to_string()),
        TransitionError::NotAllowed { .. } => AppError::new(StatusCode::CONFLICT, "transition_not_allowed", e.to_string()),
    })?;
    let note = req.note.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(|n| n.chars().take(500).collect::<String>());

    let mut tx = state.pool.begin().await?;
    if !db::orders::change_status(&mut tx, id, from.as_str(), req.to.as_str(), &session.username, note.as_deref()).await? {
        // Someone else changed the order between our read and write.
        return Err(AppError::new(StatusCode::CONFLICT, "transition_not_allowed", "the order was changed by someone else; reload"));
    }
    db::auth::audit(
        &mut *tx,
        &session.username,
        "order_status",
        "order",
        Some(&row.code),
        &serde_json::json!({ "from": from.as_str(), "to": req.to.as_str(), "note": note }),
        Some(&ip),
    )
    .await?;
    db::jobs::enqueue(&mut *tx, db::jobs::NOTIFY_CUSTOMER, &serde_json::json!({ "order_id": id, "event": "status" }), 0, 6).await?;
    tx.commit().await?;
    state.job_notify.notify_one();
    Ok(Json(OkDto { ok: true }))
}

fn file_name(code: &str, kind: &str) -> String {
    let ext = if kind == "sheet" { "pdf" } else { "png" };
    format!("{code}-{}.{ext}", kind.replace('_', "-"))
}

pub async fn download_file(State(state): State<AppState>, _s: AdminSession, Path((id, kind)): Path<(Uuid, String)>) -> AppResult<Response> {
    let row = load_order(&state, id).await?;
    let file = db::orders::file(&state.pool, id, &kind).await?.ok_or_else(AppError::not_found)?;
    let bytes = state.store.get(&file.storage_key).await?.ok_or_else(AppError::not_found)?;
    // Mockups are shown inline in the admin page; print files and the sheet
    // download with a readable file name. The name is built from the order
    // code and a fixed kind, so it cannot contain header-breaking characters.
    let disposition = if kind.starts_with("mockup") { "inline".to_string() } else { format!("attachment; filename=\"{}\"", file_name(&row.code, &kind)) };
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, file.mime.parse().map_err(AppError::internal)?);
    headers.insert(header::CONTENT_DISPOSITION, disposition.parse().map_err(AppError::internal)?);
    headers.insert(header::CACHE_CONTROL, header::HeaderValue::from_static("private, no-store"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, header::HeaderValue::from_static("nosniff"));
    Ok((headers, bytes).into_response())
}

/// Re-queue rendering of an order's files (after a failure, or after print
/// areas were corrected).
pub async fn rerender(State(state): State<AppState>, session: AdminSession, ClientIp(ip): ClientIp, Path(id): Path<Uuid>) -> AppResult<Json<OkDto>> {
    let row = load_order(&state, id).await?;
    let mut tx = state.pool.begin().await?;
    db::orders::set_files_status(&mut *tx, id, "pending").await?;
    db::jobs::enqueue(&mut *tx, db::jobs::RENDER_ORDER_FILES, &serde_json::json!({ "order_id": id }), 0, 4).await?;
    db::auth::audit(&mut *tx, &session.username, "order_rerender", "order", Some(&row.code), &serde_json::json!({}), Some(&ip)).await?;
    tx.commit().await?;
    state.job_notify.notify_one();
    Ok(Json(OkDto { ok: true }))
}

// ---------------------------------------------------------- moderation

pub async fn moderation_queue(State(state): State<AppState>, _s: AdminSession) -> AppResult<Json<Vec<ModerationItemDto>>> {
    let rows = db::designs::moderation_queue(&state.pool).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ModerationItemDto {
                design_id: r.id.to_string(),
                status: ModerationStatus::parse(&r.moderation_status).unwrap_or(ModerationStatus::Flagged),
                reason: r.moderation_reason,
                order_code: r.order_code,
                text: design::migrate(r.doc).map(|d| d.all_text()).unwrap_or_default(),
                preview_url: Some(format!("/api/admin/designs/{}/preview", r.id)),
                created_at: db::ts(r.created_at),
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct PreviewParams {
    pub side: Option<String>,
}

/// Render a design preview on demand for the moderation queue.
pub async fn design_preview(State(state): State<AppState>, _s: AdminSession, Path(id): Path<Uuid>, Query(p): Query<PreviewParams>) -> AppResult<Response> {
    let side = p.side.as_deref().and_then(Side::parse).unwrap_or(Side::Front);
    let row = db::designs::design_by_id(&state.pool, id).await?.ok_or_else(AppError::not_found)?;
    let png = crate::routes::public::design_garment_mockup(&state, &row, side, 600).await?;
    Ok(([(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "private, no-store")], png).into_response())
}

pub async fn decide_moderation(
    State(state): State<AppState>,
    session: AdminSession,
    ClientIp(ip): ClientIp,
    Path(id): Path<Uuid>,
    Json(req): Json<ModerationDecision>,
) -> AppResult<Json<OkDto>> {
    let note = req.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    let mut tx = state.pool.begin().await?;
    if !db::designs::decide_moderation(&mut *tx, id, req.approve, note).await? {
        return Err(AppError::not_found());
    }
    let action = if req.approve { "moderation_approve" } else { "moderation_reject" };
    db::auth::audit(&mut *tx, &session.username, action, "design", Some(&id.to_string()), &serde_json::json!({ "note": note }), Some(&ip)).await?;
    tx.commit().await?;
    Ok(Json(OkDto { ok: true }))
}

pub async fn reports(State(state): State<AppState>, _s: AdminSession) -> AppResult<Json<Vec<ReportDto>>> {
    let rows = db::designs::reports(&state.pool, false).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| ReportDto {
                id: r.id.to_string(),
                design_id: r.design_id.to_string(),
                design_slug: Some(r.design_slug),
                kind: if r.kind == "trademark" { ReportKind::Trademark } else { ReportKind::Report },
                reason: r.reason,
                contact: r.contact,
                status: r.status,
                created_at: db::ts(r.created_at),
            })
            .collect(),
    ))
}

/// Resolve a report. "Remove" is the takedown: the design is rejected, so it
/// disappears from share links and can no longer be sent to production.
pub async fn decide_report(
    State(state): State<AppState>,
    session: AdminSession,
    ClientIp(ip): ClientIp,
    Path(id): Path<Uuid>,
    Json(req): Json<ReportDecision>,
) -> AppResult<Json<OkDto>> {
    let mut tx = state.pool.begin().await?;
    let status = if req.remove { "removed" } else { "dismissed" };
    let design_id = db::designs::resolve_report(&mut *tx, id, status, &session.username).await?.ok_or_else(AppError::not_found)?;
    let note = req.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
    // Removing rejects the design; dismissing approves it so the flag the
    // report raised is cleared.
    db::designs::decide_moderation(&mut *tx, design_id, !req.remove, note).await?;
    db::auth::audit(
        &mut *tx,
        &session.username,
        if req.remove { "report_takedown" } else { "report_dismiss" },
        "design",
        Some(&design_id.to_string()),
        &serde_json::json!({ "report": id, "note": note }),
        Some(&ip),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(OkDto { ok: true }))
}

// ------------------------------------------------------------- pricing

pub async fn get_pricing(State(state): State<AppState>, _s: AdminSession) -> AppResult<Json<PricingConfigDto>> {
    let garments = db::catalog::garments(&state.pool).await?;
    let table = db::catalog::price_table(&state.pool).await?;
    let lang = state.cfg.admin_lang;
    Ok(Json(PricingConfigDto {
        areas: garments.iter().flat_map(|g| g.areas.iter().map(|a| AdminPrintAreaDto { garment: g.slug.clone(), area: *a })).collect(),
        garments: garments.into_iter().map(|g| GarmentPriceDto { name: g.name.get(lang).to_string(), slug: g.slug, base_price: g.base_price }).collect(),
        tiers: table.tiers,
        discounts: table.discounts,
        regions: db::catalog::regions(&state.pool).await?,
    }))
}

fn validate_pricing(cfg: &PricingConfigDto) -> AppResult<()> {
    let table = domain::pricing::PriceTable { tiers: cfg.tiers.clone(), discounts: cfg.discounts.clone() };
    table.validate().map_err(|e| AppError::invalid(e.to_string()))?;
    if cfg.tiers.is_empty() {
        return Err(AppError::invalid("at least one print price tier is required"));
    }
    let mut areas: Vec<u32> = cfg.tiers.iter().map(|t| t.max_area_cm2).collect();
    areas.sort_unstable();
    areas.dedup();
    let mut qtys: Vec<u32> = cfg.discounts.iter().map(|d| d.min_qty).collect();
    qtys.sort_unstable();
    qtys.dedup();
    if areas.len() != cfg.tiers.len() || qtys.len() != cfg.discounts.len() {
        return Err(AppError::invalid("duplicate rows in tiers or discounts"));
    }
    // Keep amounts far away from i64 overflow: 100 billion soʻm per item.
    const MAX: i64 = 10_000_000_000_000;
    let money_ok = cfg.garments.iter().all(|g| (0..=MAX).contains(&g.base_price.0))
        && cfg.tiers.iter().all(|t| (0..=MAX).contains(&t.price.0))
        && cfg.regions.iter().all(|r| (0..=MAX).contains(&r.delivery_fee.0));
    if !money_ok {
        return Err(AppError::invalid("a price is negative or unreasonably large"));
    }
    for a in &cfg.areas {
        let p = &a.area;
        let ok = [p.width_cm, p.height_cm, p.mock_x, p.mock_y, p.mock_w, p.mock_h].iter().all(|v| v.is_finite())
            && (1.0..=100.0).contains(&p.width_cm)
            && (1.0..=100.0).contains(&p.height_cm)
            && p.mock_x >= 0.0
            && p.mock_y >= 0.0
            && p.mock_w > 0.0
            && p.mock_h > 0.0
            && p.mock_x + p.mock_w <= 1.0
            && p.mock_y + p.mock_h <= 1.0;
        if !ok {
            return Err(AppError::invalid(format!("print area for {} {} is out of range", a.garment, p.side.as_str())));
        }
    }
    Ok(())
}

pub async fn put_pricing(
    State(state): State<AppState>,
    session: AdminSession,
    ClientIp(ip): ClientIp,
    Json(cfg): Json<PricingConfigDto>,
) -> AppResult<Json<OkDto>> {
    validate_pricing(&cfg)?;
    // One transaction: customers never see a half-updated price list.
    let mut tx = state.pool.begin().await?;
    for g in &cfg.garments {
        if !db::catalog::set_garment_price(&mut *tx, &g.slug, g.base_price).await? {
            return Err(AppError::invalid(format!("unknown garment {}", g.slug)));
        }
    }
    db::catalog::replace_tiers(&mut tx, &cfg.tiers).await?;
    db::catalog::replace_discounts(&mut tx, &cfg.discounts).await?;
    for r in &cfg.regions {
        if !db::catalog::set_region_fee(&mut *tx, &r.slug, r.delivery_fee).await? {
            return Err(AppError::invalid(format!("unknown region {}", r.slug)));
        }
    }
    for a in &cfg.areas {
        if !db::catalog::set_print_area(&mut *tx, a).await? {
            return Err(AppError::invalid(format!("unknown print area {} {}", a.garment, a.area.side.as_str())));
        }
    }
    let detail = serde_json::to_value(&cfg).map_err(AppError::internal)?;
    db::auth::audit(&mut *tx, &session.username, "pricing_update", "pricing", None, &detail, Some(&ip)).await?;
    tx.commit().await?;
    Ok(Json(OkDto { ok: true }))
}

pub async fn get_blocklist(State(state): State<AppState>, _s: AdminSession) -> AppResult<Json<BlocklistDto>> {
    Ok(Json(BlocklistDto { terms: db::catalog::blocklist(&state.pool).await? }))
}

pub async fn put_blocklist(
    State(state): State<AppState>,
    session: AdminSession,
    ClientIp(ip): ClientIp,
    Json(req): Json<BlocklistDto>,
) -> AppResult<Json<OkDto>> {
    let mut terms: Vec<String> = req.terms.iter().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty() && t.chars().count() <= 60).collect();
    terms.sort();
    terms.dedup();
    if terms.len() > 5000 {
        return Err(AppError::invalid("too many terms"));
    }
    let mut tx = state.pool.begin().await?;
    db::catalog::replace_blocklist(&mut tx, &terms).await?;
    // The audit entry records the size only, not the words themselves.
    db::auth::audit(&mut *tx, &session.username, "blocklist_update", "blocklist", None, &serde_json::json!({ "terms": terms.len() }), Some(&ip)).await?;
    tx.commit().await?;
    Ok(Json(OkDto { ok: true }))
}

pub async fn audit_log(State(state): State<AppState>, _s: AdminSession) -> AppResult<Json<Vec<AuditEntryDto>>> {
    let rows = db::auth::audit_recent(&state.pool, 200).await?;
    Ok(Json(
        rows.into_iter()
            .map(|r| AuditEntryDto { at: db::ts(r.created_at), actor: r.actor, action: r.action, entity: r.entity, entity_id: r.entity_id, detail: r.detail })
            .collect(),
    ))
}
