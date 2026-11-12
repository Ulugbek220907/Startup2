//! Pure math for the live camera try-on (Tier A).
//!
//! Kept free of browser types so it can be unit-tested natively: landmark
//! smoothing (One-Euro filter), pose sanity checks that drive the on-screen
//! tips, and the mesh that warps the flat garment artwork onto the body.

use crate::garment_art::{ART_H, ART_W};
use crate::geom::smoothstep;

type P = [f32; 2];

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
fn mul(a: P, k: f32) -> P {
    [a[0] * k, a[1] * k]
}
fn lerp(a: P, b: P, t: f32) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}
fn len(a: P) -> f32 {
    (a[0] * a[0] + a[1] * a[1]).sqrt()
}
fn dot(a: P, b: P) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
/// Rotate 90° (same handedness in art and screen space, both y-down).
fn perp(a: P) -> P {
    [-a[1], a[0]]
}

// ---------------------------------------------------------------- filter

/// One-Euro filter (Casiez et al. 2012): a low-pass filter whose cutoff
/// rises with speed, so a still body does not jitter and a moving body does
/// not lag.
#[derive(Clone, Debug)]
pub struct OneEuro {
    min_cutoff: f32,
    beta: f32,
    d_cutoff: f32,
    prev: Option<(f32, f32, f64)>, // (value, derivative, time)
}

impl OneEuro {
    pub fn new(min_cutoff: f32, beta: f32) -> Self {
        OneEuro { min_cutoff, beta, d_cutoff: 1.0, prev: None }
    }

    fn alpha(cutoff: f32, dt: f32) -> f32 {
        let tau = 1.0 / (2.0 * std::f32::consts::PI * cutoff);
        1.0 / (1.0 + tau / dt)
    }

    /// Feed a sample taken at `t` seconds; returns the smoothed value.
    pub fn filter(&mut self, x: f32, t: f64) -> f32 {
        let Some((px, pdx, pt)) = self.prev else {
            self.prev = Some((x, 0.0, t));
            return x;
        };
        let dt = ((t - pt) as f32).max(1e-3);
        let dx = (x - px) / dt;
        let a_d = Self::alpha(self.d_cutoff, dt);
        let dx_hat = a_d * dx + (1.0 - a_d) * pdx;
        let cutoff = self.min_cutoff + self.beta * dx_hat.abs();
        let a = Self::alpha(cutoff, dt);
        let x_hat = a * x + (1.0 - a) * px;
        self.prev = Some((x_hat, dx_hat, t));
        x_hat
    }

    pub fn reset(&mut self) {
        self.prev = None;
    }
}

// ------------------------------------------------------------------ pose

/// One pose landmark in normalised image coordinates (0..1).
#[derive(Clone, Copy, Debug, Default)]
pub struct Landmark {
    pub x: f32,
    pub y: f32,
    pub visibility: f32,
}

// MediaPipe Pose landmark indices.
pub const LM_SHOULDER_L: usize = 11;
pub const LM_SHOULDER_R: usize = 12;
pub const LM_ELBOW_L: usize = 13;
pub const LM_ELBOW_R: usize = 14;
pub const LM_WRIST_L: usize = 15;
pub const LM_WRIST_R: usize = 16;
pub const LM_HIP_L: usize = 23;
pub const LM_HIP_R: usize = 24;
pub const LM_COUNT: usize = 33;

/// Body joints in screen pixels. `_l` means the *screen-left* side, not the
/// person's anatomical left, so the garment never flips inside out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyPose {
    pub shoulder_l: P,
    pub shoulder_r: P,
    pub hip_l: P,
    pub hip_r: P,
    pub elbow_l: P,
    pub elbow_r: P,
    pub wrist_l: P,
    pub wrist_r: P,
    /// Whether each forearm is visible in front of the torso, so the
    /// compositor can draw the arm over the garment.
    pub forearm_front_l: bool,
    pub forearm_front_r: bool,
}

/// What to tell the user when the pose cannot be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseIssue {
    NoPerson,
    /// Shoulders or hips are cut off or the body fills the frame.
    StepBack,
    /// Body is turned sideways.
    FaceCamera,
}

const VIS_OK: f32 = 0.5;

