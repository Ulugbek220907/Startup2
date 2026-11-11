//! Ready-made starting designs. A template is just a function that returns
//! ordinary layers sized for a given print area, so the customer can edit
//! every part of it afterwards.

use crate::doc::*;
use crate::fonts::FontId;

pub struct TemplateDef {
    pub id: &'static str,
    build: fn(f32, f32, Color) -> Vec<Layer>,
}

pub const TEMPLATES: &[TemplateDef] = &[
    TemplateDef { id: "city_arch", build: city_arch },
    TemplateDef { id: "team_number", build: team_number },
    TemplateDef { id: "badge", build: badge },
    TemplateDef { id: "chest_small", build: chest_small },
];

pub fn find(id: &str) -> Option<&'static TemplateDef> {
    TEMPLATES.iter().find(|t| t.id == id)
}

impl TemplateDef {
    /// Layers for a print area of `w` x `h` cm. `ink` should contrast with
    /// the garment colour (white on dark garments, black on light ones).
    pub fn layers(&self, w: f32, h: f32, ink: Color) -> Vec<Layer> {
        let mut layers = (self.build)(w, h, ink);
        for (i, l) in layers.iter_mut().enumerate() {
            l.id = format!("{}-{}", self.id, i);
        }
        layers
    }
}

fn text(x: f32, y: f32, s: &str, font: FontId, size: f32, color: Color, curve: f32) -> Layer {
    Layer::new(
        "",
        x,
        y,
        LayerKind::Text(TextLayer { text: s.into(), font, size_cm: size, color, outline: None, curve, align: Align::Center }),
    )
}

fn city_arch(w: f32, h: f32, ink: Color) -> Vec<Layer> {
    let cx = w / 2.0;
    let size = w * 0.115;
    vec![
        text(cx, h * 0.2, "TOSHKENT", FontId::Montserrat, size, ink, 55.0),
        Layer::new("", cx, h * 0.34, LayerKind::Sticker(StickerLayer { sticker: "mountain".into(), width_cm: w * 0.3, color: ink })),
        text(cx, h * 0.47, "OʻZBEKISTON", FontId::Oswald, size * 0.6, ink, 0.0),
    ]
}

fn team_number(w: f32, h: f32, ink: Color) -> Vec<Layer> {
    let cx = w / 2.0;
    vec![
        text(cx, h * 0.14, "JAMOA", FontId::RubikMono, w * 0.11, ink, 0.0),
        text(cx, h * 0.42, "10", FontId::Oswald, w * 0.6, ink, 0.0),
    ]
}

fn badge(w: f32, h: f32, ink: Color) -> Vec<Layer> {
    let cx = w / 2.0;
    let d = (w * 0.7).min(h * 0.5);
    let cy = d / 2.0 + h * 0.04;
    vec![
        Layer::new(
            "",
            cx,
            cy,
            LayerKind::Shape(ShapeLayer {
                shape: ShapeKind::Ellipse,
                width_cm: d,
                height_cm: d,
                fill: None,
                stroke: Some(Outline { color: ink, width_cm: w * 0.012 }),
            }),
        ),
        Layer::new("", cx, cy - d * 0.16, LayerKind::Sticker(StickerLayer { sticker: "crown".into(), width_cm: d * 0.3, color: ink })),
        text(cx, cy + d * 0.14, "Mening\nuslubim", FontId::Pacifico, d * 0.13, ink, 0.0),
    ]
}

fn chest_small(w: f32, h: f32, ink: Color) -> Vec<Layer> {
    // Sits on the wearer's left chest, i.e. the right part of the area.
    vec![text(w * 0.74, h * 0.1, "salom", FontId::Caveat, w * 0.1, ink, 0.0)]
}

#[cfg(all(test, feature = "embed-fonts"))]
mod tests {
    use super::*;
    use crate::display::layer_bbox;
    use crate::fonts::EmbeddedFonts;
    use crate::geom::Rect;

    #[test]
    fn templates_fit_common_print_areas() {
        for (w, h) in [(30.0, 40.0), (30.0, 35.0), (28.0, 28.0)] {
            let area = Rect::new(0.0, 0.0, w, h);
            for t in TEMPLATES {
                let layers = t.layers(w, h, Color::WHITE);
                assert!(!layers.is_empty());
                for l in &layers {
                    let b = layer_bbox(l, &EmbeddedFonts);
                    let inside = b.intersect(&area);
                    assert!(
                        (inside.w * inside.h) / (b.w * b.h) > 0.999,
                        "template {} layer {} leaves the {w}x{h} area: {b:?}",
                        t.id,
                        l.id
                    );
                }
            }
        }
    }
}
