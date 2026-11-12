//! Photo-based virtual try-on (Tier B).
//!
//! `MockTryOn` composites the garment mockup onto the photo so the whole
//! flow works offline. `FashnTryOn` calls the FASHN API (model
//! `tryon-v1.6`); see `docs/adr/0008-tryon-provider.md` for why it was
//! chosen and `docs/PRIVACY.md` for its data handling.

use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use bytes::Bytes;
use domain::Tiyin;
use image::{DynamicImage, RgbaImage};
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum TryOnError {
    /// The provider refused the input (bad pose, unreadable image, content
    /// policy). Not retried; the customer should try another photo.
    #[error("try-on input rejected: {0}")]
    Rejected(String),
    /// Temporary failure; the job may be retried.
    #[error("try-on provider unavailable: {0}")]
    Unavailable(String),
    #[error("try-on provider timed out")]
    Timeout,
}

pub struct TryOnRequest {
    /// The customer's photo, already validated and re-encoded as JPEG.
    pub person_jpeg: Bytes,
    /// Garment mockup with the design, transparent PNG.
    pub garment_png: Bytes,
}

pub struct TryOnOutput {
    pub image: Bytes,
    /// `image/jpeg` or `image/png`.
    pub mime: &'static str,
}

#[async_trait]
pub trait TryOnProvider: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    /// Cost of one generated image, used for the daily spending cap.
    fn cost_per_image(&self) -> Tiyin;
    async fn generate(&self, req: TryOnRequest) -> Result<TryOnOutput, TryOnError>;
}

// ------------------------------------------------------------ prechecks

/// Why a photo was refused before any provider is called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhotoIssue {
    TooSmall,
    TooDark,
}

impl PhotoIssue {
    /// Error code understood by the web app (`error.<code>` translations).
    pub fn code(&self) -> &'static str {
        match self {
            PhotoIssue::TooSmall => "tryon_photo_small",
            PhotoIssue::TooDark => "tryon_photo_dark",
        }
    }
}

pub const MIN_PHOTO_SHORT_SIDE: u32 = 480;
pub const MIN_PHOTO_MEAN_LUMA: u32 = 45;

/// Server-side sanity checks on the uploaded photo. Person/torso detection
/// runs on the device (it needs the pose model, and keeps rejected photos
/// off the server); these two cheap checks are repeated here because the
/// server must not rely on the client.
pub fn precheck_photo(img: &DynamicImage) -> Result<(), PhotoIssue> {
    if img.width().min(img.height()) < MIN_PHOTO_SHORT_SIDE {
        return Err(PhotoIssue::TooSmall);
    }
    // Mean brightness over a small thumbnail is plenty accurate.
    let thumb = img.thumbnail(64, 64).to_luma8();
    let sum: u64 = thumb.pixels().map(|p| p.0[0] as u64).sum();
    let mean = (sum / (thumb.width() as u64 * thumb.height() as u64).max(1)) as u32;
    if mean < MIN_PHOTO_MEAN_LUMA {
        return Err(PhotoIssue::TooDark);
    }
    Ok(())
}

fn encode_jpeg(img: &RgbaImage, quality: u8) -> Result<Bytes, String> {
    let rgb = DynamicImage::ImageRgba8(img.clone()).to_rgb8();
    let mut buf = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality).encode_image(&rgb).map_err(|e| e.to_string())?;
    Ok(Bytes::from(buf))
}

// ----------------------------------------------------------------- mock

/// Offline stand-in: pastes the garment mockup over the upper body area of
/// the photo and marks the result with a striped band so nobody mistakes it
/// for a real generation.
pub struct MockTryOn {
    /// Artificial delay, to exercise the "generating" UI state.
    pub delay: Duration,
}

impl Default for MockTryOn {
    fn default() -> Self {
        MockTryOn { delay: Duration::from_millis(300) }
    }
}

