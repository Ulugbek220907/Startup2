//! Drawing garments and designs on an HTML canvas.
//!
//! The display list comes from the shared `design` crate, so this replays
//! the same paths the server rasterises for the print file.

use std::cell::RefCell;
use std::collections::HashMap;

use design::display::{DrawOp, FillRule, Paint, PathCmd};
use design::garment_art::{self, ART_H, ART_W};
use design::{Affine, Color, GarmentKind, Layer, Side};
use domain::api::PrintAreaDto;
use leptos::prelude::Update;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, CanvasWindingRule, HtmlCanvasElement, HtmlImageElement};

use crate::state::{ctx, Fonts};

thread_local! {
    // Decoded images by URL. Entries exist from the moment loading starts;
    // `complete()` tells whether they can be drawn yet.
    static IMAGES: RefCell<HashMap<String, HtmlImageElement>> = RefCell::new(HashMap::new());
}

/// Get an image by URL, starting to load it on first use. Returns `None`
/// until it has loaded; a redraw is triggered when it does.
pub fn image(url: &str) -> Option<HtmlImageElement> {
    let existing = IMAGES.with(|m| m.borrow().get(url).cloned());
    let img = match existing {
        Some(img) => img,
        None => {
            let img = HtmlImageElement::new().ok()?;
            // Needed to read pixels back (snapshots, WebGL textures) when the
            // API is on another origin, as in the mobile apps.
            img.set_cross_origin(Some("anonymous"));
            let onload = Closure::<dyn FnMut()>::new(move || ctx().redraw.update(|n| *n += 1));
            img.set_onload(Some(onload.as_ref().unchecked_ref()));
            onload.forget();
            img.set_src(url);
            IMAGES.with(|m| m.borrow_mut().insert(url.to_string(), img.clone()));
            img
        }
    };
    (img.complete() && img.natural_width() > 0).then_some(img)
}

/// `data:` URL of the garment artwork in a colour.
pub fn garment_url(kind: GarmentKind, side: Side, color: Color) -> String {
    let svg = garment_art::garment_svg(kind, side, color);
    format!("data:image/svg+xml;charset=utf-8,{}", String::from(js_sys::encode_uri_component(&svg)))
}

pub fn asset_preview_url(asset_id: &str) -> String {
    crate::api::url(&format!("/api/assets/{asset_id}/preview"))
}

/// The part of the 1000x1100 artwork shown on a canvas.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl View {
    pub const FULL: View = View { x: 0.0, y: 0.0, w: ART_W, h: ART_H };

    /// Zoomed on a print area with some garment visible around it, so the
    /// design is large enough to edit on a phone.
    pub fn around(area: &PrintAreaDto) -> View {
        let (ax, ay, aw, ah) = area_rect(area);
        let margin = aw * 0.14;
        View { x: ax - margin, y: ay - margin, w: aw + 2.0 * margin, h: ah + 2.0 * margin }
    }

    pub fn aspect(&self) -> f32 {
        self.h / self.w
    }
}

/// Print area rectangle in artwork pixels.
pub fn area_rect(area: &PrintAreaDto) -> (f32, f32, f32, f32) {
    (area.mock_x * ART_W, area.mock_y * ART_H, area.mock_w * ART_W, area.mock_h * ART_H)
}

/// Transform from print-area centimetres to canvas pixels.
pub fn cm_to_canvas(area: &PrintAreaDto, view: &View, canvas_w: f32) -> Affine {
    let k = canvas_w / view.w;
    let (ax, ay, aw, ah) = area_rect(area);
    Affine::scale(aw / area.width_cm * k, ah / area.height_cm * k).then(Affine::translate((ax - view.x) * k, (ay - view.y) * k))
}

/// Size a canvas for crisp output: backing store in device pixels, capped
/// at 2x to protect low-end phones. Returns the CSS-pixel -> backing scale.
pub fn fit_canvas(canvas: &HtmlCanvasElement, css_w: f32, css_h: f32) -> f32 {
    let dpr = web_sys::window().map(|w| w.device_pixel_ratio() as f32).unwrap_or(1.0).clamp(1.0, 2.0);
    let (w, h) = ((css_w * dpr).round() as u32, (css_h * dpr).round() as u32);
    if canvas.width() != w || canvas.height() != h {
        canvas.set_width(w);
        canvas.set_height(h);
    }
    dpr
}

pub fn context_2d(canvas: &HtmlCanvasElement) -> Option<CanvasRenderingContext2d> {
    canvas.get_context("2d").ok()??.dyn_into().ok()
}

fn set_transform(c: &CanvasRenderingContext2d, t: &Affine) {
    let _ = c.set_transform(t.a as f64, t.b as f64, t.c as f64, t.d as f64, t.e as f64, t.f as f64);
}

