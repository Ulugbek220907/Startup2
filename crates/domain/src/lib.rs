//! Business rules with no I/O. Everything here compiles for both the server
//! and the browser (WASM), so the same validation and formatting runs on
//! both sides.

pub mod api;
pub mod i18n;
pub mod moderation;
pub mod money;
pub mod order;
pub mod phone;
pub mod pricing;

pub use i18n::Lang;
pub use money::Tiyin;
