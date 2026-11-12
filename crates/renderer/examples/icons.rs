//! Renders the app icon SVG to the PNG sizes the PWA and the mobile apps
//! need. Run from the repository root:
//!
//!   cargo run -p renderer --example icons
//!
//! Outputs are committed, so this only needs re-running when the icon changes.

use resvg::tiny_skia::{Color, Pixmap, Transform};
use resvg::usvg;

fn render(svg: &str, size: u32, inset: f32, background: Option<Color>) -> Vec<u8> {
    let tree = usvg::Tree::from_str(svg, &usvg::Options::default()).expect("icon.svg is valid");
    let mut pm = Pixmap::new(size, size).expect("non-zero size");
    if let Some(bg) = background {
        pm.fill(bg);
    }
    // `inset` shrinks the artwork towards the centre (maskable icons need a
    // safe zone because launchers crop them to circles and squircles).
    let k = size as f32 / 512.0 * (1.0 - 2.0 * inset);
    let off = size as f32 * inset;
    resvg::render(&tree, Transform::from_scale(k, k).post_translate(off, off), &mut pm.as_mut());
    pm.encode_png().expect("png encoding")
}

fn main() {
    let svg = std::fs::read_to_string("crates/web/public/icons/icon.svg").expect("run from the repository root");
    let bg = Color::from_rgba8(13, 14, 17, 255);
    let out = [
        ("crates/web/public/icons/icon-192.png", 192, 0.0, None),
        ("crates/web/public/icons/icon-512.png", 512, 0.0, None),
        ("crates/web/public/icons/icon-maskable-512.png", 512, 0.12, Some(bg)),
        // Sources for `@capacitor/assets` (launcher icons and splash screens).
        ("mobile/assets/icon-only.png", 1024, 0.0, Some(bg)),
        ("mobile/assets/icon-foreground.png", 1024, 0.18, None),
        ("mobile/assets/splash.png", 2732, 0.36, Some(bg)),
        ("mobile/assets/splash-dark.png", 2732, 0.36, Some(bg)),
    ];
    std::fs::create_dir_all("mobile/assets").expect("create mobile/assets");
    for (path, size, inset, background) in out {
        std::fs::write(path, render(&svg, size, inset, background)).unwrap_or_else(|e| panic!("{path}: {e}"));
        println!("wrote {path} ({size}px)");
    }
    // The launcher background layer is a flat colour.
    let mut flat = Pixmap::new(1024, 1024).expect("size");
    flat.fill(bg);
    std::fs::write("mobile/assets/icon-background.png", flat.encode_png().expect("png")).expect("write background");
}
