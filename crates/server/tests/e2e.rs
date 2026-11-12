//! End-to-end tests through the real router against a real PostgreSQL.
//!
//! Run with Docker available (`cargo test -p server`), or point
//! `TEST_DATABASE_URL` at an existing server to skip container start-up.

mod common;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use common::*;
use serde_json::json;

/// The headline flow: design -> mockup render -> order -> admin sees and
/// downloads print files -> status changes notify the customer.
#[tokio::test]
async fn design_to_order_to_admin_print_files() {
    let app = spawn_app().await;

    // Catalog comes seeded.
    let catalog = app.get("/api/catalog").await.json();
    assert_eq!(catalog["brand"], "QiyiqDoʻppi");
    assert_eq!(catalog["garments"].as_array().unwrap().len(), 3);
    assert_eq!(catalog["regions"].as_array().unwrap().len(), 14);

    // Upload artwork.
    let up = app.post_bytes("/api/assets", png(400, 300, [220, 30, 60, 255]), &[]).await;
    assert_eq!(up.status, StatusCode::OK, "{}", String::from_utf8_lossy(&up.body));
    let asset = up.json();
    assert_eq!((asset["width"].as_u64(), asset["height"].as_u64()), (Some(400), Some(300)));
    let preview = app.get(asset["preview_url"].as_str().unwrap()).await;
    assert_eq!(preview.status, StatusCode::OK);
    assert_eq!(preview.headers[header::CONTENT_TYPE], "image/png");

    // Save a design with Uzbek text and the image; render its mockup.
    let doc = design_doc("Oʻzbekiston", Some((asset["id"].as_str().unwrap(), 400, 300)));
    let saved = app.post_json("/api/designs", &json!({ "doc": doc })).await.json();
    let slug = saved["slug"].as_str().unwrap().to_string();
    assert_eq!(app.get(&format!("/api/designs/{slug}")).await.json()["doc"]["front"][0]["text"], "Oʻzbekiston");

    let job = app.post_json(&format!("/api/designs/{slug}/render"), &json!({})).await.json();
    assert_eq!(app.get(&format!("/api/designs/{slug}/mockup/front")).await.status, StatusCode::NOT_FOUND);
    app.drain_jobs().await;
    assert_eq!(app.get(&format!("/api/jobs/{}", job["id"].as_str().unwrap())).await.json()["status"], "done");
    let mockup = app.get(&format!("/api/designs/{slug}/mockup/front")).await;
    assert_eq!(mockup.status, StatusCode::OK);
    let m = image::load_from_memory(&mockup.body).unwrap();
    assert_eq!((m.width(), m.height()), (900, 990));

    // The customer links Telegram before ordering, to receive updates.
    let mut tx = app.state.pool.begin().await.unwrap();
    db::orders::link_telegram_phone(&mut tx, 4242, 4242, "+998901234567", "uz").await.unwrap();
    tx.commit().await.unwrap();

    // Price is computed by the server: hoodie 220 000 + front print. The
    // design's bounding box is 300-600 cm², so the 50 000 tier applies.
    let quote =
        app.post_json("/api/price/quote", &json!({ "doc": doc, "quantity": 2, "delivery": { "method": "delivery", "region": "tashkent-city" } })).await.json();
    assert_eq!(quote["unit_garment"], 22_000_000);
    assert_eq!(quote["unit_print"], 5_000_000);
    assert_eq!(quote["total"], 2 * 27_000_000 + 2_500_000);

    // Order.
    let mut body = order_body(&doc, "90 123 45 67");
    body["design_slug"] = json!(slug);
    let created = app.post_json("/api/orders", &body).await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let created = created.json();
    let code = created["code"].as_str().unwrap().to_string();
    assert!(code.starts_with("QD-") && code.len() == 9);
    assert_eq!(created["quote"]["total"], quote["total"]);

    // Background jobs: moderation, files, notifications.
    app.drain_jobs().await;
    let sent = app.messenger.sent();
    let admin_alert = sent.iter().find(|(chat, _)| *chat == ADMIN_CHAT).expect("admin alert");
    assert!(admin_alert.1.contains(&code) && admin_alert.1.contains("+998 90 123 45 67"));
    assert!(sent.iter().any(|(chat, text)| *chat == 4242 && text.contains(&code) && text.contains("qabul qilindi")));

    // Customer lookup needs code AND phone.
    let found = app.post_json("/api/orders/lookup", &json!({ "code": code.to_lowercase(), "phone": "+998901234567" })).await;
    assert_eq!(found.status, StatusCode::OK);
    assert_eq!(found.json()["status"], "new");
    let wrong = app.post_json("/api/orders/lookup", &json!({ "code": code, "phone": "+998907654321" })).await;
    assert_eq!(wrong.status, StatusCode::NOT_FOUND);

    // Admin: list, detail, files.
    assert_eq!(app.get("/api/admin/orders").await.status, StatusCode::UNAUTHORIZED);
    let admin = app.login().await;
    let list = app.admin_get(&admin, "/api/admin/orders?status=new&q=90123").await.json();
    assert_eq!(list["total"], 1);
    let id = list["orders"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(list["orders"][0]["files_ready"], true);
    assert_eq!(list["orders"][0]["moderation"], "clean");

    let detail = app.admin_get(&admin, &format!("/api/admin/orders/{id}")).await.json();
    let kinds: Vec<&str> = detail["files"].as_array().unwrap().iter().map(|f| f["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, vec!["mockup_back", "mockup_front", "print_front", "sheet"], "empty back side has no print file");
    assert_eq!(detail["allowed_next"], json!(["confirmed", "cancelled"]));

    // Print file: 30 x 35 cm at 300 DPI, transparent, tagged with its DPI.
    let print = app.admin_get(&admin, &format!("/api/admin/orders/{id}/files/print_front")).await;
    assert_eq!(print.status, StatusCode::OK);
    assert!(print.headers[header::CONTENT_DISPOSITION].to_str().unwrap().contains(&format!("{code}-print-front.png")));
    let reader = png::Decoder::new(std::io::Cursor::new(&print.body)).read_info().unwrap();
    let info = reader.info();
    assert_eq!((info.width, info.height), (3543, 4134));
    assert_eq!(info.pixel_dims.unwrap().xppu, 11811);
    let img = image::load_from_memory(&print.body).unwrap().to_rgba8();
    assert_eq!(img.get_pixel(3, 3).0[3], 0, "background must be transparent");
    // The uploaded red image is centred at (15, 22) cm.
    let px = img.get_pixel((15.0 / 30.0 * 3543.0) as u32, (22.0 / 35.0 * 4134.0) as u32).0;
    assert!(px[0] > 200 && px[1] < 60 && px[3] == 255, "{px:?}");

    let sheet = app.admin_get(&admin, &format!("/api/admin/orders/{id}/files/sheet")).await;
    assert!(sheet.body.starts_with(b"%PDF-1.4"));
    assert_eq!(app.get(&format!("/api/admin/orders/{id}/files/sheet")).await.status, StatusCode::UNAUTHORIZED);

    // Status changes need the CSRF token, follow the state machine, are
    // logged with the actor, and notify the customer.
    let no_csrf = app.json(Method::POST, &format!("/api/admin/orders/{id}/status"), &json!({ "to": "confirmed" }), &[("cookie", &admin.cookie)]).await;
    assert_eq!(no_csrf.status, StatusCode::FORBIDDEN);
    let skip = app.admin_json(&admin, Method::POST, &format!("/api/admin/orders/{id}/status"), &json!({ "to": "ready" })).await;
    assert_eq!((skip.status, skip.error_code().as_str()), (StatusCode::CONFLICT, "transition_not_allowed"));
    for to in ["confirmed", "in_production", "ready", "delivered"] {
        let r = app.admin_json(&admin, Method::POST, &format!("/api/admin/orders/{id}/status"), &json!({ "to": to, "note": "ok" })).await;
        assert_eq!(r.status, StatusCode::OK, "{to}: {}", String::from_utf8_lossy(&r.body));
    }
    app.drain_jobs().await;
    assert!(app.messenger.sent().iter().any(|(chat, text)| *chat == 4242 && text.contains("Yetkazildi")));

    let detail = app.admin_get(&admin, &format!("/api/admin/orders/{id}")).await.json();
    let log = detail["log"].as_array().unwrap();
    assert_eq!(log.len(), 5);
    assert_eq!(log[0]["actor"], "customer");
    assert_eq!((log[4]["to"].as_str(), log[4]["actor"].as_str()), (Some("delivered"), Some(ADMIN_USER)));
    let timeline = app.post_json("/api/orders/lookup", &json!({ "code": code, "phone": "901234567" })).await.json();
    assert_eq!(timeline["timeline"].as_array().unwrap().len(), 5);

    let audit = app.admin_get(&admin, "/api/admin/audit").await.json();
    assert!(audit.as_array().unwrap().iter().any(|e| e["action"] == "order_status" && e["entity_id"] == code));
}

#[tokio::test]
async fn untrusted_input_is_rejected() {
    let app = spawn_app().await;

    // Not an image, whatever the extension or Content-Type claims.
    for bad in [b"<svg xmlns='http://www.w3.org/2000/svg'></svg>".to_vec(), b"GIF89a....".to_vec(), vec![0u8; 64]] {
        let r = app.post_bytes("/api/assets", bad, &[]).await;
        assert_eq!((r.status, r.error_code().as_str()), (StatusCode::BAD_REQUEST, "upload_type"));
    }
    let mut truncated = png(50, 50, [1, 2, 3, 255]);
    truncated.truncate(40);
    assert_eq!(app.post_bytes("/api/assets", truncated, &[]).await.error_code(), "upload_invalid");
    // Over the size limit: refused before decoding.
    let huge = app.post_bytes("/api/assets", vec![0u8; 11 * 1024 * 1024], &[]).await;
    assert_eq!(huge.status, StatusCode::PAYLOAD_TOO_LARGE);

    let doc = design_doc("Salom", None);
    let ok = order_body(&doc, "901234567");

    // Foil is modelled but not orderable in v1.
    let mut foil = doc.clone();
    foil["front"][0]["finish"] = json!("foil_gold");
    assert_eq!(app.post_json("/api/designs", &json!({ "doc": foil })).await.error_code(), "invalid_design");

    // (what is wrong, how to break a valid order, expected error code)
    type Mutate = Box<dyn Fn(&mut serde_json::Value)>;
    let cases: Vec<(&str, Mutate, &str)> = vec![
        ("bad phone", Box::new(|b| b["phone"] = json!("12345")), "invalid_phone"),
        ("foreign phone", Box::new(|b| b["phone"] = json!("+7 901 123 45 67")), "invalid_phone"),
        ("zero quantity", Box::new(|b| b["quantity"] = json!(0)), "invalid_quantity"),
        ("huge quantity", Box::new(|b| b["quantity"] = json!(100000)), "invalid_quantity"),
        ("unknown size", Box::new(|b| b["size"] = json!("XXXXL")), "size_required"),
        ("no rights", Box::new(|b| b["accept_rights"] = json!(false)), "rights_required"),
        ("no address", Box::new(|b| b["address"] = json!("")), "address_required"),
        ("unknown region", Box::new(|b| b["delivery"]["region"] = json!("mars")), "region_required"),
        ("empty design", Box::new(|b| b["doc"]["front"] = json!([])), "empty_design"),
        ("unknown garment", Box::new(|b| b["doc"]["garment"] = json!("tuxedo")), "invalid_design"),
        ("unknown colour", Box::new(|b| b["doc"]["color"] = json!("rainbow")), "invalid_design"),
    ];
    for (i, (name, mutate, code)) in cases.into_iter().enumerate() {
        let mut body = ok.clone();
        // A different phone per case, so the per-phone rate limit (checked
        // before validation) does not interfere.
        body["phone"] = json!(format!("9012345{i:02}"));
        mutate(&mut body);
        let r = app.post_json("/api/orders", &body).await;
        assert_eq!((r.status, r.error_code().as_str()), (StatusCode::BAD_REQUEST, code), "{name}");
    }
    // Nothing was created by any of the rejected requests.
    let n: i64 = db::sqlx::query_scalar("SELECT count(*) FROM orders").fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(n, 0);

    // A client-supplied price is simply ignored.
    let mut cheeky = ok.clone();
    cheeky["total"] = json!(1);
    cheeky["quote"] = json!({ "total": 1 });
    cheeky["delivery"] = json!({ "method": "pickup", "region": null });
    cheeky["address"] = json!(null);
    let r = app.post_json("/api/orders", &cheeky).await;
    assert_eq!(r.status, StatusCode::CREATED);
    // Hoodie 220 000 + smallest print tier 20 000 (text box under 100 cm²),
    // x2, pickup (no delivery fee).
    assert_eq!(r.json()["quote"]["total"], 2 * (22_000_000 + 2_000_000));

    // Editing a design needs its secret token.
    let saved = app.post_json("/api/designs", &json!({ "doc": doc })).await.json();
    let path = format!("/api/designs/{}", saved["slug"].as_str().unwrap());
    let other = design_doc("Oʻzgartirildi", None);
    assert_eq!(app.json(Method::PUT, &path, &json!({ "doc": other }), &[]).await.status, StatusCode::FORBIDDEN);
    assert_eq!(app.json(Method::PUT, &path, &json!({ "doc": other }), &[("x-edit-token", "wrong")]).await.status, StatusCode::FORBIDDEN);
    let good = app.json(Method::PUT, &path, &json!({ "doc": other }), &[("x-edit-token", saved["edit_token"].as_str().unwrap())]).await;
    assert_eq!(good.status, StatusCode::OK);
    assert_eq!(app.get(&path).await.json()["doc"]["front"][0]["text"], "Oʻzgartirildi");
}

#[tokio::test]
async fn security_headers_auth_and_cors() {
    let app = spawn_app_with(&[("CORS_ALLOWED_ORIGINS", "https://localhost, capacitor://localhost")]).await;

    let r = app.get("/api/catalog").await;
    let h = &r.headers;
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["x-frame-options"], "DENY");
    assert_eq!(h["cache-control"], "no-store");
    assert!(h.contains_key("x-request-id"));
    let csp = h["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("default-src 'self'") && csp.contains("frame-ancestors 'none'") && csp.contains("object-src 'none'"));
    assert!(!csp.contains("'unsafe-eval'") && !csp.contains("script-src 'self' 'unsafe-inline'"));
    // The customer app may be framed by Telegram (Mini App), nothing else.
    let web = app.get("/").await;
    assert!(web.headers["content-security-policy"].to_str().unwrap().contains("frame-ancestors 'self' https://web.telegram.org"));

    assert_eq!(app.get("/healthz").await.body, b"ok");
    assert_eq!(app.get("/readyz").await.status, StatusCode::OK);
    let metrics = String::from_utf8(app.get("/metrics").await.body).unwrap();
    assert!(metrics.contains("qiyiq_http_responses_total{class=\"2xx\"}"));
    assert_eq!(app.get("/api/nope").await.status, StatusCode::NOT_FOUND);

    // CORS: listed app origins only, and never with credentials.
    let preflight = |origin: &'static str| {
        Request::builder()
            .method(Method::OPTIONS)
            .uri("/api/orders")
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
            .body(Body::empty())
            .unwrap()
    };
    let allowed = app.send(preflight("capacitor://localhost")).await;
    assert_eq!(allowed.headers[header::ACCESS_CONTROL_ALLOW_ORIGIN], "capacitor://localhost");
    assert!(!allowed.headers.contains_key(header::ACCESS_CONTROL_ALLOW_CREDENTIALS));
    let denied = app.send(preflight("https://evil.example")).await;
    assert!(!denied.headers.contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));

    // Every admin route refuses anonymous access.
    for path in
        ["/api/admin/me", "/api/admin/orders", "/api/admin/moderation", "/api/admin/reports", "/api/admin/pricing", "/api/admin/blocklist", "/api/admin/audit"]
    {
        assert_eq!(app.get(path).await.status, StatusCode::UNAUTHORIZED, "{path}");
    }
    let forged = app.send(Request::get("/api/admin/me").header(header::COOKIE, "qd_session=deadbeef").body(Body::empty()).unwrap()).await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);

    // Login: wrong password is refused and audited; the cookie is HttpOnly
    // and SameSite=Strict; logout invalidates the session server-side.
    let bad = app.post_json("/api/admin/login", &json!({ "username": ADMIN_USER, "password": "nope-nope-nope" })).await;
    assert_eq!((bad.status, bad.error_code().as_str()), (StatusCode::UNAUTHORIZED, "wrong_login"));
    let ghost = app.post_json("/api/admin/login", &json!({ "username": "ghost", "password": ADMIN_PASS })).await;
    assert_eq!(ghost.status, StatusCode::UNAUTHORIZED);
    let ok = app.post_json("/api/admin/login", &json!({ "username": ADMIN_USER, "password": ADMIN_PASS })).await;
    let set_cookie = ok.headers[header::SET_COOKIE].to_str().unwrap();
    assert!(set_cookie.contains("HttpOnly") && set_cookie.contains("SameSite=Strict") && set_cookie.contains("Path=/api/admin"));
    // Passwords and session tokens are stored hashed.
    let hash: String = db::sqlx::query_scalar("SELECT password_hash FROM admins").fetch_one(&app.state.pool).await.unwrap();
    assert!(hash.starts_with("$argon2id$") && !hash.contains(ADMIN_PASS));
    let token = set_cookie.split(';').next().unwrap().trim_start_matches("qd_session=");
    let stored: String = db::sqlx::query_scalar("SELECT token_hash FROM sessions LIMIT 1").fetch_one(&app.state.pool).await.unwrap();
    assert_ne!(stored, token);

    let admin = app.login().await;
    assert_eq!(app.admin_get(&admin, "/api/admin/me").await.json()["username"], ADMIN_USER);
    assert_eq!(app.admin_json(&admin, Method::POST, "/api/admin/logout", &json!({})).await.status, StatusCode::OK);
    assert_eq!(app.admin_get(&admin, "/api/admin/me").await.status, StatusCode::UNAUTHORIZED);
    let audit: i64 = db::sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE action = 'login_failed'").fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(audit, 2);

    // Repeated wrong passwords get rate limited.
    let mut limited = false;
    for _ in 0..12 {
        let r = app.post_json("/api/admin/login", &json!({ "username": ADMIN_USER, "password": "wrong-wrong-wrong" })).await;
        limited |= r.status == StatusCode::TOO_MANY_REQUESTS;
    }
    assert!(limited, "login must be rate limited");
}

