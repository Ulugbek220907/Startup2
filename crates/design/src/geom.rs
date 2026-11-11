//! Small 2D geometry helpers (no external math crate needed).

use serde::{Deserialize, Serialize};

/// 2D affine transform, column-major like the HTML canvas `setTransform`:
/// `x' = a*x + c*y + e`, `y' = b*x + d*y + f`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    pub const IDENTITY: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn translate(x: f32, y: f32) -> Self {
        Affine { e: x, f: y, ..Self::IDENTITY }
    }

    pub fn scale(sx: f32, sy: f32) -> Self {
        Affine { a: sx, d: sy, ..Self::IDENTITY }
    }

    /// Clockwise rotation in a y-down coordinate system.
    pub fn rotate_deg(deg: f32) -> Self {
        let (s, c) = deg.to_radians().sin_cos();
        Affine { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }

    /// `self.then(next)`: apply `self` first, then `next`.
    pub fn then(self, n: Affine) -> Affine {
        Affine {
            a: n.a * self.a + n.c * self.b,
            b: n.b * self.a + n.d * self.b,
            c: n.a * self.c + n.c * self.d,
            d: n.b * self.c + n.d * self.d,
            e: n.a * self.e + n.c * self.f + n.e,
            f: n.b * self.e + n.d * self.f + n.f,
        }
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    pub fn invert(&self) -> Option<Affine> {
        let det = self.a * self.d - self.b * self.c;
        if det.abs() < 1e-12 {
            return None;
        }
        let id = 1.0 / det;
        Some(Affine {
            a: self.d * id,
            b: -self.b * id,
            c: -self.c * id,
            d: self.a * id,
            e: (self.c * self.f - self.d * self.e) * id,
            f: (self.b * self.e - self.a * self.f) * id,
        })
    }
}

/// Axis-aligned rectangle. `w`/`h` are never negative for a valid rect.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const EMPTY: Rect = Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };

    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }

    pub fn centered(w: f32, h: f32) -> Self {
        Rect { x: -w / 2.0, y: -h / 2.0, w, h }
    }

    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.right() && y >= self.y && y <= self.bottom()
    }

    pub fn union(&self, o: &Rect) -> Rect {
        if self.is_empty() {
            return *o;
        }
        if o.is_empty() {
            return *self;
        }
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect { x, y, w: self.right().max(o.right()) - x, h: self.bottom().max(o.bottom()) - y }
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let r = self.right().min(o.right());
        let b = self.bottom().min(o.bottom());
        if r <= x || b <= y {
            Rect::EMPTY
        } else {
            Rect { x, y, w: r - x, h: b - y }
        }
    }

    pub fn inflate(&self, d: f32) -> Rect {
        Rect { x: self.x - d, y: self.y - d, w: self.w + 2.0 * d, h: self.h + 2.0 * d }
    }

    /// Bounding box of this rect after an affine transform.
    pub fn transformed(&self, t: &Affine) -> Rect {
        let pts = [
            t.apply(self.x, self.y),
            t.apply(self.right(), self.y),
            t.apply(self.right(), self.bottom()),
            t.apply(self.x, self.bottom()),
        ];
        Rect::from_points(pts.iter().copied())
    }

    pub fn from_points(pts: impl IntoIterator<Item = (f32, f32)>) -> Rect {
        let mut it = pts.into_iter();
        let Some((x0, y0)) = it.next() else { return Rect::EMPTY };
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (x0, y0, x0, y0);
        for (x, y) in it {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        Rect { x: min_x, y: min_y, w: max_x - min_x, h: max_y - min_y }
    }
}

/// Hermite smoothstep, clamped to 0..=1.
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 - edge0).abs() < 1e-9 {
        return if x < edge0 { 0.0 } else { 1.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invert_round_trips() {
        let t = Affine::scale(2.0, 3.0).then(Affine::rotate_deg(33.0)).then(Affine::translate(5.0, -7.0));
        let inv = t.invert().unwrap();
        let (x, y) = t.apply(1.5, -2.5);
        let (bx, by) = inv.apply(x, y);
        assert!((bx - 1.5).abs() < 1e-4 && (by + 2.5).abs() < 1e-4);
    }

    #[test]
    fn rotation_is_clockwise_in_y_down_space() {
        // A point on the +x axis moves towards +y (down) for positive angles.
        let (x, y) = Affine::rotate_deg(90.0).apply(1.0, 0.0);
        assert!(x.abs() < 1e-6 && (y - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rect_ops() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersect(&b), Rect::new(5.0, 5.0, 5.0, 5.0));
        assert_eq!(a.union(&b), Rect::new(0.0, 0.0, 15.0, 15.0));
        assert!(a.intersect(&Rect::new(20.0, 20.0, 1.0, 1.0)).is_empty());
    }
}