/// Synchronous compositing core (also used by the evaluation harness).
pub fn mock_composite(person: &[u8], garment_png: &[u8]) -> Result<Bytes, TryOnError> {
    let person = image::load_from_memory(person).map_err(|e| TryOnError::Rejected(format!("photo: {e}")))?;
    let garment = image::load_from_memory(garment_png).map_err(|e| TryOnError::Rejected(format!("garment: {e}")))?.to_rgba8();
    // Bound the working size so a huge photo cannot exhaust memory.
    let person = if person.width().max(person.height()) > 1280 { person.thumbnail(1280, 1280) } else { person };
    let mut canvas = person.to_rgba8();
    let (w, h) = canvas.dimensions();

    // Roughly where a torso is in a portrait photo: centred, upper-middle.
    let gw = (w as f32 * 0.66).round().max(1.0) as u32;
    let gh = (gw as f32 * garment.height() as f32 / garment.width().max(1) as f32).round().max(1.0) as u32;
    let scaled = image::imageops::resize(&garment, gw, gh, image::imageops::FilterType::Triangle);
    let x = (w as i64 - gw as i64) / 2;
    let y = (h as f32 * 0.24) as i64;
    image::imageops::overlay(&mut canvas, &scaled, x, y);

    // Yellow/black hazard stripes along the bottom edge.
    let band = (h / 28).max(6);
    for py in h - band..h {
        for px in 0..w {
            let stripe = ((px + py) / band) % 2 == 0;
            canvas.put_pixel(px, py, if stripe { image::Rgba([255, 223, 0, 255]) } else { image::Rgba([18, 19, 22, 255]) });
        }
    }
    encode_jpeg(&canvas, 86).map_err(TryOnError::Unavailable)
}

#[async_trait]
impl TryOnProvider for MockTryOn {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn cost_per_image(&self) -> Tiyin {
        Tiyin::ZERO
    }

    async fn generate(&self, req: TryOnRequest) -> Result<TryOnOutput, TryOnError> {
        tokio::time::sleep(self.delay).await;
        let image = tokio::task::spawn_blocking(move || mock_composite(&req.person_jpeg, &req.garment_png))
            .await
            .map_err(|e| TryOnError::Unavailable(e.to_string()))??;
        Ok(TryOnOutput { image, mime: "image/jpeg" })
    }
}

// ---------------------------------------------------------------- FASHN

#[derive(Clone, Debug)]
pub struct FashnConfig {
    pub api_key: String,
    /// `https://api.fashn.ai` in production; overridable for tests.
    pub base_url: String,
    /// `performance`, `balanced` or `quality`.
    pub mode: String,
    pub timeout: Duration,
    pub poll_interval: Duration,
    /// What one image costs us, converted to tiyin by the operator.
    pub cost_per_image: Tiyin,
}

/// FASHN API client, model `tryon-v1.6` (API verified against
/// docs.fashn.ai on 2026-11-11).
///
/// Images are sent inline as base64 data URIs and the result is requested
/// as base64 too, so neither the photo nor the result is left on a public
/// URL: FASHN deletes inline inputs when the request finishes.
pub struct FashnTryOn {
    cfg: FashnConfig,
    http: reqwest::Client,
}

#[derive(Deserialize)]
struct RunResponse {
    id: Option<String>,
    error: Option<serde_json::Value>,
    message: Option<String>,
}

#[derive(Deserialize)]
struct StatusResponse {
    status: String,
    #[serde(default)]
    output: Option<Vec<String>>,
    #[serde(default)]
    error: Option<StatusError>,
}

#[derive(Deserialize)]
struct StatusError {
    #[serde(default)]
    name: String,
    #[serde(default)]
    message: String,
}

/// Hosts FASHN serves result files from. Result URLs on any other host are
/// refused, so a compromised or spoofed response cannot make the server
/// fetch arbitrary internal addresses (SSRF).
const FASHN_CDN_HOSTS: [&str; 2] = ["cdn.fashn.ai", "media.fashn.ai"];
const MAX_RESULT_BYTES: usize = 25 * 1024 * 1024;