#[tokio::test]
async fn moderation_blocks_production_until_reviewed() {
    let app = spawn_app().await;
    let admin = app.login().await;
    let put = app.admin_json(&admin, Method::PUT, "/api/admin/blocklist", &json!({ "terms": ["Yomonsoʻz", "  ", "плохое"] })).await;
    assert_eq!(put.status, StatusCode::OK);
    assert_eq!(app.admin_get(&admin, "/api/admin/blocklist").await.json()["terms"], json!(["yomonsoʻz", "плохое"]));

    // Obfuscated spelling is still caught, and flagged at order time.
    let doc = design_doc("Y-0-M-O-N s.o.ʻ.z", None);
    let created = app.post_json("/api/orders", &order_body(&doc, "901234567")).await;
    assert_eq!(created.status, StatusCode::CREATED, "a flagged design still creates the order; a human decides");
    app.drain_jobs().await;

    let queue = app.admin_get(&admin, "/api/admin/moderation").await.json();
    assert_eq!(queue.as_array().unwrap().len(), 1);
    assert!(queue[0]["reason"].as_str().unwrap().contains("blocked word"));
    let design_id = queue[0]["design_id"].as_str().unwrap().to_string();
    let preview = app.admin_get(&admin, queue[0]["preview_url"].as_str().unwrap()).await;
    assert_eq!(preview.status, StatusCode::OK);
    assert!(image::load_from_memory(&preview.body).is_ok());

    let id = app.admin_get(&admin, "/api/admin/orders").await.json()["orders"][0]["id"].as_str().unwrap().to_string();
    let status_path = format!("/api/admin/orders/{id}/status");
    let (app_ref, admin_ref, path_ref) = (&app, &admin, &status_path);
    // An `async move` block owns the JSON body, so nothing borrowed outlives it.
    let status = |to: &'static str| async move { app_ref.admin_json(admin_ref, Method::POST, path_ref, &json!({ "to": to })).await };
    assert_eq!(status("confirmed").await.status, StatusCode::OK);
    let blocked = status("in_production").await;
    assert_eq!((blocked.status, blocked.error_code().as_str()), (StatusCode::CONFLICT, "needs_moderation"));

    let decide =
        app.admin_json(&admin, Method::POST, &format!("/api/admin/designs/{design_id}/moderation"), &json!({ "approve": true, "note": "hazil" })).await;
    assert_eq!(decide.status, StatusCode::OK);
    assert_eq!(status("in_production").await.status, StatusCode::OK);
    assert!(app.admin_get(&admin, "/api/admin/moderation").await.json().as_array().unwrap().is_empty());

    // Image moderation: a provider verdict also sends the design to review.
    let flagged_app = spawn_app_with(&[("MODERATION_MOCK_MODE", "flag_all")]).await;
    let asset = flagged_app.post_bytes("/api/assets", png(64, 64, [9, 9, 9, 255]), &[]).await.json();
    let doc = design_doc("Rasm", Some((asset["id"].as_str().unwrap(), 64, 64)));
    assert_eq!(flagged_app.post_json("/api/orders", &order_body(&doc, "901234567")).await.status, StatusCode::CREATED);
    flagged_app.drain_jobs().await;
    let status: String = db::sqlx::query_scalar("SELECT moderation_status FROM designs LIMIT 1").fetch_one(&flagged_app.state.pool).await.unwrap();
    assert_eq!(status, "flagged");
}

