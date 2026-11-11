//! The design document and everything needed to turn it into drawing
//! operations.
//!
//! This crate is the single source of truth shared by the browser editor
//! (compiled to WASM) and the server renderer. Both sides call
//! [`display::side_ops`] and get the *same* list of vector paths, so what the
//! customer sees in the editor is what gets printed. Text is converted to
//! glyph outlines here (via `ttf-parser`) instead of relying on the browser's
//! or the server's text engine, which would never match pixel for pixel.
//!
//! All coordinates are centimetres inside the print area, origin top-left,
//! y pointing down.

pub mod display;
pub mod doc;
pub mod fonts;
pub mod garment_art;
pub mod geom;
pub mod shapes;
pub mod stickers;
pub mod svgpath;
pub mod templates;
pub mod text;
pub mod tryon;
pub mod validate;

pub use doc::*;
pub use geom::{Affine, Rect};
