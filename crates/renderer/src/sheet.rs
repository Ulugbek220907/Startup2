//! Production sheet: one A4 page per order for the print workshop.
//!
//! Shows the order code, what to print on which garment, both mockups, the
//! physical print sizes, notes and a QR code that opens the order in the
//! admin page. Customer contact details are deliberately left off: the sheet
//! lies around the workshop, and the QR leads to them when needed.

use design::fonts::{FontId, FontSource};
use design::text;
use design::{Align, Color, TextLayer};
use domain::i18n::{t, tf, Lang};
use resvg::tiny_skia as sk;

use crate::pdf;
use crate::{Pixmap, RenderError};

/// 150 DPI is plenty for a sheet read at arm's length and keeps the PDF
/// around 100-200 KB.
const SHEET_DPI: f32 = 150.0;
const PAGE_W: u32 = 1240; // 210 mm
const PAGE_H: u32 = 1754; // 297 mm
const MARGIN: f32 = 70.0;

pub struct SheetSide {
    pub label: String,
    pub area_w_cm: f32,
    pub area_h_cm: f32,
    /// Size of the printed content, `None` when the side is empty.
    pub printed_cm: Option<(f32, f32)>,
    pub mockup: Option<Pixmap>,
}

pub struct SheetInfo {
    pub brand: String,
    pub order_code: String,
    pub created_at: String,
    pub garment: String,
    pub color_name: String,
    pub color: Color,
    pub size: String,
    pub quantity: u32,
    pub finish: String,
    pub delivery: String,
    pub notes: Option<String>,
    pub admin_url: String,
    pub sides: Vec<SheetSide>,
    pub lang: Lang,
}

const INK: Color = Color::rgb(17, 17, 17);
const GREY: Color = Color::rgb(110, 110, 110);

struct Canvas<'a> {
    pm: Pixmap,
    fonts: &'a dyn FontSource,
}

impl Canvas<'_> {
    fn text_layer(s: &str, size: f32, color: Color) -> TextLayer {
        TextLayer { text: s.to_string(), font: FontId::Montserrat, size_cm: size, color, outline: None, curve: 0.0, align: Align::Left }
    }

    fn measure(&self, s: &str, size: f32) -> f32 {
        text::layout(&Self::text_layer(s, size, INK), self.fonts).map(|l| l.bounds.w).unwrap_or(0.0)
    }

    /// Draw text whose left edge is at `x` and whose vertical centre is `y`.
    /// Returns the drawn width.
    fn text(&mut self, x: f32, y: f32, s: &str, size: f32, color: Color) -> f32 {
        let Some(lay) = text::layout(&Self::text_layer(s, size, color), self.fonts) else { return 0.0 };
        if lay.path.is_empty() {
            return 0.0;
        }
        let mut pb = sk::PathBuilder::new();
        for c in &lay.path {
            use design::display::PathCmd::*;
            match *c {
                M(a, b) => pb.move_to(a, b),
                L(a, b) => pb.line_to(a, b),
                Q(a, b, c2, d) => pb.quad_to(a, b, c2, d),
                C(a, b, c2, d, e, f) => pb.cubic_to(a, b, c2, d, e, f),
                Z => pb.close(),
            }
        }
        if let Some(path) = pb.finish() {
            let mut paint = sk::Paint { anti_alias: true, ..Default::default() };
            let [r, g, b, a] = color.0;
            paint.set_color_rgba8(r, g, b, a);
            let ts = sk::Transform::from_translate(x - lay.bounds.x, y);
            self.pm.fill_path(&path, &paint, sk::FillRule::Winding, ts, None);
        }
        lay.bounds.w
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        if let Some(r) = sk::Rect::from_xywh(x, y, w, h) {
            let mut paint = sk::Paint::default();
            let [cr, cg, cb, ca] = color.0;
            paint.set_color_rgba8(cr, cg, cb, ca);
            self.pm.fill_rect(r, &paint, sk::Transform::identity(), None);
        }
    }

    /// Greedy word wrap to `max_w` pixels.
    fn wrap(&self, s: &str, size: f32, max_w: f32, max_lines: usize) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        for para in s.lines() {
            let mut cur = String::new();
            for word in para.split_whitespace() {
                let candidate = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
                if cur.is_empty() || self.measure(&candidate, size) <= max_w {
                    cur = candidate;
                } else {
                    lines.push(std::mem::take(&mut cur));
                    cur = word.to_string();
                }
            }
            if !cur.is_empty() {
                lines.push(cur);
            }
        }
        if lines.len() > max_lines {
            lines.truncate(max_lines);
            if let Some(last) = lines.last_mut() {
                last.push('…');
            }
        }
        lines
    }

    fn qr(&mut self, x: f32, y: f32, size: f32, data: &str) {
        // Medium error correction survives a bit of ink or a fold.
        let Ok(code) = qrcode::QrCode::with_error_correction_level(data.as_bytes(), qrcode::EcLevel::M) else { return };
        let n = code.width();
        let quiet = 2.0;
        let cell = size / (n as f32 + quiet * 2.0);
        self.rect(x, y, size, size, Color::WHITE);
        for (i, c) in code.to_colors().into_iter().enumerate() {
            if c == qrcode::Color::Dark {
                let (cx, cy) = ((i % n) as f32, (i / n) as f32);
                // Snap to whole pixels so modules have crisp edges.
                let x0 = (x + (cx + quiet) * cell).round();
                let y0 = (y + (cy + quiet) * cell).round();
                let x1 = (x + (cx + quiet + 1.0) * cell).round();
                let y1 = (y + (cy + quiet + 1.0) * cell).round();
                self.rect(x0, y0, x1 - x0, y1 - y0, INK);
            }
        }
    }
}

