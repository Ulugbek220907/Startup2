//! App-wide state: language, catalog, the design being edited, fonts.
//!
//! The context is created once in `App` and stored in a thread-local so that
//! event handlers and async tasks (which run outside Leptos' component tree)
//! can reach it. Every field is a signal handle, which is `Copy`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use design::fonts::{FontId, FontSource};
use design::{DesignDoc, GarmentKind, LayerKind, Side};
use domain::api::{CatalogDto, DesignSavedDto, GarmentDto, SaveDesignRequest, TelegramMeDto};
use domain::Lang;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::api::{self, ApiFail};

/// The design in progress plus the server identity it was saved under.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    pub doc: Option<DesignDoc>,
    pub slug: Option<String>,
    pub edit_token: Option<String>,
    /// The document as last saved to the server, to skip redundant saves.
    #[serde(skip)]
    pub saved_doc: Option<DesignDoc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedOrder {
    pub code: String,
    pub phone: String,
}

#[derive(Clone, Copy)]
pub struct Ctx {
    pub lang: RwSignal<Lang>,
    pub catalog: RwSignal<Option<CatalogDto>>,
    pub catalog_error: RwSignal<bool>,
    pub draft: RwSignal<Draft>,
    pub side: RwSignal<Side>,
    pub size: RwSignal<Option<String>>,
    pub quantity: RwSignal<u32>,
    /// `(message, is_error)`
    pub toast: RwSignal<Option<(String, bool)>>,
    /// Bumped when a font or image finishes loading, so canvases redraw.
    pub redraw: RwSignal<u32>,
    pub telegram: RwSignal<Option<TelegramMeDto>>,
    pub orders: RwSignal<Vec<SavedOrder>>,
}

thread_local! {
    static CTX: Cell<Option<Ctx>> = const { Cell::new(None) };
    // Font bytes are leaked on purpose: six small files that live for the
    // whole session, which lets `FontSource` hand out plain `&[u8]`.
    static FONTS: RefCell<HashMap<FontId, &'static [u8]>> = RefCell::new(HashMap::new());
    static FONTS_LOADING: RefCell<Vec<FontId>> = const { RefCell::new(Vec::new()) };
}

pub fn ctx() -> Ctx {
    CTX.with(|c| c.get()).expect("app context is created before any component renders")
}

/// Translate in the current language. Reads the language signal, so text
/// inside `move ||` closures updates when the language changes.
pub fn t(key: &str) -> &'static str {
    domain::i18n::t(ctx().lang.get(), key)
}

pub fn tf(key: &str, args: &[(&str, &str)]) -> String {
    domain::i18n::tf(ctx().lang.get(), key, args)
}

// ------------------------------------------------------- local storage

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

pub fn store_get<T: for<'de> Deserialize<'de>>(key: &str) -> Option<T> {
    serde_json::from_str(&storage()?.get_item(key).ok()??).ok()
}

pub fn store_set<T: Serialize>(key: &str, value: &T) {
    if let (Some(s), Ok(json)) = (storage(), serde_json::to_string(value)) {
        // Fails in private mode or when full; the app works without it.
        let _ = s.set_item(key, &json);
    }
}

/// Stable random id for this browser, used to scope try-on limits.
pub fn device_id() -> String {
    if let Some(id) = store_get::<String>("qd.device") {
        return id;
    }
    let id = web_sys::window().and_then(|w| w.crypto().ok()).map(|c| c.random_uuid()).unwrap_or_else(|| format!("dev-{:016x}", js_sys::Math::random().to_bits()));
    store_set("qd.device", &id);
    id
}

pub fn new_layer_id() -> String {
    thread_local!(static N: Cell<u32> = const { Cell::new(0) });
    let n = N.with(|n| {
        n.set(n.get() + 1);
        n.get()
    });
    format!("l{:x}{n:x}", js_sys::Date::now() as u64)
}

// ---------------------------------------------------------------- init