impl FashnTryOn {
    pub fn new(cfg: FashnConfig) -> Result<Self, TryOnError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .user_agent("qiyiqdoppi-server")
            .build()
            .map_err(|e| TryOnError::Unavailable(e.to_string()))?;
        Ok(FashnTryOn { cfg, http })
    }

    /// The mockup has a transparent background; try-on models expect a
    /// flat-lay product photo, so place it on light grey first.
    fn flatten_garment(png: &[u8]) -> Result<Bytes, TryOnError> {
        let g = image::load_from_memory(png).map_err(|e| TryOnError::Rejected(format!("garment: {e}")))?.to_rgba8();
        let mut bg = RgbaImage::from_pixel(g.width(), g.height(), image::Rgba([242, 242, 242, 255]));
        image::imageops::overlay(&mut bg, &g, 0, 0);
        encode_jpeg(&bg, 92).map_err(TryOnError::Unavailable)
    }

    async fn result_bytes(&self, item: &str) -> Result<TryOnOutput, TryOnError> {
        if let Some(rest) = item.strip_prefix("data:") {
            let (meta, b64) = rest.split_once(',').ok_or_else(|| TryOnError::Unavailable("malformed data URI".into()))?;
            let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|e| TryOnError::Unavailable(format!("result base64: {e}")))?;
            let mime = if meta.contains("png") { "image/png" } else { "image/jpeg" };
            return Ok(TryOnOutput { image: Bytes::from(bytes), mime });
        }
        let url = reqwest::Url::parse(item).map_err(|e| TryOnError::Unavailable(format!("result url: {e}")))?;
        let host_ok = url.scheme() == "https" && url.host_str().is_some_and(|h| FASHN_CDN_HOSTS.contains(&h));
        if !host_ok {
            return Err(TryOnError::Unavailable("result URL is not on the provider's CDN".into()));
        }
        let resp = self.http.get(url).send().await.map_err(|e| TryOnError::Unavailable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(TryOnError::Unavailable(format!("result download failed: {}", resp.status())));
        }
        let bytes = resp.bytes().await.map_err(|e| TryOnError::Unavailable(e.to_string()))?;
        if bytes.len() > MAX_RESULT_BYTES {
            return Err(TryOnError::Unavailable("result is too large".into()));
        }
        let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) { "image/png" } else { "image/jpeg" };
        Ok(TryOnOutput { image: bytes, mime })
    }
}