fn fmt_cm(v: f32) -> String {
    let r = (v * 10.0).round() / 10.0;
    if (r - r.round()).abs() < 0.05 {
        format!("{}", r.round() as i64)
    } else {
        format!("{r:.1}")
    }
}

/// Render the sheet as a bitmap (exposed for golden tests).
pub fn render_sheet(info: &SheetInfo, fonts: &dyn FontSource) -> Result<Pixmap, RenderError> {
    let mut pm = Pixmap::new(PAGE_W, PAGE_H).ok_or(RenderError::BadSize(PAGE_W, PAGE_H))?;
    pm.fill(sk::Color::WHITE);
    let mut c = Canvas { pm, fonts };
    let l = info.lang;
    let right = PAGE_W as f32 - MARGIN;

    // Header.
    c.text(MARGIN, 100.0, &info.brand, 40.0, INK);
    let title = t(l, "sheet.title");
    let tw = c.measure(title, 26.0);
    c.text(right - tw, 100.0, title, 26.0, GREY);
    c.rect(MARGIN, 140.0, right - MARGIN, 4.0, INK);

    // Order code and QR.
    let qr_size = 270.0;
    c.qr(right - qr_size, 170.0, qr_size, &info.admin_url);
    let scan = t(l, "sheet.scan");
    for (i, line) in c.wrap(scan, 18.0, qr_size, 2).iter().enumerate() {
        c.text(right - qr_size, 170.0 + qr_size + 22.0 + i as f32 * 26.0, line, 18.0, GREY);
    }
    c.text(MARGIN, 230.0, &info.order_code, 92.0, INK);
    c.text(MARGIN, 318.0, &format!("{}: {}", t(l, "sheet.date"), info.created_at), 24.0, GREY);

    // Facts table: label on the left, value large on the right.
    let label_w = 250.0;
    let mut y = 395.0;
    let quantity = format!("{} {}", info.quantity, t(l, "common.pcs"));
    let rows: [(&str, &str, f32); 6] = [
        (t(l, "sheet.garment"), info.garment.as_str(), 34.0),
        (t(l, "common.color"), info.color_name.as_str(), 34.0),
        (t(l, "common.size"), info.size.as_str(), 54.0),
        (t(l, "common.quantity"), quantity.as_str(), 54.0),
        (t(l, "sheet.finish"), info.finish.as_str(), 30.0),
        (t(l, "sheet.delivery"), info.delivery.as_str(), 30.0),
    ];
    for (i, (label, value, size)) in rows.iter().enumerate() {
        let row_h = size * 1.15 + 22.0;
        let cy = y + row_h / 2.0;
        c.text(MARGIN, cy, label, 22.0, GREY);
        let mut vx = MARGIN + label_w;
        if i == 1 {
            // Colour swatch with a border so white is visible on paper.
            c.rect(vx, cy - 20.0, 40.0, 40.0, INK);
            c.rect(vx + 2.0, cy - 18.0, 36.0, 36.0, info.color);
            vx += 56.0;
        }
        c.text(vx, cy, value, *size, INK);
        y += row_h;
        c.rect(MARGIN, y, PAGE_W as f32 - 2.0 * MARGIN - qr_size - 30.0, 1.0, Color::rgb(210, 210, 210));
    }

    // Mockups, side by side.
    let top = y.max(170.0 + qr_size + 90.0) + 30.0;
    let n = info.sides.len().max(1) as f32;
    let gap = 30.0;
    let cell_w = ((right - MARGIN) - gap * (n - 1.0)) / n;
    let notes_h = if info.notes.as_deref().map(str::trim).unwrap_or("").is_empty() { 0.0 } else { 190.0 };
    let cell_h = PAGE_H as f32 - top - MARGIN - notes_h - 110.0;
    for (i, side) in info.sides.iter().enumerate() {
        let x = MARGIN + i as f32 * (cell_w + gap);
        c.rect(x, top, cell_w, cell_h, Color::rgb(236, 236, 236));
        if let Some(m) = &side.mockup {
            let k = (cell_w / m.width() as f32).min(cell_h / m.height() as f32) * 0.94;
            let (dw, dh) = (m.width() as f32 * k, m.height() as f32 * k);
            let ts = sk::Transform::from_row(k, 0.0, 0.0, k, x + (cell_w - dw) / 2.0, top + (cell_h - dh) / 2.0);
            let paint = sk::PixmapPaint { quality: sk::FilterQuality::Bilinear, ..Default::default() };
            c.pm.draw_pixmap(0, 0, m.as_ref(), &paint, ts, None);
        }
        let cy = top + cell_h + 34.0;
        let used = c.text(x, cy, &side.label, 30.0, INK);
        let detail = match side.printed_cm {
            Some((w, h)) => format!("{}: {} × {} {}", t(l, "sheet.print_size"), fmt_cm(w), fmt_cm(h), t(l, "common.cm")),
            None => t(l, "sheet.not_printed").to_string(),
        };
        c.text(x + used + 20.0, cy, &detail, 24.0, if side.printed_cm.is_some() { INK } else { GREY });
        let area = tf(l, "editor.print_area", &[("w", &fmt_cm(side.area_w_cm)), ("h", &fmt_cm(side.area_h_cm))]);
        c.text(x, cy + 40.0, &area, 20.0, GREY);
    }

    // Notes.
    if notes_h > 0.0 {
        let ny = PAGE_H as f32 - MARGIN - notes_h + 30.0;
        c.rect(MARGIN, ny - 24.0, right - MARGIN, 2.0, INK);
        c.text(MARGIN, ny + 6.0, t(l, "order.notes"), 22.0, GREY);
        let lines = c.wrap(info.notes.as_deref().unwrap_or(""), 26.0, right - MARGIN, 4);
        for (i, line) in lines.iter().enumerate() {
            c.text(MARGIN, ny + 48.0 + i as f32 * 34.0, line, 26.0, INK);
        }
    }
    Ok(c.pm)
}

/// The production sheet as a ready-to-print A4 PDF.
pub fn production_sheet(info: &SheetInfo, fonts: &dyn FontSource) -> Result<Vec<u8>, RenderError> {
    let pm = render_sheet(info, fonts)?;
    // The page is opaque white, so dropping alpha is lossless.
    let mut rgb = Vec::with_capacity((pm.width() * pm.height() * 3) as usize);
    for p in pm.pixels() {
        rgb.extend_from_slice(&[p.red(), p.green(), p.blue()]);
    }
    debug_assert!((PAGE_W as f32 / SHEET_DPI * 72.0 - pdf::A4_W_PT).abs() < 1.0);
    Ok(pdf::single_image_pdf(&rgb, pm.width(), pm.height(), pdf::A4_W_PT, pdf::A4_H_PT))
}
