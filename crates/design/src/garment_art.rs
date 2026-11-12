//! Flat garment illustrations (front and back) as SVG, tinted at runtime.
//!
//! Drawn for this project so there are no image-licensing questions. The
//! same SVG string is shown by the browser (as an `<img>` data URL) and
//! rasterised by the server with resvg, so mockups match. Replace these with
//! photographs later by swapping this module for stored images; the print
//! area and try-on anchors are expressed in the same 1000x1100 art space.

use crate::doc::{Color, GarmentKind, Side};
use crate::tryon::{Sleeve, TryOnAnchors};

pub const ART_W: f32 = 1000.0;
pub const ART_H: f32 = 1100.0;

const TEE_BODY: &str = "M400 90 L180 150 Q150 160 135 190 L60 370 L190 430 L260 330 L260 1040 Q500 1060 740 1040 \
                        L740 330 L810 430 L940 370 L865 190 Q850 160 820 150 L600 90 Q500 NECK 400 90 Z";
const LONG_BODY: &str = "M400 90 L190 150 Q150 165 135 215 L40 860 L150 885 L262 420 L262 1000 L270 1040 L730 1040 \
                         L738 1000 L738 420 L850 885 L960 860 L865 215 Q850 165 810 150 L600 90 Q500 NECK 400 90 Z";