#[tokio::test]
async fn reports_and_takedown() {
    let app = spawn_app().await;
    let saved = app.post_json("/api/designs", &json!({ "doc": design_doc("Brend nusxasi", None) })).await.json();
    let slug = saved["slug"].as_str().unwrap();

    // A rights claim must say how to reach the claimant.
    let r = app.post_json(&format!("/api/designs/{slug}/report"), &json!({ "kind": "trademark", "reason": "Bu bizning logotipimiz", "contact": null })).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = app
        .post_json(&format!("/api/designs/{slug}/report"), &json!({ "kind": "trademark", "reason": "Bu bizning logotipimiz", "contact": "legal@example.uz" }))
        .await;
    assert_eq!(r.status, StatusCode::OK);
    app.drain_jobs().await;
    assert!(app.messenger.sent().iter().any(|(chat, text)| *chat == ADMIN_CHAT && text.contains(slug)));

    let admin = app.login().await;
    let reports = app.admin_get(&admin, "/api/admin/reports").await.json();
    assert_eq!((reports[0]["kind"].as_str(), reports[0]["status"].as_str()), (Some("trademark"), Some("open")));
    assert_eq!(app.admin_get(&admin, "/api/admin/moderation").await.json().as_array().unwrap().len(), 1);

    // Takedown: the design disappears for the public and cannot be ordered
    // through its link any more.
    let id = reports[0]["id"].as_str().unwrap();
    let done = app.admin_json(&admin, Method::POST, &format!("/api/admin/reports/{id}"), &json!({ "remove": true, "note": "tasdiqlandi" })).await;
    assert_eq!(done.status, StatusCode::OK);
    assert_eq!(app.get(&format!("/api/designs/{slug}")).await.status, StatusCode::NOT_FOUND);
    assert_eq!(app.get(&format!("/api/designs/{slug}/mockup/front")).await.status, StatusCode::NOT_FOUND);
    assert_eq!(app.admin_get(&admin, "/api/admin/reports").await.json()[0]["status"], "removed");
    // Resolving twice is refused.
    assert_eq!(app.admin_json(&admin, Method::POST, &format!("/api/admin/reports/{id}"), &json!({ "remove": false })).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_price_editor_changes_quotes() {
    let app = spawn_app().await;
    let admin = app.login().await;
    let doc = design_doc("Narx", None);
    let quote_req = json!({ "doc": doc, "quantity": 10, "delivery": { "method": "delivery", "region": "samarkand" } });
    let before = app.post_json("/api/price/quote", &quote_req).await.json();
    // 10 pieces: 10 % off (220 000 + 20 000) * 10, plus 40 000 delivery.
    assert_eq!(before["discount_bp"], 1000);
    assert_eq!(before["total"], 240_000_000 - 24_000_000 + 4_000_000);

    let mut cfg = app.admin_get(&admin, "/api/admin/pricing").await.json();
    assert_eq!(cfg["areas"].as_array().unwrap().len(), 6);
    for g in cfg["garments"].as_array_mut().unwrap() {
        if g["slug"] == "hoodie" {
            g["base_price"] = json!(30_000_000);
        }
    }
    cfg["tiers"] = json!([{ "max_area_cm2": 2000, "price": 5_000_000 }]);
    cfg["discounts"] = json!([{ "min_qty": 10, "percent_bp": 2000 }]);
    for r in cfg["regions"].as_array_mut().unwrap() {
        if r["slug"] == "samarkand" {
            r["delivery_fee"] = json!(0);
        }
    }
    assert_eq!(app.admin_json(&admin, Method::PUT, "/api/admin/pricing", &cfg).await.status, StatusCode::OK);
    let after = app.post_json("/api/price/quote", &quote_req).await.json();
    assert_eq!(after["total"], (30_000_000i64 + 5_000_000) * 10 * 80 / 100);

    // Invalid tables are refused as a whole and change nothing.
    let mut bad = cfg.clone();
    bad["discounts"] = json!([{ "min_qty": 5, "percent_bp": 20000 }]);
    assert_eq!(app.admin_json(&admin, Method::PUT, "/api/admin/pricing", &bad).await.status, StatusCode::BAD_REQUEST);
    let mut bad = cfg.clone();
    bad["garments"][0]["base_price"] = json!(-5);
    assert_eq!(app.admin_json(&admin, Method::PUT, "/api/admin/pricing", &bad).await.status, StatusCode::BAD_REQUEST);
    let mut bad = cfg.clone();
    bad["areas"][0]["mock_w"] = json!(1.5);
    assert_eq!(app.admin_json(&admin, Method::PUT, "/api/admin/pricing", &bad).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(app.post_json("/api/price/quote", &quote_req).await.json(), after);

    // An existing order keeps the price it was created with.
    let order = app.post_json("/api/orders", &order_body(&doc, "901234567")).await.json();
    let frozen = order["quote"]["total"].clone();
    cfg["garments"][0]["base_price"] = json!(99_000_000);
    cfg["garments"][1]["base_price"] = json!(99_000_000);
    cfg["garments"][2]["base_price"] = json!(99_000_000);
    assert_eq!(app.admin_json(&admin, Method::PUT, "/api/admin/pricing", &cfg).await.status, StatusCode::OK);
    let looked = app.post_json("/api/orders/lookup", &json!({ "code": order["code"], "phone": "901234567" })).await.json();
    assert_eq!(looked["quote"]["total"], frozen);
}

#[tokio::test]
async fn privacy_export_and_delete() {
    let app = spawn_app().await;
    let created = app.post_json("/api/orders", &order_body(&design_doc("Maxfiy", None), "901234567")).await.json();
    let code = created["code"].as_str().unwrap();
    let proof = json!({ "phone": "+998 90 123 45 67", "code": code });

    // The proof is phone + one of that phone's order codes.
    assert_eq!(app.post_json("/api/me/export", &json!({ "phone": "907654321", "code": code })).await.status, StatusCode::NOT_FOUND);
    let export = app.post_json("/api/me/export", &proof).await.json();
    assert_eq!(export["phone"], "+998901234567");
    assert_eq!(export["orders"][0]["address"], "Mirobod tumani, Nukus koʻchasi 42");

    // Active orders must be finished or cancelled first.
    let blocked = app.post_json("/api/me/delete", &proof).await;
    assert_eq!((blocked.status, blocked.error_code().as_str()), (StatusCode::CONFLICT, "active_orders"));
    let admin = app.login().await;
    let id = app.admin_get(&admin, "/api/admin/orders").await.json()["orders"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(app.admin_json(&admin, Method::POST, &format!("/api/admin/orders/{id}/status"), &json!({ "to": "cancelled" })).await.status, StatusCode::OK);

    assert_eq!(app.post_json("/api/me/delete", &proof).await.json()["deleted"], true);
    let (phone, name, address, notes): (Option<String>, Option<String>, Option<String>, Option<String>) =
        db::sqlx::query_as("SELECT phone, customer_name, address, notes FROM orders").fetch_one(&app.state.pool).await.unwrap();
    assert_eq!((phone, name, address, notes), (None, None, None, None));
    let customers: i64 = db::sqlx::query_scalar("SELECT count(*) FROM customers").fetch_one(&app.state.pool).await.unwrap();
    assert_eq!(customers, 0);
    // The sales record itself survives, anonymised.
    let total: i64 = db::sqlx::query_scalar("SELECT total_tiyin FROM orders").fetch_one(&app.state.pool).await.unwrap();
    assert!(total > 0);
    assert_eq!(app.post_json("/api/orders/lookup", &json!({ "code": code, "phone": "901234567" })).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn telegram_mini_app_auth() {
    let app = spawn_app().await;
    let now = chrono::Utc::now().timestamp().to_string();
    let user = r#"{"id":4242,"first_name":"Aziza","language_code":"ru"}"#;
    let init = server::telegram::sign_init_data(&[("auth_date", &now), ("user", user)], BOT_TOKEN);

    let me = app.post_json("/api/telegram/me", &json!({ "init_data": init })).await;
    assert_eq!(me.status, StatusCode::OK, "{}", String::from_utf8_lossy(&me.body));
    let me = me.json();
    assert_eq!((me["lang"].as_str(), me["first_name"].as_str(), me["phone"].is_null()), (Some("ru"), Some("Aziza"), true));

    // Signed with another bot's token, or tampered: refused.
    let forged = server::telegram::sign_init_data(&[("auth_date", &now), ("user", user)], "999:someone-else");
    assert_eq!(app.post_json("/api/telegram/me", &json!({ "init_data": forged })).await.status, StatusCode::UNAUTHORIZED);
    let tampered = init.replace("4242", "1");
    assert_eq!(app.post_json("/api/telegram/me", &json!({ "init_data": tampered })).await.status, StatusCode::UNAUTHORIZED);

    // After sharing the phone with the bot, "my orders" works in the Mini App.
    app.post_json("/api/orders", &order_body(&design_doc("TG", None), "901234567")).await;
    let mut tx = app.state.pool.begin().await.unwrap();
    db::orders::link_telegram_phone(&mut tx, 4242, 4242, "+998901234567", "ru").await.unwrap();
    tx.commit().await.unwrap();
    let me = app.post_json("/api/telegram/me", &json!({ "init_data": init })).await.json();
    assert_eq!(me["phone"], "+998901234567");
    assert_eq!(me["orders"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn lookup_is_rate_limited() {
    let app = spawn_app().await;
    let mut statuses = Vec::new();
    for i in 0..25 {
        let r = app.post_json("/api/orders/lookup", &json!({ "code": format!("QD-{i:06}"), "phone": "901234567" })).await;
        statuses.push(r.status);
    }
    assert!(statuses[..20].iter().all(|s| *s == StatusCode::NOT_FOUND));
    assert!(statuses[20..].iter().all(|s| *s == StatusCode::TOO_MANY_REQUESTS), "guessing order codes must be throttled");
}