#[async_trait]
impl TryOnProvider for FashnTryOn {
    fn name(&self) -> &'static str {
        "fashn"
    }

    fn cost_per_image(&self) -> Tiyin {
        self.cfg.cost_per_image
    }

    async fn generate(&self, req: TryOnRequest) -> Result<TryOnOutput, TryOnError> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let garment = Self::flatten_garment(&req.garment_png)?;
        let body = serde_json::json!({
            "model_name": "tryon-v1.6",
            "inputs": {
                "model_image": format!("data:image/jpeg;base64,{}", b64.encode(&req.person_jpeg)),
                "garment_image": format!("data:image/jpeg;base64,{}", b64.encode(&garment)),
                "category": "tops",
                "garment_photo_type": "flat-lay",
                "mode": self.cfg.mode,
                "moderation_level": "conservative",
                "num_samples": 1,
                "output_format": "jpeg",
                "return_base64": true,
            }
        });
        let base = self.cfg.base_url.trim_end_matches('/');
        let resp = self
            .http
            .post(format!("{base}/v1/run"))
            .bearer_auth(&self.cfg.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| TryOnError::Unavailable(e.to_string()))?;
        let status = resp.status();
        let run: RunResponse = resp.json().await.map_err(|e| TryOnError::Unavailable(format!("run response: {e}")))?;
        let id = match run.id {
            Some(id) if status.is_success() => id,
            _ => {
                let detail = run.message.or_else(|| run.error.map(|e| e.to_string())).unwrap_or_else(|| status.to_string());
                // 4xx other than rate limiting means our request was refused.
                return Err(if status.is_client_error() && status.as_u16() != 429 { TryOnError::Rejected(detail) } else { TryOnError::Unavailable(detail) });
            }
        };
        // Ids come from the provider; make sure one cannot alter the URL path.
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(TryOnError::Unavailable("unexpected prediction id format".into()));
        }

        let deadline = tokio::time::Instant::now() + self.cfg.timeout;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(TryOnError::Timeout);
            }
            tokio::time::sleep(self.cfg.poll_interval).await;
            let resp = self
                .http
                .get(format!("{base}/v1/status/{id}"))
                .bearer_auth(&self.cfg.api_key)
                .send()
                .await
                .map_err(|e| TryOnError::Unavailable(e.to_string()))?;
            if resp.status().is_server_error() || resp.status().as_u16() == 429 {
                continue; // transient; keep polling until the deadline
            }
            let st: StatusResponse = resp.json().await.map_err(|e| TryOnError::Unavailable(format!("status response: {e}")))?;
            match st.status.as_str() {
                "completed" => {
                    let first = st.output.and_then(|o| o.into_iter().next()).ok_or_else(|| TryOnError::Unavailable("no output".into()))?;
                    return self.result_bytes(&first).await;
                }
                "failed" => {
                    let e = st.error.unwrap_or(StatusError { name: String::new(), message: String::new() });
                    let detail = format!("{}: {}", e.name, e.message);
                    // Input problems are the customer's to fix; anything
                    // else is treated as a provider-side failure.
                    let input_errors = ["PoseError", "ImageLoadError", "InputValidationError", "ContentModerationError", "PhotoTypeError"];
                    return Err(if input_errors.contains(&e.name.as_str()) { TryOnError::Rejected(detail) } else { TryOnError::Unavailable(detail) });
                }
                _ => {} // starting, in_queue, processing
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use std::io::Cursor;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn png(w: u32, h: u32, px: [u8; 4]) -> Bytes {
        let img = RgbaImage::from_pixel(w, h, image::Rgba(px));
        let mut buf = Vec::new();
        DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Png).unwrap();
        Bytes::from(buf)
    }

    fn jpeg(w: u32, h: u32, luma: u8) -> Bytes {
        encode_jpeg(&RgbaImage::from_pixel(w, h, image::Rgba([luma, luma, luma, 255])), 90).unwrap()
    }

    #[test]
    fn prechecks() {
        let ok = image::load_from_memory(&jpeg(600, 800, 150)).unwrap();
        assert_eq!(precheck_photo(&ok), Ok(()));
        let small = image::load_from_memory(&jpeg(300, 800, 150)).unwrap();
        assert_eq!(precheck_photo(&small), Err(PhotoIssue::TooSmall));
        let dark = image::load_from_memory(&jpeg(600, 800, 10)).unwrap();
        assert_eq!(precheck_photo(&dark), Err(PhotoIssue::TooDark));
    }

    #[tokio::test]
    async fn mock_puts_the_garment_on_the_photo() {
        let out = MockTryOn { delay: Duration::ZERO }
            .generate(TryOnRequest { person_jpeg: jpeg(600, 800, 200), garment_png: png(100, 110, [255, 0, 0, 255]) })
            .await
            .unwrap();
        assert_eq!(out.mime, "image/jpeg");
        let img = image::load_from_memory(&out.image).unwrap().to_rgb8();
        assert_eq!(img.dimensions(), (600, 800));
        // Centre of the pasted garment is red, a top corner is still grey.
        let c = img.get_pixel(300, 400).0;
        assert!(c[0] > 200 && c[1] < 60, "{c:?}");
        let k = img.get_pixel(10, 10).0;
        assert!(k[0].abs_diff(200) < 12 && k[1].abs_diff(200) < 12, "{k:?}");
        // Bottom band is the mock marker (yellow or near-black, not grey).
        let b = img.get_pixel(3, 798).0;
        assert!(b[2] < 80, "{b:?}");
    }

    #[tokio::test]
    async fn mock_rejects_garbage() {
        let r = MockTryOn { delay: Duration::ZERO }
            .generate(TryOnRequest { person_jpeg: Bytes::from_static(b"not an image"), garment_png: png(4, 4, [0; 4]) })
            .await;
        assert!(matches!(r, Err(TryOnError::Rejected(_))));
    }

    #[derive(Clone)]
    struct Fake {
        polls: Arc<AtomicUsize>,
        fail_with: Option<&'static str>,
        result: String,
    }

    /// A local server that behaves like the documented FASHN endpoints.
    async fn fake_fashn(fake: Fake) -> String {
        async fn run(headers: HeaderMap, Json(body): Json<serde_json::Value>) -> (StatusCode, Json<serde_json::Value>) {
            if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer test-key") {
                return (StatusCode::UNAUTHORIZED, Json(serde_json::json!({"error": "UnauthorizedAccess", "message": "Unauthorized: Invalid token"})));
            }
            let inputs = &body["inputs"];
            let ok = body["model_name"] == "tryon-v1.6"
                && inputs["model_image"].as_str().is_some_and(|s| s.starts_with("data:image/jpeg;base64,"))
                && inputs["garment_image"].as_str().is_some_and(|s| s.starts_with("data:image/jpeg;base64,"))
                && inputs["return_base64"] == true;
            if !ok {
                return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": "BadRequest", "message": "bad inputs"})));
            }
            (StatusCode::OK, Json(serde_json::json!({"id": "pred-123_u1", "error": null})))
        }
        async fn status(State(f): State<Fake>, Path(id): Path<String>) -> Json<serde_json::Value> {
            assert_eq!(id, "pred-123_u1");
            let n = f.polls.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                return Json(serde_json::json!({"id": id, "status": if n == 0 { "in_queue" } else { "processing" }, "output": null, "error": null}));
            }
            match f.fail_with {
                Some(name) => Json(serde_json::json!({"id": id, "status": "failed", "output": null, "error": {"name": name, "message": "nope"}})),
                None => Json(serde_json::json!({"id": id, "status": "completed", "output": [f.result], "error": null})),
            }
        }
        let app = Router::new().route("/v1/run", post(run)).route("/v1/status/{id}", get(status)).with_state(fake);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn client(base_url: String, key: &str) -> FashnTryOn {
        FashnTryOn::new(FashnConfig {
            api_key: key.into(),
            base_url,
            mode: "balanced".into(),
            timeout: Duration::from_secs(5),
            poll_interval: Duration::from_millis(5),
            cost_per_image: Tiyin(95_000),
        })
        .unwrap()
    }

    fn request() -> TryOnRequest {
        TryOnRequest { person_jpeg: jpeg(64, 64, 120), garment_png: png(32, 32, [0, 0, 255, 128]) }
    }

    #[tokio::test]
    async fn fashn_happy_path_polls_until_completed() {
        let result = jpeg(8, 8, 77);
        let data_uri = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(&result));
        let polls = Arc::new(AtomicUsize::new(0));
        let base = fake_fashn(Fake { polls: polls.clone(), fail_with: None, result: data_uri }).await;
        let out = client(base, "test-key").generate(request()).await.unwrap();
        assert_eq!(out.image, result);
        assert_eq!(out.mime, "image/jpeg");
        assert_eq!(polls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn fashn_maps_errors() {
        let polls = Arc::new(AtomicUsize::new(0));
        let base = fake_fashn(Fake { polls, fail_with: Some("PoseError"), result: String::new() }).await;
        assert!(matches!(client(base.clone(), "test-key").generate(request()).await, Err(TryOnError::Rejected(_))));
        // Wrong key: the run call is refused outright.
        assert!(matches!(client(base, "wrong").generate(request()).await, Err(TryOnError::Rejected(_))));

        let polls = Arc::new(AtomicUsize::new(0));
        let base = fake_fashn(Fake { polls, fail_with: Some("PipelineError"), result: String::new() }).await;
        assert!(matches!(client(base, "test-key").generate(request()).await, Err(TryOnError::Unavailable(_))));
    }

    #[tokio::test]
    async fn fashn_refuses_result_urls_outside_the_cdn() {
        let polls = Arc::new(AtomicUsize::new(0));
        let base = fake_fashn(Fake { polls, fail_with: None, result: "http://169.254.169.254/latest/meta-data".into() }).await;
        let r = client(base, "test-key").generate(request()).await;
        assert!(matches!(r, Err(TryOnError::Unavailable(m)) if m.contains("CDN")));
    }
}