impl BodyPose {
    /// Build a pose from MediaPipe landmarks. `mirrored` is true for the
    /// selfie camera, whose picture is shown flipped like a mirror.
    pub fn from_landmarks(lm: &[Landmark], frame_w: f32, frame_h: f32, mirrored: bool) -> Result<BodyPose, PoseIssue> {
        if lm.len() < LM_COUNT {
            return Err(PoseIssue::NoPerson);
        }
        let px = |i: usize| -> P {
            let x = if mirrored { 1.0 - lm[i].x } else { lm[i].x };
            [x * frame_w, lm[i].y * frame_h]
        };
        let vis = |i: usize| lm[i].visibility;
        if vis(LM_SHOULDER_L) < VIS_OK || vis(LM_SHOULDER_R) < VIS_OK {
            return Err(PoseIssue::NoPerson);
        }
        // Order both sides by screen x.
        let (mut sl, mut sr) = (LM_SHOULDER_L, LM_SHOULDER_R);
        if px(sl)[0] > px(sr)[0] {
            std::mem::swap(&mut sl, &mut sr);
        }
        // Landmark ids come in left/right pairs: +2 elbow, +4 wrist, +12 hip.
        let (s_l, s_r) = (px(sl), px(sr));
        let sw = len(sub(s_r, s_l));
        if sw < frame_w * 0.08 {
            return Err(PoseIssue::FaceCamera);
        }
        let in_frame = |p: P| p[0] >= 0.0 && p[0] <= frame_w && p[1] >= 0.0 && p[1] <= frame_h;
        if !in_frame(s_l) || !in_frame(s_r) || sw > frame_w * 0.85 {
            return Err(PoseIssue::StepBack);
        }

        // Hips are often out of frame on a phone held at arm's length. When
        // they are not reliable, estimate them from the shoulders using
        // average body proportions (torso ≈ 1.45 shoulder widths long, hips
        // ≈ 0.68 shoulder widths apart).
        let axis = mul(sub(s_r, s_l), 1.0 / sw);
        let down = perp(axis);
        let mid = lerp(s_l, s_r, 0.5);
        let hips_ok = vis(sl + 12) >= VIS_OK && vis(sr + 12) >= VIS_OK;
        let (h_l, h_r) = if hips_ok {
            (px(sl + 12), px(sr + 12))
        } else {
            let hm = add(mid, mul(down, sw * 1.45));
            (sub(hm, mul(axis, sw * 0.34)), add(hm, mul(axis, sw * 0.34)))
        };
        let torso = len(sub(lerp(h_l, h_r, 0.5), mid));
        if torso < sw * 0.6 {
            // Shoulders far apart but hips right below them: bending/too close.
            return Err(PoseIssue::StepBack);
        }
        if sw < torso * 0.35 {
            return Err(PoseIssue::FaceCamera);
        }

        // Arms: fall back to hanging straight down when not visible.
        let arm = |s: P, shoulder_i: usize, outward: f32| -> (P, P, bool) {
            let (ei, wi) = (shoulder_i + 2, shoulder_i + 4);
            let e = if vis(ei) >= VIS_OK { px(ei) } else { add(s, add(mul(down, torso * 0.55), mul(axis, outward * sw * 0.08))) };
            let w_vis = vis(wi) >= VIS_OK;
            let w = if w_vis { px(wi) } else { add(e, mul(down, torso * 0.5)) };
            (e, w, w_vis)
        };
        let (e_l, w_l, wl_vis) = arm(s_l, sl, -1.0);
        let (e_r, w_r, wr_vis) = arm(s_r, sr, 1.0);

        // A forearm counts as "in front" when its wrist is visible and lies
        // inside the torso quad.
        let inside_torso = |p: P| {
            let t = dot(sub(p, mid), down) / torso;
            let s = dot(sub(p, mid), axis) / (sw * 0.5);
            (0.0..=1.05).contains(&t) && s.abs() < 0.95
        };
        Ok(BodyPose {
            shoulder_l: s_l,
            shoulder_r: s_r,
            hip_l: h_l,
            hip_r: h_r,
            elbow_l: e_l,
            elbow_r: e_r,
            wrist_l: w_l,
            wrist_r: w_r,
            forearm_front_l: wl_vis && inside_torso(w_l),
            forearm_front_r: wr_vis && inside_torso(w_r),
        })
    }
}

// ------------------------------------------------------------------ mesh

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Sleeve {
    /// T-shirt: `tip` is the middle of the sleeve hem.
    Short { tip: P },
    /// Long sleeve: joint positions on the artwork.
    Long { elbow: P, wrist: P },
}

