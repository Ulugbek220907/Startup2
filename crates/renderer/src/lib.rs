//! Deterministic server-side rendering.
//!
//! * [`render_print`]: transparent PNG at 300 DPI, exactly the physical size
//!   of the print area. This is the file that goes to the DTF printer.
//! * [`render_mockup`]: the garment artwork with the design applied, on a
//!   transparent background. Input to both try-on tiers and shown to admins.
//! * [`sheet::production_sheet`]: a one-page A4 PDF per order.
//!
//! Everything is pure CPU work on in-memory buffers: no I/O, no clocks, no
//! randomness, so the same input always produces the same bytes. Callers
//! run these inside `spawn_blocking`.

pub mod pdf;
pub mod sheet;

use std::collections::HashMap;
use std::sync::Arc;

use design::display::{self, DrawOp, FillRule, Paint, PathCmd};
use design::fonts::FontSource;
use design::garment_art::{self, ART_H, ART_W};
use design::{Affine, Color, DesignDoc, GarmentKind, Side};
use resvg::tiny_skia as sk;
use resvg::usvg;

pub use resvg::tiny_skia::Pixmap;

pub const PRINT_DPI: u32 = 300;
/// Hard ceiling on any output dimension, as a guard against bad config.
pub const MAX_DIM_PX: u32 = 12_000;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("output size {0}x{1} px is invalid")]
    BadSize(u32, u32),
    #[error("garment artwork failed to parse: {0}")]
    Art(String),
    #[error("encoding failed: {0}")]
    Encode(String),
}

/// Decoded uploaded images, keyed by asset id.
pub trait AssetSource {
    fn image(&self, asset_id: &str) -> Option<Arc<image::RgbaImage>>;
}

impl AssetSource for HashMap<String, Arc<image::RgbaImage>> {
    fn image(&self, asset_id: &str) -> Option<Arc<image::RgbaImage>> {
        self.get(asset_id).cloned()
    }
}

/// Physical print area and where it sits on the garment artwork.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaGeom {
    pub width_cm: f32,
    pub height_cm: f32,
    /// Fractions (0..1) of the artwork width/height.
    pub mock_x: f32,
    pub mock_y: f32,
    pub mock_w: f32,
    pub mock_h: f32,
}

pub fn cm_to_px(cm: f32, dpi: u32) -> u32 {
    (cm / 2.54 * dpi as f32).round() as u32
}

fn new_pixmap(w: u32, h: u32) -> Result<Pixmap, RenderError> {
    if w == 0 || h == 0 || w > MAX_DIM_PX || h > MAX_DIM_PX {
        return Err(RenderError::BadSize(w, h));
    }
    Pixmap::new(w, h).ok_or(RenderError::BadSize(w, h))
}

fn sk_color(c: Color) -> sk::Color {
    let [r, g, b, a] = c.0;
    sk::Color::from_rgba8(r, g, b, a)
}

fn sk_transform(t: &Affine) -> sk::Transform {
    sk::Transform::from_row(t.a, t.b, t.c, t.d, t.e, t.f)
}

fn build_path(cmds: &[PathCmd]) -> Option<sk::Path> {
    let mut pb = sk::PathBuilder::new();
    for c in cmds {
        match *c {
            PathCmd::M(x, y) => pb.move_to(x, y),
            PathCmd::L(x, y) => pb.line_to(x, y),
            PathCmd::Q(a, b, x, y) => pb.quad_to(a, b, x, y),
            PathCmd::C(a, b, c2, d, x, y) => pb.cubic_to(a, b, c2, d, x, y),
            PathCmd::Z => pb.close(),
        }
    }
    pb.finish()
}

