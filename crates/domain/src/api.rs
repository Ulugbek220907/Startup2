//! JSON types exchanged between the web app and the server.
//!
//! Both sides compile against these structs, so a renamed field is a
//! compile error instead of a runtime surprise. All money fields are tiyin.

use design::{DesignDoc, GarmentKind, Side};
use serde::{Deserialize, Serialize};

use crate::i18n::Lang;
use crate::money::Tiyin;
use crate::order::{DeliveryMethod, ModerationStatus, OrderStatus};
use crate::pricing::{AreaTier, QuantityDiscount, Quote};

/// A string in all three UI languages.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct L10n {
    pub uz: String,
    pub ru: String,
    pub en: String,
}

impl L10n {
    pub fn get(&self, lang: Lang) -> &str {
        match lang {
            Lang::Uz => &self.uz,
            Lang::Ru => &self.ru,
            Lang::En => &self.en,
        }
    }
}

/// Error body returned with every non-2xx response.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub error: ApiErrorBody,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiErrorBody {
    /// Stable machine-readable code, e.g. `invalid_phone`. The web app maps
    /// it to a translated message (`error.<code>`).
    pub code: String,
    /// English detail for developers and logs.
    pub message: String,
}

// ------------------------------------------------------------- catalog

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogDto {
    pub brand: String,
    pub garments: Vec<GarmentDto>,
    pub regions: Vec<RegionDto>,
    pub price_tiers: Vec<AreaTier>,
    pub quantity_discounts: Vec<QuantityDiscount>,
    pub pickup_address: L10n,
    pub max_upload_bytes: u64,
    pub max_quantity: u32,
    /// Hours until try-on photos are deleted automatically.
    pub tryon_ttl_hours: u32,
    /// Username of the Telegram bot (without @), if one is configured.
    pub telegram_bot: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GarmentDto {
    pub slug: String,
    pub kind: GarmentKind,
    pub name: L10n,
    pub base_price: Tiyin,
    pub colors: Vec<ColorDto>,
    pub sizes: Vec<String>,
    pub areas: Vec<PrintAreaDto>,
}

impl GarmentDto {
    pub fn area(&self, side: Side) -> Option<&PrintAreaDto> {
        self.areas.iter().find(|a| a.side == side)
    }

    pub fn color(&self, slug: &str) -> Option<&ColorDto> {
        self.colors.iter().find(|c| c.slug == slug)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorDto {
    pub slug: String,
    pub name: L10n,
    /// `#rrggbb`
    pub hex: String,
}

/// Printable region of one garment side. Physical size is in cm; the
/// `mock_*` fields place it on the garment artwork as fractions (0..1) of
/// the artwork's width and height.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrintAreaDto {
    pub side: Side,
    pub width_cm: f32,
    pub height_cm: f32,
    pub mock_x: f32,
    pub mock_y: f32,
    pub mock_w: f32,
    pub mock_h: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionDto {
    pub slug: String,
    pub name: L10n,
    pub delivery_fee: Tiyin,
}

// -------------------------------------------------------------- assets

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetDto {
    pub id: String,
    pub width: u32,
    pub height: u32,
    /// Path of a downscaled preview for the editor.
    pub preview_url: String,
}

// ------------------------------------------------------------- designs

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SaveDesignRequest {
    pub doc: DesignDoc,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignSavedDto {
    /// Public id used in share links.
    pub slug: String,
    /// Secret that allows editing; kept in the creator's browser only.
    pub edit_token: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignDto {
    pub slug: String,
    pub doc: DesignDoc,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobDto {
    pub id: String,
    pub status: JobState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportKind {
    /// Offensive or otherwise inappropriate.
    Report,
    /// A rights holder says the design uses their trademark/artwork.
    Trademark,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportRequest {
    pub kind: ReportKind,
    pub reason: String,
    /// How to reach the reporter (required for trademark claims).
    pub contact: Option<String>,
}

// -------------------------------------------------------------- orders

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryChoice {
    pub method: DeliveryMethod,
    /// Region slug; required for `delivery`.
    pub region: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuoteRequest {
    pub doc: DesignDoc,
    pub quantity: u32,
    pub delivery: DeliveryChoice,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CreateOrderRequest {
    pub doc: DesignDoc,
    /// Share slug of the saved design, if any (kept for reference only; the
    /// order stores its own frozen copy of `doc`).
    pub design_slug: Option<String>,
    pub size: String,
    pub quantity: u32,
    pub phone: String,
    pub customer_name: Option<String>,
    pub delivery: DeliveryChoice,
    pub address: Option<String>,
    pub notes: Option<String>,
    pub lang: Lang,
    /// The customer confirmed they own or may use the uploaded artwork.
    pub accept_rights: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderCreatedDto {
    pub code: String,
    pub status: OrderStatus,
    pub quote: Quote,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderLookupRequest {
    pub code: String,
    pub phone: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusEventDto {
    pub status: OrderStatus,
    /// RFC 3339 timestamp.
    pub at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderPublicDto {
    pub code: String,
    pub status: OrderStatus,
    pub created_at: String,
    pub garment_name: String,
    pub color: String,
    pub size: String,
    pub quantity: u32,
    pub delivery_method: DeliveryMethod,
    pub quote: Quote,
    pub timeline: Vec<StatusEventDto>,
}

/// Proof of identity for export/delete: the phone plus any order code that
/// belongs to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivacyRequest {
    pub phone: String,
    pub code: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeletedDto {
    pub deleted: bool,
}

// -------------------------------------------------------------- try-on

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TryOnState {
    Queued,
    Running,
    Done,
    Failed,
    Deleted,
}

impl TryOnState {
    pub fn as_str(&self) -> &'static str {
        match self {
            TryOnState::Queued => "queued",
            TryOnState::Running => "running",
            TryOnState::Done => "done",
            TryOnState::Failed => "failed",
            TryOnState::Deleted => "deleted",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Self::Queued, Self::Running, Self::Done, Self::Failed, Self::Deleted].into_iter().find(|x| x.as_str() == s)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TryOnDto {
    pub id: String,
    pub status: TryOnState,
    pub result_url: Option<String>,
    /// Machine-readable failure code (see `error.tryon_*` translations).
    pub error: Option<String>,
    pub expires_at: String,
}

// ------------------------------------------------------------ telegram

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramAuthRequest {
    /// Raw `Telegram.WebApp.initData` query string.
    pub init_data: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelegramMeDto {
    pub lang: Lang,
    /// Linked phone, if the user shared their contact with the bot.
    pub phone: Option<String>,
    pub first_name: Option<String>,
    pub orders: Vec<OrderSummaryDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderSummaryDto {
    pub code: String,
    pub status: OrderStatus,
    pub created_at: String,
    pub total: Tiyin,
}

// --------------------------------------------------------------- admin

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminMeDto {
    pub username: String,
    /// Must be echoed in the `X-CSRF-Token` header of every mutating call.
    pub csrf_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminOrderRow {
    pub id: String,
    pub code: String,
    pub status: OrderStatus,
    pub created_at: String,
    pub phone: String,
    pub customer_name: Option<String>,
    pub garment_name: String,
    pub size: String,
    pub quantity: u32,
    pub total: Tiyin,
    pub moderation: ModerationStatus,
    pub files_ready: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminOrderList {
    pub orders: Vec<AdminOrderRow>,
    pub total: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderFileDto {
    /// `mockup_front`, `mockup_back`, `print_front`, `print_back`, `sheet`.
    pub kind: String,
    pub mime: String,
    pub bytes: i64,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusLogDto {
    pub from: Option<OrderStatus>,
    pub to: OrderStatus,
    pub actor: String,
    pub note: Option<String>,
    pub at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdminOrderDetail {
    pub row: AdminOrderRow,
    pub color: String,
    pub delivery_method: DeliveryMethod,
    pub region: Option<String>,
    pub address: Option<String>,
    pub notes: Option<String>,
    pub lang: Lang,
    pub quote: Quote,
    pub files: Vec<OrderFileDto>,
    /// `pending`, `ready` or `failed`.
    pub files_status: String,
    pub log: Vec<StatusLogDto>,
    pub allowed_next: Vec<OrderStatus>,
    pub design_id: String,
    pub moderation_reason: Option<String>,
    pub doc: DesignDoc,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusChangeRequest {
    pub to: OrderStatus,
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModerationItemDto {
    pub design_id: String,
    pub status: ModerationStatus,
    pub reason: Option<String>,
    pub order_code: Option<String>,
    pub text: String,
    pub preview_url: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModerationDecision {
    pub approve: bool,
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportDto {
    pub id: String,
    pub design_id: String,
    pub design_slug: Option<String>,
    pub kind: ReportKind,
    pub reason: String,
    pub contact: Option<String>,
    /// `open`, `dismissed` or `removed`.
    pub status: String,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportDecision {
    /// true = take the design down (reject it), false = dismiss the report.
    pub remove: bool,
    pub note: Option<String>,
}

/// Everything the price editor shows and saves in one go.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PricingConfigDto {
    pub garments: Vec<GarmentPriceDto>,
    pub tiers: Vec<AreaTier>,
    pub discounts: Vec<QuantityDiscount>,
    pub regions: Vec<RegionDto>,
    pub areas: Vec<AdminPrintAreaDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GarmentPriceDto {
    pub slug: String,
    pub name: String,
    pub base_price: Tiyin,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdminPrintAreaDto {
    pub garment: String,
    #[serde(flatten)]
    pub area: PrintAreaDto,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlocklistDto {
    pub terms: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AuditEntryDto {
    pub at: String,
    pub actor: String,
    pub action: String,
    pub entity: String,
    pub entity_id: Option<String>,
    pub detail: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OkDto {
    pub ok: bool,
}
