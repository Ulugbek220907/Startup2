//! Minimal SVG path-data parser for the bundled sticker art.
//!
//! Supports M/L/H/V/C/Q/Z in absolute and relative form. That is all our
//! own sticker paths use, so arcs and smooth shorthands are rejected rather
//! than silently mis-drawn.

use crate::display::PathCmd;

pub fn parse(d: &str) -> Result<Vec<PathCmd>, String> {
    let mut out = Vec::new();
    let mut nums = Tokens { s: d.as_bytes(), i: 0 };
    let (mut cx, mut cy) = (0.0f32, 0.0f32); // current point
    let (mut sx, mut sy) = (0.0f32, 0.0f32); // sub-path start
    let mut cmd = b' ';
    loop {
        nums.skip_ws();
        if nums.i >= nums.s.len() {
            break;
        }
        let c = nums.s[nums.i];
        if c.is_ascii_alphabetic() {
            cmd = c;
            nums.i += 1;
            if cmd == b'Z' || cmd == b'z' {
                out.push(PathCmd::Z);
                cx = sx;
                cy = sy;
                continue;
            }
        } else if cmd == b' ' {
            return Err("path data must start with a command".into());
        } else if cmd == b'M' {
            // Extra coordinate pairs after a moveto are implicit linetos.
            cmd = b'L';
        } else if cmd == b'm' {
            cmd = b'l';
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let (x, y) = (nums.num()? + ox, nums.num()? + oy);
                out.push(PathCmd::M(x, y));
                cx = x;
                cy = y;
                sx = x;
                sy = y;
            }
            b'L' => {
                let (x, y) = (nums.num()? + ox, nums.num()? + oy);
                out.push(PathCmd::L(x, y));
                cx = x;
                cy = y;
            }
            b'H' => {
                let x = nums.num()? + ox;
                out.push(PathCmd::L(x, cy));
                cx = x;
            }
            b'V' => {
                let y = nums.num()? + oy;
                out.push(PathCmd::L(cx, y));
                cy = y;
            }
            b'Q' => {
                let (x1, y1) = (nums.num()? + ox, nums.num()? + oy);
                let (x, y) = (nums.num()? + ox, nums.num()? + oy);
                out.push(PathCmd::Q(x1, y1, x, y));
                cx = x;
                cy = y;
            }
            b'C' => {
                let (x1, y1) = (nums.num()? + ox, nums.num()? + oy);
                let (x2, y2) = (nums.num()? + ox, nums.num()? + oy);
                let (x, y) = (nums.num()? + ox, nums.num()? + oy);
                out.push(PathCmd::C(x1, y1, x2, y2, x, y));
                cx = x;
                cy = y;
            }
            other => return Err(format!("unsupported path command {:?}", other as char)),
        }
    }
    Ok(out)
}

struct Tokens<'a> {
    s: &'a [u8],
    i: usize,
}

impl Tokens<'_> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() && (self.s[self.i].is_ascii_whitespace() || self.s[self.i] == b',') {
            self.i += 1;
        }
    }

    fn num(&mut self) -> Result<f32, String> {
        self.skip_ws();
        let start = self.i;
        if self.i < self.s.len() && (self.s[self.i] == b'-' || self.s[self.i] == b'+') {
            self.i += 1;
        }
        let mut seen_dot = false;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_digit() {
                self.i += 1;
            } else if c == b'.' && !seen_dot {
                seen_dot = true;
                self.i += 1;
            } else {
                break;
            }
        }
        // The slice is ASCII digits/sign/dot only, so it is valid UTF-8.
        let txt = std::str::from_utf8(&self.s[start..self.i]).unwrap_or("");
        txt.parse::<f32>().map_err(|_| format!("expected a number at byte {start}"))
    }
}

/// Serialise a path back to SVG path data (used for garment art overlays).
pub fn to_svg(path: &[PathCmd]) -> String {
    let mut s = String::new();
    for c in path {
        match *c {
            PathCmd::M(x, y) => s.push_str(&format!("M{x:.2} {y:.2}")),
            PathCmd::L(x, y) => s.push_str(&format!("L{x:.2} {y:.2}")),
            PathCmd::Q(a, b, x, y) => s.push_str(&format!("Q{a:.2} {b:.2} {x:.2} {y:.2}")),
            PathCmd::C(a, b, c2, d, x, y) => s.push_str(&format!("C{a:.2} {b:.2} {c2:.2} {d:.2} {x:.2} {y:.2}")),
            PathCmd::Z => s.push('Z'),
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_absolute_and_relative() {
        let p = parse("M10 10 h5 v5 l-5,0 z m1 1 L2 2 Q3 3 4 4 c1 0 1 1 2 2").unwrap();
        assert_eq!(p[0], PathCmd::M(10.0, 10.0));
        assert_eq!(p[1], PathCmd::L(15.0, 10.0));
        assert_eq!(p[2], PathCmd::L(15.0, 15.0));
        assert_eq!(p[3], PathCmd::L(10.0, 15.0));
        assert_eq!(p[4], PathCmd::Z);
        // After Z the current point is the sub-path start (10,10).
        assert_eq!(p[5], PathCmd::M(11.0, 11.0));
        assert_eq!(p[7], PathCmd::Q(3.0, 3.0, 4.0, 4.0));
        assert_eq!(p[8], PathCmd::C(5.0, 4.0, 5.0, 5.0, 6.0, 6.0));
    }

    #[test]
    fn implicit_lineto_and_compact_numbers() {
        let p = parse("M0 0 10 0 10-5").unwrap();
        assert_eq!(p, vec![PathCmd::M(0.0, 0.0), PathCmd::L(10.0, 0.0), PathCmd::L(10.0, -5.0)]);
    }

    #[test]
    fn rejects_arcs_and_garbage() {
        assert!(parse("M0 0 A5 5 0 0 1 10 10").is_err());
        assert!(parse("10 10").is_err());
        assert!(parse("M0 x").is_err());
    }
}
