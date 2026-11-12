//! QiyiqDoʻppi web app (customer app + admin page), compiled to WebAssembly.
//!
//! Almost everything is Rust. The only JavaScript is in `shims/`: thin,
//! typed wrappers for host APIs that have no Rust binding (see the header
//! comment of each shim for why it exists).

pub mod admin;
pub mod api;
pub mod app;
pub mod draw;
pub mod icons;
pub mod pages;
pub mod platform;
pub mod router;
pub mod state;
pub mod tryon_gl;