impl Ctx {
    pub fn init() -> Ctx {
        let tg_lang = crate::platform::tg_language();
        let lang = store_get::<Lang>("qd.lang").unwrap_or_else(|| {
            if !tg_lang.is_empty() {
                return Lang::detect(&tg_lang);
            }
            // navigator.languages is the user's ordered preference list.
            let prefs: Vec<String> = web_sys::window()
                .map(|w| w.navigator().languages().iter().filter_map(|v| v.as_string()).collect())
                .unwrap_or_default();
            Lang::detect_from(prefs.iter().map(String::as_str))
        });
        let mut draft: Draft = store_get("qd.draft").unwrap_or_default();
        // A stored draft from an older schema version is dropped.
        if draft.doc.as_ref().is_some_and(|d| d.version != design::SCHEMA_VERSION) {
            draft = Draft::default();
        }
        let ctx = Ctx {
            lang: RwSignal::new(lang),
            catalog: RwSignal::new(None),
            catalog_error: RwSignal::new(false),
            draft: RwSignal::new(draft),
            side: RwSignal::new(Side::Front),
            size: RwSignal::new(None),
            quantity: RwSignal::new(1),
            toast: RwSignal::new(None),
            redraw: RwSignal::new(0),
            telegram: RwSignal::new(None),
            orders: RwSignal::new(store_get("qd.orders").unwrap_or_default()),
        };
        CTX.with(|c| c.set(Some(ctx)));
        ctx.load_catalog();
        ctx.load_telegram();
        ctx
    }

    pub fn set_lang(&self, lang: Lang) {
        self.lang.set(lang);
        store_set("qd.lang", &lang);
        if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.document_element()) {
            let _ = el.set_attribute("lang", lang.code());
        }
    }

    pub fn load_catalog(&self) {
        let ctx = *self;
        ctx.catalog_error.set(false);
        leptos::task::spawn_local(async move {
            match api::get::<CatalogDto>("/api/catalog").await {
                Ok(c) => ctx.catalog.set(Some(c)),
                Err(_) => ctx.catalog_error.set(true),
            }
        });
    }

    fn load_telegram(&self) {
        let init_data = crate::platform::tg_init_data();
        if init_data.is_empty() {
            return;
        }
        crate::platform::tg_ready();
        let ctx = *self;
        leptos::task::spawn_local(async move {
            let req = domain::api::TelegramAuthRequest { init_data };
            if let Ok(me) = api::post::<_, TelegramMeDto>("/api/telegram/me", &req).await {
                // Telegram's language wins unless the user picked one here.
                if store_get::<Lang>("qd.lang").is_none() {
                    ctx.lang.set(me.lang);
                }
                ctx.telegram.set(Some(me));
            }
        });
    }

    pub fn show_toast(&self, message: impl Into<String>, is_error: bool) {
        let message = message.into();
        self.toast.set(Some((message.clone(), is_error)));
        crate::platform::haptic(if is_error { "error" } else { "light" });
        let toast = self.toast;
        set_timeout(
            move || {
                // Only clear if a newer toast has not replaced this one.
                if toast.get_untracked().as_ref().map(|(m, _)| m) == Some(&message) {
                    toast.set(None);
                }
            },
            std::time::Duration::from_millis(3500),
        );
    }

    pub fn show_error(&self, fail: &ApiFail) {
        let lang = self.lang.get_untracked();
        let mut text = domain::i18n::tf(lang, &fail.key(), &[]);
        // Unknown codes fall back to the generic message.
        if text == fail.key() {
            text = domain::i18n::t(lang, "error.generic").to_string();
        }
        self.show_toast(text, true);
    }

    /// The garment of the current draft.
    pub fn garment(&self) -> Option<GarmentDto> {
        let slug = self.draft.with(|d| d.doc.as_ref().map(|doc| doc.garment.clone()))?;
        self.catalog.with(|c| c.as_ref()?.garments.iter().find(|g| g.slug == slug).cloned())
    }

    /// Replace the document and persist the draft locally.
    pub fn set_doc(&self, doc: DesignDoc) {
        self.draft.update(|d| d.doc = Some(doc));
        self.persist_draft();
    }

    pub fn persist_draft(&self) {
        self.draft.with_untracked(|d| store_set("qd.draft", d));
    }

    pub fn remember_order(&self, code: &str, phone: &str) {
        self.orders.update(|o| {
            o.retain(|x| x.code != code);
            o.insert(0, SavedOrder { code: code.to_string(), phone: phone.to_string() });
            o.truncate(20);
        });
        self.orders.with_untracked(|o| store_set("qd.orders", o));
    }

    /// Save the draft on the server (create, or update with the edit token)
    /// and return its share slug.
    pub async fn ensure_saved(&self) -> Result<String, ApiFail> {
        let draft = self.draft.get_untracked();
        let doc = draft.doc.clone().ok_or(ApiFail { status: 0, code: "empty_design".into() })?;
        if let (Some(slug), Some(saved)) = (&draft.slug, &draft.saved_doc) {
            if *saved == doc {
                return Ok(slug.clone());
            }
        }
        let body = SaveDesignRequest { doc: doc.clone() };
        if let (Some(slug), Some(token)) = (&draft.slug, &draft.edit_token) {
            let r: Result<domain::api::OkDto, _> = api::request("PUT", &format!("/api/designs/{slug}"), &body, &[("x-edit-token", token)]).await;
            if r.is_ok() {
                self.draft.update(|d| d.saved_doc = Some(doc));
                return Ok(slug.clone());
            }
            // The server copy is gone or was taken down: save as new below.
        }
        let saved: DesignSavedDto = api::post("/api/designs", &body).await?;
        self.draft.update(|d| {
            d.slug = Some(saved.slug.clone());
            d.edit_token = Some(saved.edit_token);
            d.saved_doc = Some(doc);
        });
        self.persist_draft();
        Ok(saved.slug)
    }
}