/// SVG source for a garment side in the given colour.
pub fn garment_svg(kind: GarmentKind, side: Side, color: Color) -> String {
    let fill = color.to_hex();
    // Seam lines must stay visible on both black and white garments.
    let (line, line_op) = if color.luma() < 110 { ("#ffffff", "0.26") } else { ("#000000", "0.30") };
    let front = side == Side::Front;
    let mut s = String::with_capacity(4096);
    s.push_str(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1100" width="1000" height="1100">"#);
    s.push_str(
        r##"<defs><linearGradient id="sh" x1="0" x2="1" y1="0" y2="0"><stop offset="0" stop-color="#000" stop-opacity="0.22"/><stop offset="0.22" stop-color="#000" stop-opacity="0"/><stop offset="0.78" stop-color="#000" stop-opacity="0"/><stop offset="1" stop-color="#000" stop-opacity="0.22"/></linearGradient></defs>"##,
    );
    let stroke = format!(r#"fill="none" stroke="{line}" stroke-opacity="{line_op}" stroke-linecap="round" stroke-linejoin="round""#);

    let body = match kind {
        GarmentKind::Tshirt => TEE_BODY.replace("NECK", if front { "200" } else { "130" }),
        GarmentKind::Sweatshirt => LONG_BODY.replace("NECK", if front { "190" } else { "125" }),
        GarmentKind::Hoodie => LONG_BODY.replace("NECK", if front { "230" } else { "110" }),
    };

    // The hood on the back view hangs over the shoulders, so it is drawn
    // after the body; on the front view it sits behind the neckline.
    if kind == GarmentKind::Hoodie && front {
        s.push_str(&format!(r#"<path d="M392 100 Q360 18 500 12 Q640 18 608 100 Q500 215 392 100 Z" fill="{fill}"/>"#));
        s.push_str(r##"<path d="M392 100 Q360 18 500 12 Q640 18 608 100 Q500 215 392 100 Z" fill="#000" fill-opacity="0.2"/>"##);
    }
    s.push_str(&format!(r#"<path d="{body}" fill="{fill}"/>"#));
    s.push_str(&format!(r#"<path d="{body}" fill="url(#sh)"/>"#));
    s.push_str(&format!(r#"<path d="{body}" {stroke} stroke-width="5"/>"#));

    match kind {
        GarmentKind::Tshirt => {
            let neck = if front { "M400 90 Q500 200 600 90" } else { "M400 90 Q500 130 600 90" };
            s.push_str(&format!(r#"<path d="{neck}" {stroke} stroke-width="16"/>"#));
            s.push_str(&format!(
                r#"<path d="M72 345 L200 405 M928 345 L800 405 M260 1012 Q500 1032 740 1012 M260 330 Q215 240 205 145 M740 330 Q785 240 795 145" {stroke} stroke-width="4"/>"#
            ));
        }
        GarmentKind::Sweatshirt | GarmentKind::Hoodie => {
            s.push_str(&format!(
                r#"<path d="M50 815 L160 840 M950 815 L840 840 M263 985 H737 M262 420 Q225 280 212 152 M738 420 Q775 280 788 152" {stroke} stroke-width="4"/>"#
            ));
            if kind == GarmentKind::Sweatshirt {
                let neck = if front { "M400 90 Q500 190 600 90" } else { "M400 90 Q500 125 600 90" };
                s.push_str(&format!(r#"<path d="{neck}" {stroke} stroke-width="18"/>"#));
            } else if front {
                s.push_str(&format!(r#"<path d="M400 92 Q500 228 600 92 M340 770 L660 770 L700 935 L300 935 Z" {stroke} stroke-width="5"/>"#));
                s.push_str(&format!(r#"<path d="M462 160 L456 300 M538 160 L544 300" {stroke} stroke-width="8"/>"#));
            } else {
                let hood = "M388 96 Q500 50 612 96 L648 250 Q500 335 352 250 Z";
                s.push_str(&format!(r#"<path d="{hood}" fill="{fill}"/>"#));
                s.push_str(&format!(r##"<path d="{hood}" fill="#000" fill-opacity="0.08"/>"##));
                s.push_str(&format!(r#"<path d="{hood}" {stroke} stroke-width="5"/>"#));
                s.push_str(&format!(r#"<path d="M500 62 L500 300" {stroke} stroke-width="3"/>"#));
            }
        }
    }
    s.push_str("</svg>");
    s
}

/// Where the body joints sit on the artwork, used by the live try-on warp.
pub fn tryon_anchors(kind: GarmentKind) -> TryOnAnchors {
    match kind {
        GarmentKind::Tshirt => TryOnAnchors {
            shoulder_l: [205.0, 150.0],
            shoulder_r: [795.0, 150.0],
            hip_l: [300.0, 960.0],
            hip_r: [700.0, 960.0],
            armpit_l_x: 260.0,
            armpit_r_x: 740.0,
            sleeve_l: Sleeve::Short { tip: [125.0, 400.0] },
            sleeve_r: Sleeve::Short { tip: [875.0, 400.0] },
        },
        GarmentKind::Hoodie | GarmentKind::Sweatshirt => TryOnAnchors {
            shoulder_l: [200.0, 160.0],
            shoulder_r: [800.0, 160.0],
            hip_l: [305.0, 960.0],
            hip_r: [695.0, 960.0],
            armpit_l_x: 262.0,
            armpit_r_x: 738.0,
            sleeve_l: Sleeve::Long { elbow: [148.0, 520.0], wrist: [95.0, 872.0] },
            sleeve_r: Sleeve::Long { elbow: [852.0, 520.0], wrist: [905.0, 872.0] },
        },
    }
}

/// A colour that reads well on the garment, for templates and new text.
pub fn contrast_ink(garment: Color) -> Color {
    if garment.luma() < 140 {
        Color::WHITE
    } else {
        Color::rgb(17, 17, 17)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::svgpath;

    #[test]
    fn body_paths_are_parseable_and_inside_the_art_box() {
        for body in [TEE_BODY, LONG_BODY] {
            let p = svgpath::parse(&body.replace("NECK", "200")).unwrap();
            for c in p {
                if let crate::display::PathCmd::L(x, y) | crate::display::PathCmd::M(x, y) = c {
                    assert!((0.0..=ART_W).contains(&x) && (0.0..=ART_H).contains(&y));
                }
            }
        }
    }

    #[test]
    fn svg_is_tinted_and_well_formed() {
        for kind in [GarmentKind::Tshirt, GarmentKind::Hoodie, GarmentKind::Sweatshirt] {
            for side in Side::ALL {
                let svg = garment_svg(kind, side, Color::rgb(255, 223, 0));
                assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
                assert!(svg.contains("#ffdf00"));
                assert!(!svg.contains("NECK"));
                assert_eq!(svg.matches("<path").count(), svg.matches("/>").count() - 4, "unbalanced tags in {kind:?}");
            }
        }
    }

    #[test]
    fn ink_contrasts() {
        assert_eq!(contrast_ink(Color::BLACK), Color::WHITE);
        assert_ne!(contrast_ink(Color::WHITE), Color::WHITE);
    }
}
