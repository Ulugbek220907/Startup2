//! Typed Rust declarations for the JavaScript shims.
//!
//! `#[wasm_bindgen(module = "/shims/...")]` makes wasm-bindgen bundle the
//! file and generate the glue; from Rust these are ordinary functions.

use js_sys::{Float32Array, Promise, Uint8Array};
use wasm_bindgen::prelude::*;
use web_sys::{Blob, HtmlCanvasElement, HtmlImageElement, HtmlVideoElement};

#[wasm_bindgen(module = "/shims/platform.js")]
extern "C" {
    #[wasm_bindgen(js_name = apiBase)]
    pub fn api_base() -> String;

    #[wasm_bindgen(js_name = tgInitData)]
    pub fn tg_init_data() -> String;
    #[wasm_bindgen(js_name = tgLanguage)]
    pub fn tg_language() -> String;
    #[wasm_bindgen(js_name = tgReady)]
    pub fn tg_ready();

    #[wasm_bindgen(js_name = isNative)]
    pub fn is_native() -> bool;
    #[wasm_bindgen(js_name = platformName)]
    pub fn platform_name() -> String;
    pub fn haptic(kind: &str);
    /// Resolves to "shared" | "copied" | "downloaded" | "failed".
    pub fn share(title: &str, url: Option<&str>, blob: Option<&Blob>, filename: &str) -> Promise;
    #[wasm_bindgen(js_name = canvasBlob)]
    pub fn canvas_blob(canvas: &HtmlCanvasElement, mime: &str, quality: f64) -> Promise;

    /// Resolves to "ok" | "denied" | "unavailable".
    #[wasm_bindgen(js_name = startCamera)]
    pub fn start_camera(video: &HtmlVideoElement, facing: &str) -> Promise;
    #[wasm_bindgen(js_name = stopCamera)]
    pub fn stop_camera(video: &HtmlVideoElement);

    #[wasm_bindgen(js_name = registerServiceWorker)]
    pub fn register_service_worker();
    #[wasm_bindgen(js_name = initNative)]
    pub fn init_native();
    #[wasm_bindgen(js_name = removeBootSplash)]
    pub fn remove_boot_splash();
}

#[wasm_bindgen(module = "/shims/pose.js")]
extern "C" {
    /// Loads the model; the callback receives progress 0..=100. Resolves to
    /// a bool: false when the device cannot run it.
    #[wasm_bindgen(js_name = poseInit)]
    pub fn pose_init(on_progress: &Closure<dyn FnMut(f64)>) -> Promise;
    /// Number of people found in the frame (0 or 1).
    #[wasm_bindgen(js_name = poseDetect)]
    pub fn pose_detect(video: &HtmlVideoElement, timestamp_ms: f64) -> u32;
    #[wasm_bindgen(js_name = poseWantMask)]
    pub fn pose_want_mask(on: bool);
    /// 33 x (x, y, visibility), normalised to the frame.
    #[wasm_bindgen(js_name = poseLandmarks)]
    pub fn pose_landmarks() -> Float32Array;
    #[wasm_bindgen(js_name = poseMask)]
    pub fn pose_mask() -> Uint8Array;
    #[wasm_bindgen(js_name = poseMaskWidth)]
    pub fn pose_mask_width() -> u32;
    #[wasm_bindgen(js_name = poseMaskHeight)]
    pub fn pose_mask_height() -> u32;
    /// -1 unavailable, 0 nobody, 1 ok, 2 several people, 3 torso cut off.
    #[wasm_bindgen(js_name = poseCheckImage)]
    pub fn pose_check_image(image: &HtmlImageElement) -> Promise;
}

/// Await a shim promise that resolves to a string.
pub async fn await_string(p: Promise) -> String {
    wasm_bindgen_futures::JsFuture::from(p).await.ok().and_then(|v| v.as_string()).unwrap_or_default()
}
