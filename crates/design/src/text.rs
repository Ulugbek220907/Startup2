//! Text layout: string -> glyph outlines in centimetres.
//!
//! Deliberately simple and deterministic: one glyph per `char`, horizontal
//! advances from the font, no kerning or complex shaping. That is enough for
//! Latin and Cyrillic and guarantees the browser and the print file agree.

use ttf_parser::{Face, GlyphId, OutlineBuilder};

use crate::display::PathCmd;
use crate::doc::{Align, TextLayer};
use crate::fonts::FontSource;
use crate::geom::Rect;

/// Characters people type for the Uzbek modifier letter in oʻ / gʻ, in the
/// order we try them when the font lacks the one that was typed.
const APOSTROPHES: [char; 6] = ['\u{02BB}', '\u{2018}', '\u{02BC}', '\u{2019}', '\'', '`'];

/// Glyph for `c`, falling back across apostrophe variants so that
/// "Oʻzbekiston" renders in every bundled font, whichever apostrophe the
/// keyboard produced.
pub fn glyph_for(face: &Face, c: char) -> Option<GlyphId> {
    if let Some(g) = face.glyph_index(c) {
        return Some(g);
    }
    if APOSTROPHES.contains(&c) {
        return APOSTROPHES.iter().find_map(|a| face.glyph_index(*a));
    }
    None
}

/// Result of laying out a text layer, in local cm around the origin.
pub struct TextLayout {
    pub path: Vec<PathCmd>,
    pub bounds: Rect,
}

struct Sink<'a> {
    out: &'a mut Vec<PathCmd>,
    // Per-glyph transform: font units -> cm, flip y, rotate, translate.
    scale: f32,
    cos: f32,
    sin: f32,
    tx: f32,
    ty: f32,
    min: (f32, f32),
    max: (f32, f32),
}

impl Sink<'_> {
    fn map(&mut self, x: f32, y: f32) -> (f32, f32) {
        // Font y points up; our space points down.
        let (lx, ly) = (x * self.scale, -y * self.scale);
        let px = lx * self.cos - ly * self.sin + self.tx;
        let py = lx * self.sin + ly * self.cos + self.ty;
        self.min = (self.min.0.min(px), self.min.1.min(py));
        self.max = (self.max.0.max(px), self.max.1.max(py));
        (px, py)
    }
}

// `OutlineBuilder` is ttf-parser's visitor trait: the font calls these
// methods for every segment of a glyph outline.
impl OutlineBuilder for Sink<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.out.push(PathCmd::M(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.map(x, y);
        self.out.push(PathCmd::L(x, y));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.map(x1, y1);
        let (x, y) = self.map(x, y);
        self.out.push(PathCmd::Q(x1, y1, x, y));
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.map(x1, y1);
        let (x2, y2) = self.map(x2, y2);
        let (x, y) = self.map(x, y);
        self.out.push(PathCmd::C(x1, y1, x2, y2, x, y));
    }
    fn close(&mut self) {
        self.out.push(PathCmd::Z);
    }
}

/// Lay out a text layer. Returns `None` when the font is not available.
pub fn layout(t: &TextLayer, fonts: &dyn FontSource) -> Option<TextLayout> {
    let data = fonts.font_data(t.font)?;
    let face = Face::parse(data, 0).ok()?;
    let upem = face.units_per_em() as f32;
    let scale = t.size_cm / upem;
    let ascent = face.ascender() as f32 * scale;
    let descent = -(face.descender() as f32) * scale;
    let line_height = (ascent + descent + face.line_gap() as f32 * scale).max(t.size_cm);

    // Measure each line first: (glyph, x offset from line start, advance).
    struct Line {
        glyphs: Vec<(GlyphId, f32, f32)>,
        width: f32,
    }
    let lines: Vec<Line> = t
        .text
        .split('\n')
        .map(|line| {
            let mut x = 0.0f32;
            let mut glyphs = Vec::new();
            for c in line.chars() {
                // Unknown characters are skipped rather than drawn as tofu
                // boxes, which would end up on the printed garment.
                let Some(g) = glyph_for(&face, c) else { continue };
                let adv = face.glyph_hor_advance(g).unwrap_or(0) as f32 * scale;
                glyphs.push((g, x, adv));
                x += adv;
            }
            Line { glyphs, width: x }
        })
        .collect();

    let block_w = lines.iter().map(|l| l.width).fold(0.0f32, f32::max);
    let block_h = line_height * lines.len() as f32;
    if block_w <= 0.0 {
        return Some(TextLayout { path: vec![], bounds: Rect::EMPTY });
    }

    // Curve: the widest line spans `theta` radians of a circle.
    let theta = (t.curve.clamp(-100.0, 100.0) / 100.0) * std::f32::consts::PI;
    let radius = if theta.abs() > 1e-3 { Some(block_w / theta.abs()) } else { None };
    let up = theta > 0.0;

    let mut path = Vec::new();
    let mut sink = Sink {
        out: &mut path,
        scale,
        cos: 1.0,
        sin: 0.0,
        tx: 0.0,
        ty: 0.0,
        min: (f32::MAX, f32::MAX),
        max: (f32::MIN, f32::MIN),
    };

    for (i, line) in lines.iter().enumerate() {
        // Baseline y of this line, block vertically centred on the origin.
        let baseline = -block_h / 2.0 + i as f32 * line_height + ascent + (line_height - ascent - descent) / 2.0;
        let start_x = match t.align {
            Align::Left => -block_w / 2.0,
            Align::Center => -line.width / 2.0,
            Align::Right => block_w / 2.0 - line.width,
        };
        for &(g, gx, adv) in &line.glyphs {
            match radius {
                None => {
                    sink.cos = 1.0;
                    sink.sin = 0.0;
                    sink.tx = start_x + gx;
                    sink.ty = baseline;
                }
                Some(r) => {
                    // Place the glyph's centre on the arc and rotate it to
                    // follow the tangent. All lines share the same angles so
                    // multi-line text stays aligned like a fan.
                    let centre = start_x + gx + adv / 2.0;
                    let phi = centre / r;
                    let (s, c) = phi.sin_cos();
                    if up {
                        // Circle centre below the text at (0, r).
                        let rr = r - baseline;
                        sink.cos = c;
                        sink.sin = s;
                        let (cx, cy) = (rr * s, r - rr * c);
                        sink.tx = cx - (adv / 2.0) * c;
                        sink.ty = cy - (adv / 2.0) * s;
                    } else {
                        // Circle centre above the text at (0, -r).
                        let rr = r + baseline;
                        sink.cos = c;
                        sink.sin = -s;
                        let (cx, cy) = (rr * s, -r + rr * c);
                        sink.tx = cx - (adv / 2.0) * c;
                        sink.ty = cy + (adv / 2.0) * s;
                    }
                }
            }
            face.outline_glyph(g, &mut sink);
        }
    }

    let bounds = if sink.min.0 > sink.max.0 {
        Rect::EMPTY
    } else {
        Rect::new(sink.min.0, sink.min.1, sink.max.0 - sink.min.0, sink.max.1 - sink.min.1)
    };
    Some(TextLayout { path, bounds })
}