/// Joint positions on the garment artwork, in art pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TryOnAnchors {
    pub shoulder_l: P,
    pub shoulder_r: P,
    pub hip_l: P,
    pub hip_r: P,
    /// Art x where the sleeve joins the body.
    pub armpit_l_x: f32,
    pub armpit_r_x: f32,
    pub sleeve_l: Sleeve,
    pub sleeve_r: Sleeve,
}

/// Triangle mesh: screen positions (px), texture coordinates (0..1) and
/// triangle indices, ready for a WebGL vertex buffer.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub positions: Vec<f32>,
    pub uvs: Vec<f32>,
    pub indices: Vec<u16>,
}

/// How far along the upper arm a short sleeve reaches.
const SHORT_SLEEVE_REACH: f32 = 0.6;

/// Map a bone-relative art point onto the matching screen bone. Along the
/// bone the garment stretches with the limb; across it, it keeps the torso
/// scale `k` so sleeves do not get fat when an arm points at the camera.
fn bone_map(p: P, a0: P, a1: P, b0: P, b1: P, k: f32) -> P {
    let da = sub(a1, a0);
    let db = sub(b1, b0);
    let (la, lb) = (len(da).max(1e-3), len(db).max(1e-3));
    let (ua, ub) = (mul(da, 1.0 / la), mul(db, 1.0 / lb));
    let rel = sub(p, a0);
    let along = dot(rel, ua);
    let across = dot(rel, perp(ua));
    add(b0, add(mul(ub, along * lb / la), mul(perp(ub), across * k)))
}