/// Straight-alpha RGBA -> tiny-skia's premultiplied pixmap.
fn premultiplied(img: &image::RgbaImage) -> Option<Pixmap> {
    let mut pm = Pixmap::new(img.width(), img.height())?;
    for (dst, src) in pm.pixels_mut().iter_mut().zip(img.pixels()) {
        let [r, g, b, a] = src.0;
        // `premultiply` multiplies the colour channels by alpha, the format
        // compositing math expects.
        *dst = sk::ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    Some(pm)
}

/// High-quality downscale of an uploaded image before it is composited.
/// tiny-skia's bilinear sampling aliases badly when shrinking a lot, so
/// large sources are first reduced with a Lanczos filter.
fn downscale(img: &image::RgbaImage, max_w: u32, max_h: u32) -> Option<image::RgbaImage> {
    use fast_image_resize as fr;
    let (w, h) = (max_w.clamp(1, img.width()), max_h.clamp(1, img.height()));
    if w == img.width() && h == img.height() {
        return None;
    }
    let src = fr::images::Image::from_vec_u8(img.width(), img.height(), img.as_raw().clone(), fr::PixelType::U8x4).ok()?;
    let mut dst = fr::images::Image::new(w, h, fr::PixelType::U8x4);
    let mut resizer = fr::Resizer::new();
    // The default options handle alpha correctly (premultiply, resize,
    // un-premultiply) so transparent edges do not get dark fringes.
    let opts = fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::Lanczos3));
    resizer.resize(&src, &mut dst, &opts).ok()?;
    image::RgbaImage::from_raw(w, h, dst.into_vec())
}

/// Draw a display list. `base` maps print-area cm to output pixels.
fn draw_ops(pm: &mut Pixmap, ops: &[DrawOp], base: sk::Transform, assets: &dyn AssetSource) {
    for op in ops {
        match op {
            DrawOp::Path { path, paint, transform } => {
                let Some(p) = build_path(path) else { continue };
                let ts = sk_transform(transform).post_concat(base);
                let mut sp = sk::Paint { anti_alias: true, ..Default::default() };
                match *paint {
                    Paint::Fill(color, rule) => {
                        sp.set_color(sk_color(color));
                        let rule = match rule {
                            FillRule::NonZero => sk::FillRule::Winding,
                            FillRule::EvenOdd => sk::FillRule::EvenOdd,
                        };
                        pm.fill_path(&p, &sp, rule, ts, None);
                    }
                    Paint::Stroke { color, width } => {
                        sp.set_color(sk_color(color));
                        let stroke = sk::Stroke { width, line_cap: sk::LineCap::Round, line_join: sk::LineJoin::Round, ..Default::default() };
                        pm.stroke_path(&p, &sp, &stroke, ts, None);
                    }
                }
            }
            DrawOp::Image { asset, w, h, transform } => {
                let Some(img) = assets.image(asset) else { continue };
                if img.width() == 0 || img.height() == 0 {
                    continue;
                }
                let ts = sk_transform(transform).post_concat(base);
                // Approximate on-screen size, to decide how much to shrink.
                let sx = (ts.sx * ts.sx + ts.ky * ts.ky).sqrt();
                let sy = (ts.kx * ts.kx + ts.sy * ts.sy).sqrt();
                let (tw, th) = ((w * sx).ceil() as u32, (h * sy).ceil() as u32);
                let reduced = if tw * 3 < img.width() * 2 { downscale(&img, tw.max(1), th.max(1)) } else { None };
                let src = reduced.as_ref().unwrap_or(&img);
                let Some(src_pm) = premultiplied(src) else { continue };
                // Map source pixels onto the local rect (-w/2, -h/2, w, h).
                let local = sk::Transform::from_row(w / src.width() as f32, 0.0, 0.0, h / src.height() as f32, -w / 2.0, -h / 2.0);
                let paint = sk::PixmapPaint { quality: sk::FilterQuality::Bicubic, ..Default::default() };
                pm.draw_pixmap(0, 0, src_pm.as_ref(), &paint, local.post_concat(ts), None);
            }
        }
    }
}

/// Render one side of the design at `dpi`, transparent background, exactly
/// the size of the print area.
pub fn render_print(
    doc: &DesignDoc,
    side: Side,
    width_cm: f32,
    height_cm: f32,
    dpi: u32,
    fonts: &dyn FontSource,
    assets: &dyn AssetSource,
) -> Result<Pixmap, RenderError> {
    let (w, h) = (cm_to_px(width_cm, dpi), cm_to_px(height_cm, dpi));
    let mut pm = new_pixmap(w, h)?;
    let ops = display::side_ops(doc, side, fonts);
    // Scale from cm to pixels using the exact pixel size, so the design
    // spans the whole bitmap even after rounding the dimensions.
    let base = sk::Transform::from_scale(w as f32 / width_cm, h as f32 / height_cm);
    draw_ops(&mut pm, &ops, base, assets);
    Ok(pm)
}