#[cfg(all(test, feature = "embed-fonts"))]
mod tests {
    use super::*;
    use crate::doc::Color;
    use crate::fonts::{EmbeddedFonts, FontId};

    fn layer(text: &str, font: FontId, curve: f32) -> TextLayer {
        TextLayer { text: text.into(), font, size_cm: 2.0, color: Color::BLACK, outline: None, curve, align: Align::Center }
    }

    #[test]
    fn every_font_renders_uzbek_and_cyrillic() {
        // U+02BB, the ASCII apostrophe and the curly quote must all produce
        // the same number of contours-bearing glyphs in every font.
        for font in FontId::ALL {
            let face = Face::parse(EmbeddedFonts.font_data(font).unwrap(), 0).unwrap();
            for c in "Oʻzbekiston gʻoya Ўзбекистон қғ Привет Hello 0123456789".chars().filter(|c| !c.is_whitespace()) {
                assert!(glyph_for(&face, c).is_some(), "{font:?} lacks {c:?}");
            }
            // Rubik Mono One has no Cyrillic "ҳ"; every other font does.
            if font != FontId::RubikMono {
                assert!(glyph_for(&face, 'ҳ').is_some(), "{font:?} lacks ҳ");
            }
            for a in APOSTROPHES {
                assert!(glyph_for(&face, a).is_some(), "{font:?} has no apostrophe fallback for {a:?}");
            }
            let l = layout(&layer("Oʻzbekiston", font, 0.0), &EmbeddedFonts).unwrap();
            assert!(!l.path.is_empty());
            assert!(l.bounds.w > 4.0 && l.bounds.h > 1.0, "{font:?} bounds {:?}", l.bounds);
        }
    }

    #[test]
    fn straight_text_is_centred() {
        let l = layout(&layer("HELLO", FontId::Montserrat, 0.0), &EmbeddedFonts).unwrap();
        let (cx, _) = l.bounds.center();
        assert!(cx.abs() < 0.3, "centre x = {cx}");
    }

    #[test]
    fn curved_text_bends_the_right_way() {
        let flat = layout(&layer("TOSHKENT CITY", FontId::Montserrat, 0.0), &EmbeddedFonts).unwrap();
        let up = layout(&layer("TOSHKENT CITY", FontId::Montserrat, 60.0), &EmbeddedFonts).unwrap();
        let down = layout(&layer("TOSHKENT CITY", FontId::Montserrat, -60.0), &EmbeddedFonts).unwrap();
        // Bending makes the block taller and narrower.
        assert!(up.bounds.h > flat.bounds.h * 1.5);
        assert!(up.bounds.w < flat.bounds.w);
        // Arch up: ends hang lower than the middle, so the box extends
        // further below the origin than above it; arch down is the mirror.
        assert!(up.bounds.bottom() > -up.bounds.y);
        assert!(-down.bounds.y > down.bounds.bottom());
    }

    #[test]
    fn multi_line_grows_downwards() {
        let one = layout(&layer("A", FontId::Oswald, 0.0), &EmbeddedFonts).unwrap();
        let two = layout(&layer("A\nA", FontId::Oswald, 0.0), &EmbeddedFonts).unwrap();
        assert!(two.bounds.h > one.bounds.h * 1.8);
    }

    #[test]
    fn missing_font_returns_none() {
        assert!(layout(&layer("x", FontId::Caveat, 0.0), &crate::fonts::NoFonts).is_none());
    }
}