/// Build the warp mesh: a `cols` x `rows` grid over the artwork where each
/// vertex is moved by blending three transforms (linear blend skinning):
/// the torso quad, the upper arm and, for long sleeves, the forearm.
pub fn build_mesh(a: &TryOnAnchors, p: &BodyPose, cols: usize, rows: usize) -> Mesh {
    let cols = cols.clamp(2, 60);
    let rows = rows.clamp(2, 60);
    let k = len(sub(p.shoulder_r, p.shoulder_l)) / (a.shoulder_r[0] - a.shoulder_l[0]).max(1.0);

    let torso = |q: P| -> P {
        // Bilinear map through shoulders and hips, extrapolated linearly so
        // the collar (above the shoulders) and hem (below the hips) follow.
        let t = (q[1] - a.shoulder_l[1]) / (a.hip_l[1] - a.shoulder_l[1]);
        let xl = a.shoulder_l[0] + (a.hip_l[0] - a.shoulder_l[0]) * t;
        let xr = a.shoulder_r[0] + (a.hip_r[0] - a.shoulder_r[0]) * t;
        let s = (q[0] - xl) / (xr - xl).max(1.0);
        lerp(lerp(p.shoulder_l, p.hip_l, t), lerp(p.shoulder_r, p.hip_r, t), s)
    };

    let arm = |q: P, sleeve: Sleeve, sh_a: P, sh_p: P, el_p: P, wr_p: P| -> P {
        match sleeve {
            Sleeve::Short { tip } => bone_map(q, sh_a, tip, sh_p, lerp(sh_p, el_p, SHORT_SLEEVE_REACH), k),
            Sleeve::Long { elbow, wrist } => {
                let upper = bone_map(q, sh_a, elbow, sh_p, el_p, k);
                let fore = bone_map(q, elbow, wrist, el_p, wr_p, k);
                // Blend across the elbow so the sleeve bends smoothly.
                let fa = sub(wrist, elbow);
                let along = dot(sub(q, elbow), fa) / dot(fa, fa).max(1.0);
                let w = smoothstep(-0.15, 0.15, along);
                lerp(upper, fore, w)
            }
        }
    };

    let mut m = Mesh::default();
    for r in 0..rows {
        for c in 0..cols {
            let (u, v) = (c as f32 / (cols - 1) as f32, r as f32 / (rows - 1) as f32);
            let q = [u * ART_W, v * ART_H];
            let t = torso(q);
            // 0 inside the body, 1 well out on the sleeve.
            let wl = smoothstep(a.armpit_l_x + 10.0, a.armpit_l_x - 40.0, q[0]);
            let wr = smoothstep(a.armpit_r_x - 10.0, a.armpit_r_x + 40.0, q[0]);
            let pos = if wl > 0.0 {
                lerp(t, arm(q, a.sleeve_l, a.shoulder_l, p.shoulder_l, p.elbow_l, p.wrist_l), wl)
            } else if wr > 0.0 {
                lerp(t, arm(q, a.sleeve_r, a.shoulder_r, p.shoulder_r, p.elbow_r, p.wrist_r), wr)
            } else {
                t
            };
            m.positions.extend_from_slice(&pos);
            m.uvs.extend_from_slice(&[u, v]);
        }
    }
    for r in 0..rows - 1 {
        for c in 0..cols - 1 {
            let i = (r * cols + c) as u16;
            let (right, below, diag) = (i + 1, i + cols as u16, i + cols as u16 + 1);
            m.indices.extend_from_slice(&[i, right, below, right, diag, below]);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::GarmentKind;
    use crate::garment_art::tryon_anchors;

    /// A pose that is exactly the artwork scaled by `k` and moved by `off`.
    fn rest_pose(a: &TryOnAnchors, k: f32, off: P) -> BodyPose {
        let m = |p: P| add(mul(p, k), off);
        let (el, wl, er, wr) = match (a.sleeve_l, a.sleeve_r) {
            (Sleeve::Long { elbow: el, wrist: wl }, Sleeve::Long { elbow: er, wrist: wr }) => (el, wl, er, wr),
            (Sleeve::Short { tip: tl }, Sleeve::Short { tip: tr }) => {
                // Elbow such that the sleeve tip lands at SHORT_SLEEVE_REACH.
                let el = add(a.shoulder_l, mul(sub(tl, a.shoulder_l), 1.0 / SHORT_SLEEVE_REACH));
                let er = add(a.shoulder_r, mul(sub(tr, a.shoulder_r), 1.0 / SHORT_SLEEVE_REACH));
                (el, el, er, er)
            }
            _ => unreachable!(),
        };
        BodyPose {
            shoulder_l: m(a.shoulder_l),
            shoulder_r: m(a.shoulder_r),
            hip_l: m(a.hip_l),
            hip_r: m(a.hip_r),
            elbow_l: m(el),
            elbow_r: m(er),
            wrist_l: m(wl),
            wrist_r: m(wr),
            forearm_front_l: false,
            forearm_front_r: false,
        }
    }

    #[test]
    fn rest_pose_is_a_plain_scale() {
        // If the body matches the artwork, the warp must be the identity
        // (up to scale and offset) everywhere, including the sleeves.
        for kind in [GarmentKind::Tshirt, GarmentKind::Hoodie] {
            let a = tryon_anchors(kind);
            let (k, off) = (0.5, [40.0, 70.0]);
            let mesh = build_mesh(&a, &rest_pose(&a, k, off), 21, 23);
            for (pos, uv) in mesh.positions.chunks(2).zip(mesh.uvs.chunks(2)) {
                let want = [uv[0] * ART_W * k + off[0], uv[1] * ART_H * k + off[1]];
                assert!((pos[0] - want[0]).abs() < 0.6 && (pos[1] - want[1]).abs() < 0.6, "{kind:?} uv {uv:?}: got {pos:?}, want {want:?}");
            }
        }
    }

    #[test]
    fn raising_an_arm_moves_only_that_sleeve() {
        let a = tryon_anchors(GarmentKind::Hoodie);
        let rest = rest_pose(&a, 1.0, [0.0, 0.0]);
        let mut raised = rest;
        // Lift the screen-left arm straight out to the side.
        raised.elbow_l = [rest.shoulder_l[0] - 360.0, rest.shoulder_l[1]];
        raised.wrist_l = [rest.shoulder_l[0] - 720.0, rest.shoulder_l[1]];
        let (m0, m1) = (build_mesh(&a, &rest, 21, 23), build_mesh(&a, &raised, 21, 23));
        let mut moved_left = 0;
        for ((p0, p1), uv) in m0.positions.chunks(2).zip(m1.positions.chunks(2)).zip(m0.uvs.chunks(2)) {
            let d = len(sub([p1[0], p1[1]], [p0[0], p0[1]]));
            let x = uv[0] * ART_W;
            if x > a.armpit_l_x + 10.0 {
                assert!(d < 0.01, "body/right vertex at x={x} moved by {d}");
            } else if d > 50.0 {
                moved_left += 1;
            }
        }
        assert!(moved_left > 10);
    }

    #[test]
    fn mesh_topology() {
        let a = tryon_anchors(GarmentKind::Tshirt);
        let m = build_mesh(&a, &rest_pose(&a, 1.0, [0.0, 0.0]), 5, 4);
        assert_eq!(m.positions.len(), 5 * 4 * 2);
        assert_eq!(m.indices.len(), 4 * 3 * 6);
        assert!(m.indices.iter().all(|&i| (i as usize) < 20));
    }

    fn landmarks(shoulders: (P, P), hips: Option<(P, P)>) -> Vec<Landmark> {
        let mut lm = vec![Landmark::default(); LM_COUNT];
        let set = |lm: &mut Vec<Landmark>, i: usize, p: P| lm[i] = Landmark { x: p[0], y: p[1], visibility: 0.9 };
        set(&mut lm, LM_SHOULDER_L, shoulders.0);
        set(&mut lm, LM_SHOULDER_R, shoulders.1);
        if let Some((l, r)) = hips {
            set(&mut lm, LM_HIP_L, l);
            set(&mut lm, LM_HIP_R, r);
        }
        lm
    }

    #[test]
    fn pose_orders_sides_by_screen_position_and_mirrors() {
        // Unmirrored camera: the person's left shoulder is on the image right.
        let lm = landmarks(([0.65, 0.3], [0.35, 0.3]), Some(([0.6, 0.7], [0.4, 0.7])));
        let p = BodyPose::from_landmarks(&lm, 1000.0, 1000.0, false).unwrap();
        assert!(p.shoulder_l[0] < p.shoulder_r[0]);
        assert!((p.shoulder_l[0] - 350.0).abs() < 0.01);
        let pm = BodyPose::from_landmarks(&lm, 1000.0, 1000.0, true).unwrap();
        assert!(pm.shoulder_l[0] < pm.shoulder_r[0]);
        assert!((pm.hip_l[0] - 400.0).abs() < 0.01);
    }

    #[test]
    fn pose_estimates_missing_hips_below_the_shoulders() {
        let lm = landmarks(([0.65, 0.3], [0.35, 0.3]), None);
        let p = BodyPose::from_landmarks(&lm, 1000.0, 1000.0, false).unwrap();
        assert!(p.hip_l[1] > 600.0 && p.hip_r[1] > 600.0);
        assert!(p.hip_l[0] > p.shoulder_l[0] && p.hip_r[0] < p.shoulder_r[0]);
    }

    #[test]
    fn pose_issues() {
        assert_eq!(BodyPose::from_landmarks(&[], 100.0, 100.0, false), Err(PoseIssue::NoPerson));
        let none = vec![Landmark::default(); LM_COUNT];
        assert_eq!(BodyPose::from_landmarks(&none, 100.0, 100.0, false), Err(PoseIssue::NoPerson));
        let sideways = landmarks(([0.52, 0.3], [0.48, 0.3]), None);
        assert_eq!(BodyPose::from_landmarks(&sideways, 100.0, 100.0, false), Err(PoseIssue::FaceCamera));
        let close = landmarks(([0.97, 0.3], [0.03, 0.3]), None);
        assert_eq!(BodyPose::from_landmarks(&close, 100.0, 100.0, false), Err(PoseIssue::StepBack));
    }

    #[test]
    fn one_euro_smooths_jitter_but_follows_motion() {
        let mut f = OneEuro::new(1.0, 0.05);
        // Still signal with ±2 px noise at 30 fps: output must be calmer.
        let mut max_dev = 0.0f32;
        for i in 0..90 {
            let noise = if i % 2 == 0 { 2.0 } else { -2.0 };
            let y = f.filter(100.0 + noise, i as f64 / 30.0);
            if i > 30 {
                max_dev = max_dev.max((y - 100.0).abs());
            }
        }
        assert!(max_dev < 1.0, "jitter {max_dev}");
        // A fast sweep must be tracked with little lag.
        let mut f = OneEuro::new(1.0, 0.05);
        let mut y = 0.0;
        for i in 0..60 {
            y = f.filter(i as f32 * 20.0, i as f64 / 30.0);
        }
        assert!((y - 59.0 * 20.0).abs() < 40.0, "lag {}", 59.0 * 20.0 - y);
    }
}
