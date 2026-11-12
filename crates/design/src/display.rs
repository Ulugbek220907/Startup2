//! Turns layers into a flat display list.
//!
//! The editor replays the list on an HTML canvas, the server on a tiny-skia
//! pixmap. Neither needs to know about text, shapes or stickers: everything
//! except uploaded images is already a vector path here.

use serde::{Deserialize, Serialize};

use crate::doc::{Color, DesignDoc, Layer, LayerKind, Side};
use crate::fonts::FontSource;
use crate::geom::{Affine, Rect};
use crate::{shapes, stickers, text};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum PathCmd {
    M(f32, f32),
    L(f32, f32),
    Q(f32, f32, f32, f32),
    C(f32, f32, f32, f32, f32, f32),
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Paint {
    Fill(Color, FillRule),
    /// Round-joined, round-capped stroke; width in the path's local cm.
    Stroke {
        color: Color,
        width: f32,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum DrawOp {
    Path {
        path: Vec<PathCmd>,
        paint: Paint,
        /// Local cm -> print-area cm.
        transform: Affine,
    },
    /// Uploaded image drawn into the rect `(-w/2, -h/2, w, h)` in local cm.
    Image { asset: String, w: f32, h: f32, transform: Affine },
}

/// Local-space -> print-area transform of a layer.
pub fn layer_transform(l: &Layer) -> Affine {
    let sx = if l.flip_x { -l.scale } else { l.scale };
    let sy = if l.flip_y { -l.scale } else { l.scale };
    Affine::scale(sx, sy).then(Affine::rotate_deg(l.rotation)).then(Affine::translate(l.x, l.y))
}

/// Bounds of a layer in its own local space (cm, before scale/rotation).
/// Returns a small placeholder box for text whose font is still loading so
/// the layer stays selectable in the editor.
pub fn local_bounds(l: &Layer, fonts: &dyn FontSource) -> Rect {
    match &l.kind {
        LayerKind::Text(t) => {
            let stroke = t.outline.map(|o| o.width_cm).unwrap_or(0.0);
            match text::layout(t, fonts) {
                Some(lay) if !lay.bounds.is_empty() => lay.bounds.inflate(stroke),
                _ => Rect::centered(t.size_cm * 2.0, t.size_cm),
            }
        }
        LayerKind::Image(i) => Rect::centered(i.width_cm, i.height_cm),
        LayerKind::Draw(d) => {
            let mut r = Rect::EMPTY;
            for s in &d.strokes {
                let b = Rect::from_points(s.points.iter().map(|p| (p[0], p[1]))).inflate(s.width_cm / 2.0);
                r = r.union(&b);
            }
            r
        }
        LayerKind::Shape(s) => {
            let stroke = s.stroke.map(|o| o.width_cm / 2.0).unwrap_or(0.0);
            Rect::centered(s.width_cm, s.height_cm).inflate(stroke)
        }
        LayerKind::Sticker(s) => {
            let aspect = stickers::find(&s.sticker).map(|d| d.height / d.width).unwrap_or(1.0);
            Rect::centered(s.width_cm, s.width_cm * aspect)
        }
    }
}

/// Axis-aligned bounding box of a layer in print-area cm.
pub fn layer_bbox(l: &Layer, fonts: &dyn FontSource) -> Rect {
    local_bounds(l, fonts).transformed(&layer_transform(l))
}

/// Printed area of one side in cm²: the union bounding box of all layers,
/// clipped to the print area. This is what the price tier is based on.
pub fn printed_area_cm2(layers: &[Layer], area_w: f32, area_h: f32, fonts: &dyn FontSource) -> f32 {
    let clip = Rect::new(0.0, 0.0, area_w, area_h);
    let mut u = Rect::EMPTY;
    for l in layers {
        u = u.union(&layer_bbox(l, fonts).intersect(&clip));
    }
    if u.is_empty() {
        0.0
    } else {
        u.w * u.h
    }
}

/// Drawing operations for one layer, bottom-most first.
pub fn layer_ops(l: &Layer, fonts: &dyn FontSource) -> Vec<DrawOp> {
    let transform = layer_transform(l);
    let mut ops = Vec::new();
    match &l.kind {
        LayerKind::Text(t) => {
            if let Some(lay) = text::layout(t, fonts) {
                if lay.path.is_empty() {
                    return ops;
                }
                if let Some(o) = t.outline.filter(|o| o.width_cm > 0.0) {
                    // The outline is stroked first and twice as wide, so the
                    // fill on top keeps the letter shapes crisp.
                    ops.push(DrawOp::Path { path: lay.path.clone(), paint: Paint::Stroke { color: o.color, width: o.width_cm * 2.0 }, transform });
                }
                ops.push(DrawOp::Path { path: lay.path, paint: Paint::Fill(t.color, FillRule::NonZero), transform });
            }
        }
        LayerKind::Image(i) => {
            ops.push(DrawOp::Image { asset: i.asset.clone(), w: i.width_cm, h: i.height_cm, transform });
        }
        LayerKind::Draw(d) => {
            for s in &d.strokes {
                if s.points.is_empty() {
                    continue;
                }
                ops.push(DrawOp::Path { path: freehand_path(&s.points), paint: Paint::Stroke { color: s.color, width: s.width_cm }, transform });
            }
        }
        LayerKind::Shape(s) => {
            let path = shapes::shape_path(s.shape, s.width_cm, s.height_cm);
            if let Some(fill) = s.fill {
                ops.push(DrawOp::Path { path: path.clone(), paint: Paint::Fill(fill, FillRule::NonZero), transform });
            }
            if let Some(o) = s.stroke.filter(|o| o.width_cm > 0.0) {
                ops.push(DrawOp::Path { path, paint: Paint::Stroke { color: o.color, width: o.width_cm }, transform });
            }
        }
        LayerKind::Sticker(s) => {
            if let Some(def) = stickers::find(&s.sticker) {
                ops.push(DrawOp::Path { path: stickers::path_cm(def, s.width_cm), paint: Paint::Fill(s.color, FillRule::EvenOdd), transform });
            }
        }
    }
    ops
}

/// Smooth a freehand polyline with quadratic segments through midpoints.
fn freehand_path(points: &[[f32; 2]]) -> Vec<PathCmd> {
    let mut p = Vec::with_capacity(points.len() + 1);
    p.push(PathCmd::M(points[0][0], points[0][1]));
    match points.len() {
        1 => {
            // A tap: a zero-length line renders as a dot with round caps.
            p.push(PathCmd::L(points[0][0] + 0.001, points[0][1]));
        }
        2 => p.push(PathCmd::L(points[1][0], points[1][1])),
        _ => {
            for w in points[1..].windows(2) {
                let mid = ((w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0);
                p.push(PathCmd::Q(w[0][0], w[0][1], mid.0, mid.1));
            }
            let last = points[points.len() - 1];
            p.push(PathCmd::L(last[0], last[1]));
        }
    }
    p
}

/// Display list for one side of a design.
pub fn side_ops(doc: &DesignDoc, side: Side, fonts: &dyn FontSource) -> Vec<DrawOp> {
    doc.layers(side).iter().flat_map(|l| layer_ops(l, fonts)).collect()
}

/// Top-most layer whose (rotated) box contains the point, for tap selection.
pub fn hit_test<'a>(layers: &'a [Layer], x: f32, y: f32, slop_cm: f32, fonts: &dyn FontSource) -> Option<&'a Layer> {
    layers.iter().rev().find(|l| {
        let Some(inv) = layer_transform(l).invert() else { return false };
        let (lx, ly) = inv.apply(x, y);
        // Slop is given in area cm; convert to local units.
        local_bounds(l, fonts).inflate(slop_cm / l.scale.max(1e-3)).contains(lx, ly)
    })
}

#[cfg(all(test, feature = "embed-fonts"))]
mod tests {
    use super::*;
    use crate::doc::*;
    use crate::fonts::{EmbeddedFonts, FontId};

    fn rect_layer(x: f32, y: f32, w: f32, h: f32) -> Layer {
        Layer::new("r", x, y, LayerKind::Shape(ShapeLayer { shape: ShapeKind::Rect, width_cm: w, height_cm: h, fill: Some(Color::BLACK), stroke: None }))
    }

    #[test]
    fn printed_area_is_union_bbox_clipped() {
        let layers = vec![rect_layer(5.0, 5.0, 10.0, 10.0), rect_layer(25.0, 5.0, 10.0, 10.0)];
        // Union spans x 0..30, y 0..10 -> 300 cm² inside a 30x40 area.
        let a = printed_area_cm2(&layers, 30.0, 40.0, &EmbeddedFonts);
        assert!((a - 300.0).abs() < 0.01, "{a}");
        // Half outside the area is not counted.
        let a = printed_area_cm2(&[rect_layer(0.0, 5.0, 10.0, 10.0)], 30.0, 40.0, &EmbeddedFonts);
        assert!((a - 50.0).abs() < 0.01, "{a}");
        assert_eq!(printed_area_cm2(&[], 30.0, 40.0, &EmbeddedFonts), 0.0);
    }

    #[test]
    fn rotation_grows_bbox_and_hit_test_respects_it() {
        let mut l = rect_layer(10.0, 10.0, 10.0, 2.0);
        let flat = layer_bbox(&l, &EmbeddedFonts);
        l.rotation = 90.0;
        let rot = layer_bbox(&l, &EmbeddedFonts);
        assert!((flat.w - 10.0).abs() < 1e-3 && (rot.h - 10.0).abs() < 1e-3 && (rot.w - 2.0).abs() < 1e-3);
        let layers = vec![l];
        assert!(hit_test(&layers, 10.0, 14.0, 0.0, &EmbeddedFonts).is_some());
        assert!(hit_test(&layers, 14.0, 10.0, 0.0, &EmbeddedFonts).is_none());
    }

    #[test]
    fn text_with_outline_emits_stroke_then_fill() {
        let l = Layer::new(
            "t",
            5.0,
            5.0,
            LayerKind::Text(TextLayer {
                text: "Salom".into(),
                font: FontId::Pacifico,
                size_cm: 2.0,
                color: Color::WHITE,
                outline: Some(Outline { color: Color::BLACK, width_cm: 0.1 }),
                curve: 0.0,
                align: Align::Center,
            }),
        );
        let ops = layer_ops(&l, &EmbeddedFonts);
        assert_eq!(ops.len(), 2);
        assert!(matches!(ops[0], DrawOp::Path { paint: Paint::Stroke { .. }, .. }));
        assert!(matches!(ops[1], DrawOp::Path { paint: Paint::Fill(..), .. }));
    }
}
