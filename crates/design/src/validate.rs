//! Server-side validation of untrusted design documents.
//!
//! The editor enforces the same limits for a good experience, but the server
//! must never trust a client: a hostile document could otherwise make the
//! renderer allocate or loop excessively.

use crate::doc::*;
use crate::stickers;

pub const MAX_LAYERS_PER_SIDE: usize = 40;
pub const MAX_TEXT_CHARS: usize = 200;
pub const MAX_TEXT_LINES: usize = 8;
pub const MAX_POINTS_PER_SIDE: usize = 20_000;
pub const MAX_SLUG_LEN: usize = 64;
/// Sizes are bounded generously; the renderer clips to the print area.
pub const MIN_SIZE_CM: f32 = 0.05;
pub const MAX_SIZE_CM: f32 = 200.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError(pub String);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ValidationError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ValidationError> {
    Err(ValidationError(msg.into()))
}

fn finite(v: f32, what: &str) -> Result<(), ValidationError> {
    if v.is_finite() {
        Ok(())
    } else {
        err(format!("{what} is not a finite number"))
    }
}

fn size(v: f32, what: &str) -> Result<(), ValidationError> {
    finite(v, what)?;
    if (MIN_SIZE_CM..=MAX_SIZE_CM).contains(&v) {
        Ok(())
    } else {
        err(format!("{what} is out of range"))
    }
}

fn slug(s: &str, what: &str) -> Result<(), ValidationError> {
    let ok = !s.is_empty()
        && s.len() <= MAX_SLUG_LEN
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        err(format!("{what} is not a valid identifier"))
    }
}

/// Validate a document. `allow_foil` is false in v1: foil finishes exist in
/// the schema but cannot be ordered yet.
pub fn validate(doc: &DesignDoc, allow_foil: bool) -> Result<(), ValidationError> {
    if doc.version != SCHEMA_VERSION {
        return err("unsupported design version");
    }
    slug(&doc.garment, "garment")?;
    slug(&doc.color, "color")?;
    for side in Side::ALL {
        let layers = doc.layers(side);
        if layers.len() > MAX_LAYERS_PER_SIDE {
            return err(format!("too many layers on the {} side", side.as_str()));
        }
        let mut points = 0usize;
        for l in layers {
            validate_layer(l, allow_foil, &mut points)?;
        }
        if points > MAX_POINTS_PER_SIDE {
            return err("drawing is too detailed");
        }
    }
    Ok(())
}

fn validate_layer(l: &Layer, allow_foil: bool, points: &mut usize) -> Result<(), ValidationError> {
    if l.id.is_empty() || l.id.len() > MAX_SLUG_LEN {
        return err("layer id is missing or too long");
    }
    finite(l.x, "layer x")?;
    finite(l.y, "layer y")?;
    finite(l.rotation, "layer rotation")?;
    if l.x.abs() > 1000.0 || l.y.abs() > 1000.0 {
        return err("layer position is out of range");
    }
    finite(l.scale, "layer scale")?;
    if !(0.01..=100.0).contains(&l.scale) {
        return err("layer scale is out of range");
    }
    if l.finish != Finish::Standard && !allow_foil {
        return err("foil finishes are not available yet");
    }
    match &l.kind {
        LayerKind::Text(t) => {
            if t.text.chars().count() > MAX_TEXT_CHARS {
                return err("text is too long");
            }
            if t.text.split('\n').count() > MAX_TEXT_LINES {
                return err("text has too many lines");
            }
            if t.text.chars().any(|c| c.is_control() && c != '\n') {
                return err("text contains control characters");
            }
            size(t.size_cm, "text size")?;
            finite(t.curve, "text curve")?;
            if !(-100.0..=100.0).contains(&t.curve) {
                return err("text curve is out of range");
            }
            if let Some(o) = t.outline {
                finite(o.width_cm, "outline width")?;
                if !(0.0..=5.0).contains(&o.width_cm) {
                    return err("outline width is out of range");
                }
            }
        }
        LayerKind::Image(i) => {
            // Asset ids are UUIDs issued by the upload endpoint.
            if i.asset.len() != 36 || !i.asset.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
                return err("image asset id is invalid");
            }
            size(i.width_cm, "image width")?;
            size(i.height_cm, "image height")?;
            if i.px_width == 0 || i.px_height == 0 {
                return err("image pixel size is missing");
            }
        }
        LayerKind::Draw(d) => {
            if d.strokes.is_empty() {
                return err("drawing has no strokes");
            }
            for s in &d.strokes {
                size(s.width_cm, "stroke width")?;
                *points += s.points.len();
                for p in &s.points {
                    finite(p[0], "stroke point")?;
                    finite(p[1], "stroke point")?;
                    if p[0].abs() > 1000.0 || p[1].abs() > 1000.0 {
                        return err("stroke point is out of range");
                    }
                }
            }
        }
        LayerKind::Shape(s) => {
            size(s.width_cm, "shape width")?;
            size(s.height_cm, "shape height")?;
            if s.fill.is_none() && s.stroke.is_none() {
                return err("shape has neither fill nor stroke");
            }
            if let Some(o) = s.stroke {
                finite(o.width_cm, "shape stroke width")?;
                if !(0.0..=5.0).contains(&o.width_cm) {
                    return err("shape stroke width is out of range");
                }
            }
        }
        LayerKind::Sticker(s) => {
            if stickers::find(&s.sticker).is_none() {
                return err("unknown sticker");
            }
            size(s.width_cm, "sticker width")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::FontId;

    fn text_doc(text: &str) -> DesignDoc {
        let mut d = DesignDoc::new("tshirt", "white");
        d.front.push(Layer::new(
            "a",
            1.0,
            1.0,
            LayerKind::Text(TextLayer {
                text: text.into(),
                font: FontId::Oswald,
                size_cm: 2.0,
                color: Color::BLACK,
                outline: None,
                curve: 0.0,
                align: Align::Center,
            }),
        ));
        d
    }

    #[test]
    fn accepts_a_normal_design() {
        assert!(validate(&text_doc("Salom\ndunyo"), false).is_ok());
    }

    #[test]
    fn rejects_hostile_values() {
        assert!(validate(&text_doc(&"x".repeat(201)), false).is_err());
        assert!(validate(&text_doc("a\u{0007}b"), false).is_err());

        let mut d = text_doc("ok");
        d.front[0].scale = f32::NAN;
        assert!(validate(&d, false).is_err());

        let mut d = text_doc("ok");
        d.garment = "../etc".into();
        assert!(validate(&d, false).is_err());

        let mut d = text_doc("ok");
        d.front = (0..41).map(|_| d.front[0].clone()).collect();
        assert!(validate(&d, false).is_err());
    }

    #[test]
    fn foil_is_gated() {
        let mut d = text_doc("ok");
        d.front[0].finish = Finish::FoilGold;
        assert!(validate(&d, false).is_err());
        assert!(validate(&d, true).is_ok());
    }

    #[test]
    fn point_budget_is_per_side() {
        let mut d = DesignDoc::new("tshirt", "white");
        let stroke = Stroke { points: vec![[0.0, 0.0]; 10_001], width_cm: 0.3, color: Color::BLACK };
        d.front.push(Layer::new("a", 0.0, 0.0, LayerKind::Draw(DrawLayer { strokes: vec![stroke.clone(), stroke] })));
        assert!(validate(&d, false).is_err());
    }
}