fn trace(c: &CanvasRenderingContext2d, path: &[PathCmd]) {
    c.begin_path();
    for cmd in path {
        match *cmd {
            PathCmd::M(x, y) => c.move_to(x as f64, y as f64),
            PathCmd::L(x, y) => c.line_to(x as f64, y as f64),
            PathCmd::Q(a, b, x, y) => c.quadratic_curve_to(a as f64, b as f64, x as f64, y as f64),
            PathCmd::C(a, b, d, e, x, y) => c.bezier_curve_to(a as f64, b as f64, d as f64, e as f64, x as f64, y as f64),
            PathCmd::Z => c.close_path(),
        }
    }
}

/// Replay a display list. `base` maps print-area cm to backing-store pixels.
pub fn replay(c: &CanvasRenderingContext2d, ops: &[DrawOp], base: &Affine) {
    for op in ops {
        match op {
            DrawOp::Path { path, paint, transform } => {
                set_transform(c, &transform.then(*base));
                trace(c, path);
                match *paint {
                    Paint::Fill(color, rule) => {
                        c.set_fill_style_str(&color.to_css());
                        c.fill_with_canvas_winding_rule(match rule {
                            FillRule::NonZero => CanvasWindingRule::Nonzero,
                            FillRule::EvenOdd => CanvasWindingRule::Evenodd,
                        });
                    }
                    Paint::Stroke { color, width } => {
                        c.set_stroke_style_str(&color.to_css());
                        c.set_line_width(width as f64);
                        c.set_line_cap("round");
                        c.set_line_join("round");
                        c.stroke();
                    }
                }
            }
            DrawOp::Image { asset, w, h, transform } => {
                if let Some(img) = image(&asset_preview_url(asset)) {
                    set_transform(c, &transform.then(*base));
                    let _ = c.draw_image_with_html_image_element_and_dw_and_dh(&img, -*w as f64 / 2.0, -*h as f64 / 2.0, *w as f64, *h as f64);
                }
            }
        }
    }
}

pub struct MockupOpts {
    /// Draw the dashed print-area outline (editor only).
    pub show_area: bool,
    /// Fill behind the garment; `None` leaves the canvas transparent.
    pub background: Option<&'static str>,
}

/// Draw garment + design. `scale` is the backing-store scale from
/// [`fit_canvas`], `css_w` the canvas width in CSS pixels.
#[allow(clippy::too_many_arguments)]
pub fn draw_mockup(
    c: &CanvasRenderingContext2d,
    css_w: f32,
    scale: f32,
    view: &View,
    kind: GarmentKind,
    side: Side,
    color: Color,
    area: Option<&PrintAreaDto>,
    layers: &[Layer],
    opts: &MockupOpts,
) {
    let w = css_w * scale;
    let h = w * view.aspect();
    let _ = c.reset_transform();
    match opts.background {
        Some(bg) => {
            c.set_fill_style_str(bg);
            c.fill_rect(0.0, 0.0, w as f64, h as f64);
        }
        None => c.clear_rect(0.0, 0.0, w as f64, h as f64),
    }
    let k = w / view.w;
    if let Some(img) = image(&garment_url(kind, side, color)) {
        let _ = c.draw_image_with_html_image_element_and_dw_and_dh(&img, (-view.x * k) as f64, (-view.y * k) as f64, (ART_W * k) as f64, (ART_H * k) as f64);
    }
    let Some(area) = area else { return };
    let base = cm_to_canvas(area, view, w);
    let (x0, y0) = base.apply(0.0, 0.0);
    let (x1, y1) = base.apply(area.width_cm, area.height_cm);

    // Clip to the print area exactly like the print file is.
    c.save();
    c.begin_path();
    c.rect(x0 as f64, y0 as f64, (x1 - x0) as f64, (y1 - y0) as f64);
    c.clip();
    let ops: Vec<DrawOp> = layers.iter().flat_map(|l| design::display::layer_ops(l, &Fonts)).collect();
    replay(c, &ops, &base);
    c.restore();
    let _ = c.reset_transform();

    if opts.show_area {
        // Dashes that read on both light and dark garments.
        let dash = js_sys::Array::of2(&JsValue::from_f64(8.0 * scale as f64), &JsValue::from_f64(6.0 * scale as f64));
        let _ = c.set_line_dash(&dash);
        c.set_line_width(1.5 * scale as f64);
        c.set_stroke_style_str(if color.luma() < 110 { "rgba(255,255,255,0.55)" } else { "rgba(0,0,0,0.45)" });
        c.stroke_rect(x0 as f64, y0 as f64, (x1 - x0) as f64, (y1 - y0) as f64);
        let _ = c.set_line_dash(&js_sys::Array::new());
    }
}

/// Paint a whole-garment mockup onto a canvas element sized `css_w` wide.
pub fn paint_full(canvas: &HtmlCanvasElement, css_w: f32, kind: GarmentKind, side: Side, color: Color, area: Option<&PrintAreaDto>, layers: &[Layer]) {
    let scale = fit_canvas(canvas, css_w, css_w * View::FULL.aspect());
    if let Some(c) = context_2d(canvas) {
        draw_mockup(&c, css_w, scale, &View::FULL, kind, side, color, area, layers, &MockupOpts { show_area: false, background: None });
    }
}
