//! Shared harness for integration tests: a real PostgreSQL (started with
//! testcontainers, or the one at `TEST_DATABASE_URL`), in-memory object
//! storage, mock providers, and the real router.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use providers::messenger::LogMessenger;
use providers::tryon::MockTryOn;
use server::config::Config;
use server::state::AppState;
use storage::MemStore;
use testcontainers::core::{IntoContainerPort, WaitFor};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tower::ServiceExt;

pub const ADMIN_USER: &str = "owner";
pub const ADMIN_PASS: &str = "correct-horse-battery";
pub const BOT_TOKEN: &str = "123456:TEST-TOKEN-not-a-real-bot";
pub const ADMIN_CHAT: i64 = 777;

pub struct TestApp {
    pub state: AppState,
    pub app: Router,
    pub messenger: Arc<LogMessenger>,
    pub store: Arc<MemStore>,
    // Dropping the container handle stops and removes the container.
    pub _pg: Option<ContainerAsync<GenericImage>>,
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&self.body)))
    }

    pub fn error_code(&self) -> String {
        self.json()["error"]["code"].as_str().unwrap_or("").to_string()
    }
}

/// An admin session: cookie plus CSRF token.
pub struct Admin {
    pub cookie: String,
    pub csrf: String,
}

async fn start_postgres() -> (Option<ContainerAsync<GenericImage>>, String) {
    if let Ok(admin_url) = std::env::var("TEST_DATABASE_URL") {
        // Reuse an existing server: give this test its own fresh database.
        let name = format!("qd_test_{}", uuid::Uuid::new_v4().simple());
        let admin = db::connect(&admin_url, 1).await.expect("connect to TEST_DATABASE_URL");
        db::sqlx::query(db::sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}"))).execute(&admin).await.expect("create test database");
        let base = admin_url.rsplit_once('/').map(|(b, _)| b.to_string()).unwrap_or(admin_url);
        return (None, format!("{base}/{name}"));
    }
    let container = GenericImage::new("postgres", "16-alpine")
        .with_exposed_port(5432.tcp())
        .with_wait_for(WaitFor::message_on_stderr("database system is ready to accept connections"))
        .with_env_var("POSTGRES_PASSWORD", "test")
        .start()
        .await
        .expect("start postgres container (is Docker running? or set TEST_DATABASE_URL)");
    let port = container.get_host_port_ipv4(5432).await.expect("mapped port");
    (Some(container), format!("postgres://postgres:test@127.0.0.1:{port}/postgres"))
}

pub async fn spawn_app() -> TestApp {
    spawn_app_with(&[]).await
}

