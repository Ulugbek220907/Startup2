//! Simple vector shapes, centred on the origin.

use crate::display::PathCmd;
use crate::doc::ShapeKind;

/// Control-point offset for approximating a quarter circle with a cubic.
const KAPPA: f32 = 0.552_284_8;

pub fn shape_path(kind: ShapeKind, w: f32, h: f32) -> Vec<PathCmd> {
    let (hw, hh) = (w / 2.0, h / 2.0);
    use PathCmd::*;
    match kind {
        ShapeKind::Rect => vec![M(-hw, -hh), L(hw, -hh), L(hw, hh), L(-hw, hh), Z],
        ShapeKind::Ellipse => {
            let (kx, ky) = (hw * KAPPA, hh * KAPPA);
            vec![
                M(0.0, -hh),
                C(kx, -hh, hw, -ky, hw, 0.0),
                C(hw, ky, kx, hh, 0.0, hh),
                C(-kx, hh, -hw, ky, -hw, 0.0),
                C(-hw, -ky, -kx, -hh, 0.0, -hh),
                Z,
            ]
        }
        ShapeKind::Triangle => vec![M(0.0, -hh), L(hw, hh), L(-hw, hh), Z],
        ShapeKind::Star => {
            // Five-point star: alternate outer and inner radius every 36°.
            let mut p = Vec::with_capacity(11);
            for i in 0..10 {
                let r = if i % 2 == 0 { 1.0 } else { 0.42 };
                let a = (i as f32 * 36.0 - 90.0).to_radians();
                let (x, y) = (a.cos() * r * hw, a.sin() * r * hh);
                p.push(if i == 0 { M(x, y) } else { L(x, y) });
            }
            p.push(Z);
            p
        }
        ShapeKind::Heart => {
            // Unit heart in a -1..1 box, scaled to the requested size.
            let s = |x: f32, y: f32| (x * hw, y * hh);
            let c = |a: (f32, f32), b: (f32, f32), e: (f32, f32)| C(a.0, a.1, b.0, b.1, e.0, e.1);
            let top = s(0.0, -0.45);
            vec![
                M(top.0, top.1),
                c(s(-0.25, -1.1), s(-1.0, -0.95), s(-1.0, -0.3)),
                c(s(-1.0, 0.25), s(-0.4, 0.6), s(0.0, 1.0)),
                c(s(0.4, 0.6), s(1.0, 0.25), s(1.0, -0.3)),
                c(s(1.0, -0.95), s(0.25, -1.1), s(0.0, -0.45)),
                Z,
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_stay_inside_their_box() {
        for kind in ShapeKind::ALL {
            for cmd in shape_path(kind, 10.0, 6.0) {
                let pts: Vec<(f32, f32)> = match cmd {
                    PathCmd::M(x, y) | PathCmd::L(x, y) => vec![(x, y)],
                    PathCmd::Q(_, _, x, y) | PathCmd::C(_, _, _, _, x, y) => vec![(x, y)],
                    PathCmd::Z => vec![],
                };
                for (x, y) in pts {
                    assert!(x.abs() <= 5.001 && y.abs() <= 3.001, "{kind:?} point {x},{y}");
                }
            }
        }
    }
}
