//! Built-in single-colour stickers.
//!
//! Each sticker is one SVG path in a 100x100 box, drawn by us for this
//! project (no third-party artwork, so no licensing questions). They are
//! filled with the even-odd rule so inner sub-paths become holes.

use crate::display::PathCmd;
use crate::svgpath;

pub struct StickerDef {
    pub id: &'static str,
    pub width: f32,
    pub height: f32,
    pub d: &'static str,
}

pub const STICKERS: &[StickerDef] = &[
    StickerDef { id: "bolt", width: 100.0, height: 100.0, d: "M58 2 L18 56 H44 L36 98 L82 40 H54 Z" },
    StickerDef {
        id: "crown",
        width: 100.0,
        height: 100.0,
        d: "M8 78 L4 26 L30 48 L50 12 L70 48 L96 26 L92 78 Z M8 84 H92 V94 H8 Z",
    },
    StickerDef {
        id: "flame",
        width: 100.0,
        height: 100.0,
        d: "M50 4 C58 26 82 38 82 64 C82 84 68 96 50 96 C32 96 18 84 18 64 C18 50 26 40 32 32 C34 44 40 50 46 50 C40 34 44 16 50 4 Z",
    },
    StickerDef {
        id: "smile",
        width: 100.0,
        height: 100.0,
        d: "M50 4 C75.4 4 96 24.6 96 50 C96 75.4 75.4 96 50 96 C24.6 96 4 75.4 4 50 C4 24.6 24.6 4 50 4 Z \
            M34 31 C37.9 31 41 34.1 41 38 C41 41.9 37.9 45 34 45 C30.1 45 27 41.9 27 38 C27 34.1 30.1 31 34 31 Z \
            M66 31 C69.9 31 73 34.1 73 38 C73 41.9 69.9 45 66 45 C62.1 45 59 41.9 59 38 C59 34.1 62.1 31 66 31 Z \
            M26 58 Q50 88 74 58 Q50 72 26 58 Z",
    },
    StickerDef {
        id: "moon",
        width: 100.0,
        height: 100.0,
        d: "M62 4 C36 8 18 28 18 52 C18 76 38 96 62 96 C70 96 78 94 84 90 C62 88 46 72 46 50 C46 30 58 14 76 8 C72 5 67 4 62 4 Z",
    },
    StickerDef {
        id: "mountain",
        width: 100.0,
        height: 100.0,
        d: "M2 90 L34 26 L50 54 L64 36 L98 90 Z M34 36 L27 50 L34 46 L41 52 Z",
    },
    StickerDef { id: "diamond", width: 100.0, height: 100.0, d: "M20 10 H80 L98 38 L50 94 L2 38 Z M24 40 H76 L50 80 Z" },
    StickerDef { id: "arrow", width: 100.0, height: 100.0, d: "M4 40 H60 V18 L96 50 L60 82 V60 H4 Z" },
    StickerDef {
        id: "sun",
        width: 100.0,
        height: 100.0,
        d: "M50 28 C62.2 28 72 37.8 72 50 C72 62.2 62.2 72 50 72 C37.8 72 28 62.2 28 50 C28 37.8 37.8 28 50 28 Z \
            M46 4 H54 L52 20 H48 Z M46 96 H54 L52 80 H48 Z M4 46 V54 L20 52 V48 Z M96 46 V54 L80 52 V48 Z \
            M16 21 L21 16 L33 29 L29 33 Z M84 21 L79 16 L67 29 L71 33 Z M16 79 L21 84 L33 71 L29 67 Z M84 79 L79 84 L67 71 L71 67 Z",
    },
    StickerDef {
        id: "bubble",
        width: 100.0,
        height: 100.0,
        d: "M10 10 H90 Q96 10 96 16 V62 Q96 68 90 68 H46 L26 90 V68 H10 Q4 68 4 62 V16 Q4 10 10 10 Z",
    },
];

pub fn find(id: &str) -> Option<&'static StickerDef> {
    STICKERS.iter().find(|s| s.id == id)
}

/// Sticker outline scaled to `width_cm` and centred on the origin.
pub fn path_cm(def: &StickerDef, width_cm: f32) -> Vec<PathCmd> {
    let k = width_cm / def.width;
    let (ox, oy) = (def.width / 2.0, def.height / 2.0);
    let m = |x: f32, y: f32| ((x - ox) * k, (y - oy) * k);
    // The built-in paths are covered by the test below, so a parse failure
    // cannot happen at runtime; an empty path is the harmless fallback.
    svgpath::parse(def.d)
        .unwrap_or_default()
        .into_iter()
        .map(|c| match c {
            PathCmd::M(x, y) => {
                let (x, y) = m(x, y);
                PathCmd::M(x, y)
            }
            PathCmd::L(x, y) => {
                let (x, y) = m(x, y);
                PathCmd::L(x, y)
            }
            PathCmd::Q(a, b, x, y) => {
                let (a, b) = m(a, b);
                let (x, y) = m(x, y);
                PathCmd::Q(a, b, x, y)
            }
            PathCmd::C(a, b, c2, d, x, y) => {
                let (a, b) = m(a, b);
                let (c2, d) = m(c2, d);
                let (x, y) = m(x, y);
                PathCmd::C(a, b, c2, d, x, y)
            }
            PathCmd::Z => PathCmd::Z,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_stickers_parse_and_fit_their_box() {
        for s in STICKERS {
            let p = svgpath::parse(s.d).unwrap_or_else(|e| panic!("{}: {e}", s.id));
            assert!(p.len() > 4, "{}", s.id);
            for c in path_cm(s, 10.0) {
                if let PathCmd::M(x, y) | PathCmd::L(x, y) = c {
                    assert!(x.abs() <= 5.01 && y.abs() <= 5.01, "{} point {x},{y}", s.id);
                }
            }
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut ids: Vec<_> = STICKERS.iter().map(|s| s.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), STICKERS.len());
    }
}
