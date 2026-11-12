//! HTTP client for the backend, built directly on `fetch`.

use domain::api::ApiError;
use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Request, RequestInit, Response};

/// An API failure: the server's error code (`error.<code>` translation key
/// suffix), or `network` when the request did not complete.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiFail {
    pub status: u16,
    pub code: String,
}

impl ApiFail {
    fn network() -> Self {
        ApiFail { status: 0, code: "network".into() }
    }

    /// Translation key for this failure.
    pub fn key(&self) -> String {
        format!("error.{}", self.code)
    }
}

pub type ApiResult<T> = Result<T, ApiFail>;

pub fn url(path: &str) -> String {
    format!("{}{}", crate::platform::api_base(), path)
}

enum Body<'a> {
    None,
    Json(String),
    Bytes(&'a [u8]),
}

async fn send(method: &str, path: &str, body: Body<'_>, headers: &[(&str, &str)]) -> ApiResult<Response> {
    let init = RequestInit::new();
    init.set_method(method);
    // Cookies are only needed (and only sent) for the same-origin admin API.
    init.set_credentials(web_sys::RequestCredentials::SameOrigin);
    let req_headers = web_sys::Headers::new().map_err(|_| ApiFail::network())?;
    match &body {
        Body::None => {}
        Body::Json(s) => {
            let _ = req_headers.set("content-type", "application/json");
            init.set_body(&JsValue::from_str(s));
        }
        Body::Bytes(b) => {
            let _ = req_headers.set("content-type", "application/octet-stream");
            // Copy into a JS-owned buffer; a view into WASM memory could be
            // invalidated if memory grows while the request is in flight.
            let arr = js_sys::Uint8Array::new_with_length(b.len() as u32);
            arr.copy_from(b);
            init.set_body(&arr);
        }
    }
    for (k, v) in headers {
        let _ = req_headers.set(k, v);
    }
    init.set_headers(&req_headers);
    let request = Request::new_with_str_and_init(&url(path), &init).map_err(|_| ApiFail::network())?;
    let window = web_sys::window().ok_or_else(ApiFail::network)?;
    let resp = JsFuture::from(window.fetch_with_request(&request)).await.map_err(|_| ApiFail::network())?;
    let resp: Response = resp.dyn_into().map_err(|_| ApiFail::network())?;
    if resp.ok() {
        return Ok(resp);
    }
    let status = resp.status();
    // Error bodies are `{ "error": { "code": ..., "message": ... } }`.
    let code = match resp.text() {
        Ok(p) => JsFuture::from(p).await.ok().and_then(|v| v.as_string()),
        Err(_) => None,
    }
    .and_then(|t| serde_json::from_str::<ApiError>(&t).ok())
    .map(|e| e.error.code)
    .unwrap_or_else(|| match status {
        413 => "upload_too_large".into(),
        429 => "rate_limited".into(),
        _ => "generic".into(),
    });
    Err(ApiFail { status, code })
}

async fn parse<T: DeserializeOwned>(resp: Response) -> ApiResult<T> {
    let text = JsFuture::from(resp.text().map_err(|_| ApiFail::network())?).await.map_err(|_| ApiFail::network())?;
    serde_json::from_str(&text.as_string().unwrap_or_default()).map_err(|_| ApiFail { status: 0, code: "generic".into() })
}

pub async fn get<T: DeserializeOwned>(path: &str) -> ApiResult<T> {
    parse(send("GET", path, Body::None, &[]).await?).await
}

pub async fn post<B: Serialize, T: DeserializeOwned>(path: &str, body: &B) -> ApiResult<T> {
    request("POST", path, body, &[]).await
}

pub async fn request<B: Serialize, T: DeserializeOwned>(method: &str, path: &str, body: &B, headers: &[(&str, &str)]) -> ApiResult<T> {
    let json = serde_json::to_string(body).map_err(|_| ApiFail { status: 0, code: "generic".into() })?;
    parse(send(method, path, Body::Json(json), headers).await?).await
}

pub async fn post_bytes<T: DeserializeOwned>(path: &str, bytes: &[u8], headers: &[(&str, &str)]) -> ApiResult<T> {
    parse(send("POST", path, Body::Bytes(bytes), headers).await?).await
}

pub async fn delete<T: DeserializeOwned>(path: &str) -> ApiResult<T> {
    parse(send("DELETE", path, Body::None, &[]).await?).await
}

/// Download a binary response (fonts).
pub async fn get_bytes(path: &str) -> ApiResult<Vec<u8>> {
    let resp = send("GET", path, Body::None, &[]).await?;
    let buf = JsFuture::from(resp.array_buffer().map_err(|_| ApiFail::network())?).await.map_err(|_| ApiFail::network())?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

/// Read a user-selected file into memory.
pub async fn read_file(file: &web_sys::File) -> Option<Vec<u8>> {
    let buf = JsFuture::from(file.array_buffer()).await.ok()?;
    Some(js_sys::Uint8Array::new(&buf).to_vec())
}