pub async fn spawn_app_with(extra_env: &[(&str, &str)]) -> TestApp {
    let (pg, url) = start_postgres().await;
    // The container logs "ready" once during init and again for real;
    // retry until connections are actually accepted.
    let mut pool = None;
    for _ in 0..60 {
        match db::connect(&url, 5).await {
            Ok(p) => {
                pool = Some(p);
                break;
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(250)).await,
        }
    }
    let pool = pool.expect("database did not become ready");
    db::migrate(&pool).await.expect("migrations");

    let mut env: HashMap<String, String> = [
        ("DATABASE_URL", url.as_str()),
        ("STORAGE_BACKEND", "memory"),
        ("STATIC_DIR", "/nonexistent"),
        ("ADMIN_USERNAME", ADMIN_USER),
        ("ADMIN_PASSWORD", ADMIN_PASS),
        ("TELEGRAM_BOT_TOKEN", BOT_TOKEN),
        ("TELEGRAM_ADMIN_CHAT_ID", "777"),
        ("PUBLIC_BASE_URL", "http://localhost:8080"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    for (k, v) in extra_env {
        env.insert(k.to_string(), v.to_string());
    }
    let cfg = Config::from_lookup(|k| env.get(k).cloned()).expect("test config");

    let messenger = Arc::new(LogMessenger::new());
    let store = Arc::new(MemStore::new());
    let state = server::assemble_state(cfg, pool, store.clone(), Arc::new(MockTryOn { delay: Duration::ZERO }), messenger.clone());
    server::bootstrap_admin(&state).await.expect("admin bootstrap");
    let app = server::build_app(state.clone());
    TestApp { state, app, messenger, store, _pg: pg }
}

impl TestApp {
    pub async fn send(&self, req: Request<Body>) -> Reply {
        // `oneshot` drives the router with a single request, no socket needed.
        let res = self.app.clone().oneshot(req).await.expect("infallible");
        let (parts, body) = res.into_parts();
        let body = body.collect().await.expect("body").to_bytes().to_vec();
        Reply { status: parts.status, headers: parts.headers, body }
    }

    pub async fn get(&self, path: &str) -> Reply {
        self.send(Request::get(path).body(Body::empty()).unwrap()).await
    }

    pub async fn post_json(&self, path: &str, body: &serde_json::Value) -> Reply {
        self.json(Method::POST, path, body, &[]).await
    }

    pub async fn json(&self, method: Method, path: &str, body: &serde_json::Value, headers: &[(&str, &str)]) -> Reply {
        let mut b = Request::builder().method(method).uri(path).header(header::CONTENT_TYPE, "application/json");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        self.send(b.body(Body::from(body.to_string())).unwrap()).await
    }

    pub async fn post_bytes(&self, path: &str, bytes: Vec<u8>, headers: &[(&str, &str)]) -> Reply {
        let mut b = Request::post(path).header(header::CONTENT_TYPE, "application/octet-stream");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        self.send(b.body(Body::from(bytes)).unwrap()).await
    }

    pub async fn login(&self) -> Admin {
        let r = self.post_json("/api/admin/login", &serde_json::json!({ "username": ADMIN_USER, "password": ADMIN_PASS })).await;
        assert_eq!(r.status, StatusCode::OK, "login failed: {}", String::from_utf8_lossy(&r.body));
        let set_cookie = r.headers.get(header::SET_COOKIE).expect("session cookie").to_str().unwrap().to_string();
        let cookie = set_cookie.split(';').next().unwrap().to_string();
        Admin { cookie, csrf: r.json()["csrf_token"].as_str().unwrap().to_string() }
    }

    pub async fn admin_get(&self, admin: &Admin, path: &str) -> Reply {
        self.send(Request::get(path).header(header::COOKIE, &admin.cookie).body(Body::empty()).unwrap()).await
    }

    pub async fn admin_json(&self, admin: &Admin, method: Method, path: &str, body: &serde_json::Value) -> Reply {
        self.json(method, path, body, &[("cookie", &admin.cookie), ("x-csrf-token", &admin.csrf)]).await
    }

    /// Run queued background jobs to completion.
    pub async fn drain_jobs(&self) -> usize {
        server::worker::drain(&self.state).await
    }
}

/// A small valid PNG.
pub fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
    buf
}

/// A bright portrait-shaped "photo" that passes the try-on pre-checks.
pub fn photo(seed: u8) -> Vec<u8> {
    let img = image::RgbImage::from_fn(600, 800, |x, y| image::Rgb([150u8.wrapping_add(seed), (120 + (x / 10) % 60) as u8, (130 + (y / 10) % 60) as u8]));
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90).encode_image(&img).unwrap();
    buf
}

/// A hoodie design with Uzbek text and, optionally, an uploaded image.
pub fn design_doc(text: &str, asset: Option<(&str, u32, u32)>) -> serde_json::Value {
    let mut front = vec![serde_json::json!({
        "id": "t1", "x": 15.0, "y": 8.0, "type": "text", "text": text, "font": "montserrat",
        "size_cm": 3.0, "color": "#ffffff", "curve": 30.0, "align": "center"
    })];
    if let Some((id, w, h)) = asset {
        front.push(serde_json::json!({
            "id": "i1", "x": 15.0, "y": 22.0, "type": "image", "asset": id,
            "width_cm": 10.0, "height_cm": 10.0, "px_width": w, "px_height": h
        }));
    }
    serde_json::json!({ "version": 1, "garment": "hoodie", "color": "black", "front": front, "back": [] })
}

pub fn order_body(doc: &serde_json::Value, phone: &str) -> serde_json::Value {
    serde_json::json!({
        "doc": doc, "design_slug": null, "size": "L", "quantity": 2, "phone": phone, "customer_name": "Ulugʻbek",
        "delivery": { "method": "delivery", "region": "tashkent-city" },
        "address": "Mirobod tumani, Nukus koʻchasi 42", "notes": "Iltimos, tezroq", "lang": "uz", "accept_rights": true
    })
}