/// Render the garment artwork alone (no design) `out_width` pixels wide.
pub fn render_garment(kind: GarmentKind, side: Side, color: Color, out_width: u32) -> Result<Pixmap, RenderError> {
    let out_h = (out_width as f32 * ART_H / ART_W).round() as u32;
    let mut pm = new_pixmap(out_width, out_h)?;
    let svg = garment_art::garment_svg(kind, side, color);
    let tree = usvg::Tree::from_str(&svg, &usvg::Options::default()).map_err(|e| RenderError::Art(e.to_string()))?;
    let k = out_width as f32 / ART_W;
    resvg::render(&tree, sk::Transform::from_scale(k, k), &mut pm.as_mut());
    Ok(pm)
}

/// Garment mockup with the design applied, transparent outside the garment.
pub fn render_mockup(
    doc: &DesignDoc,
    side: Side,
    kind: GarmentKind,
    color: Color,
    area: &AreaGeom,
    out_width: u32,
    fonts: &dyn FontSource,
    assets: &dyn AssetSource,
) -> Result<Pixmap, RenderError> {
    let mut pm = render_garment(kind, side, color, out_width)?;
    let ops = display::side_ops(doc, side, fonts);
    if ops.is_empty() {
        return Ok(pm);
    }
    // Render the design into its own area-sized layer so anything outside
    // the print area is clipped, exactly like on the print file.
    let (ax, ay) = (area.mock_x * pm.width() as f32, area.mock_y * pm.height() as f32);
    let (aw, ah) = ((area.mock_w * pm.width() as f32).round() as u32, (area.mock_h * pm.height() as f32).round() as u32);
    let mut layer = new_pixmap(aw.max(1), ah.max(1))?;
    let base = sk::Transform::from_scale(aw as f32 / area.width_cm, ah as f32 / area.height_cm);
    draw_ops(&mut layer, &ops, base, assets);
    pm.draw_pixmap(ax.round() as i32, ay.round() as i32, layer.as_ref(), &sk::PixmapPaint::default(), sk::Transform::identity(), None);
    Ok(pm)
}

/// Encode as PNG with straight alpha. `dpi` is written to the pHYs chunk so
/// RIP/print software opens the file at the right physical size.
pub fn encode_png(pm: &Pixmap, dpi: Option<u32>) -> Result<Vec<u8>, RenderError> {
    let mut rgba = Vec::with_capacity(pm.data().len());
    for p in pm.pixels() {
        let c = p.demultiply();
        rgba.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, pm.width(), pm.height());
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Balanced);
        if let Some(dpi) = dpi {
            // PNG stores resolution in pixels per metre.
            let ppm = (dpi as f64 / 0.0254).round() as u32;
            enc.set_pixel_dims(Some(png::PixelDimensions { xppu: ppm, yppu: ppm, unit: png::Unit::Meter }));
        }
        let mut writer = enc.write_header().map_err(|e| RenderError::Encode(e.to_string()))?;
        writer.write_image_data(&rgba).map_err(|e| RenderError::Encode(e.to_string()))?;
        writer.finish().map_err(|e| RenderError::Encode(e.to_string()))?;
    }
    Ok(out)
}

/// Size of the printed content of one side in cm (union box of all layers,
/// clipped to the area), for the production sheet.
pub fn printed_box_cm(doc: &DesignDoc, side: Side, width_cm: f32, height_cm: f32, fonts: &dyn FontSource) -> Option<design::Rect> {
    let clip = design::Rect::new(0.0, 0.0, width_cm, height_cm);
    let mut u = design::Rect::EMPTY;
    for l in doc.layers(side) {
        u = u.union(&display::layer_bbox(l, fonts).intersect(&clip));
    }
    (!u.is_empty()).then_some(u)
}