// --------------------------------------------------------------- fonts

/// Fonts fetched so far.
pub struct Fonts;

impl FontSource for Fonts {
    fn font_data(&self, id: FontId) -> Option<&[u8]> {
        FONTS.with(|f| f.borrow().get(&id).copied())
    }
}

/// Fetch a print font if it is not loaded yet; bumps `redraw` when ready.
pub fn ensure_font(id: FontId) {
    let needed = FONTS.with(|f| !f.borrow().contains_key(&id)) && FONTS_LOADING.with(|l| !l.borrow().contains(&id));
    if !needed {
        return;
    }
    FONTS_LOADING.with(|l| l.borrow_mut().push(id));
    leptos::task::spawn_local(async move {
        // Fonts are static files served next to the app, not by the API.
        let result = fetch_static(&format!("/fonts/{}", id.file_name())).await;
        FONTS_LOADING.with(|l| l.borrow_mut().retain(|x| *x != id));
        if let Some(bytes) = result {
            FONTS.with(|f| f.borrow_mut().insert(id, Box::leak(bytes.into_boxed_slice())));
            ctx().redraw.update(|n| *n += 1);
        }
    });
}

async fn fetch_static(path: &str) -> Option<Vec<u8>> {
    use wasm_bindgen::JsCast;
    let window = web_sys::window()?;
    let resp = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(path)).await.ok()?;
    let resp: web_sys::Response = resp.dyn_into().ok()?;
    if !resp.ok() {
        return None;
    }
    let buf = wasm_bindgen_futures::JsFuture::from(resp.array_buffer().ok()?).await.ok()?;
    Some(js_sys::Uint8Array::new(&buf).to_vec())
}

/// Make sure every font used by the document is loading or loaded.
pub fn ensure_doc_fonts(doc: &DesignDoc) {
    for layer in doc.front.iter().chain(doc.back.iter()) {
        if let LayerKind::Text(t) = &layer.kind {
            ensure_font(t.font);
        }
    }
}

/// Start a new design for a garment, keeping the first colour as default.
pub fn new_doc_for(garment: &GarmentDto) -> DesignDoc {
    let color = garment.colors.first().map(|c| c.slug.clone()).unwrap_or_default();
    DesignDoc::new(garment.slug.clone(), color)
}

pub fn kind_of(garment: &GarmentDto) -> GarmentKind {
    garment.kind
}
