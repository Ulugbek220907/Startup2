//! Golden-image tests: render fixed designs and compare with committed PNGs.
//!
//! Regenerate after an intentional rendering change with
//! `UPDATE_GOLDEN=1 cargo test -p renderer --test golden` and review the
//! image diff in the pull request.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use design::fonts::{EmbeddedFonts, FontId};
use design::*;
use renderer::sheet::{SheetInfo, SheetSide};
use renderer::{encode_png, render_mockup, render_print, AreaGeom, Pixmap};

/// A deterministic test image: colour gradient with a transparent corner.
fn test_image() -> image::RgbaImage {
    image::RgbaImage::from_fn(64, 48, |x, y| {
        let a = if x < 16 && y < 16 { 0 } else { 255 };
        let checker = if (x / 8 + y / 8) % 2 == 0 { 255 } else { 60 };
        image::Rgba([(x * 4) as u8, checker, (y * 5) as u8, a])
    })
}

const ASSET: &str = "00000000-0000-4000-8000-000000000001";

fn assets() -> HashMap<String, Arc<image::RgbaImage>> {
    HashMap::from([(ASSET.to_string(), Arc::new(test_image()))])
}

fn text(id: &str, x: f32, y: f32, s: &str, font: FontId, size: f32, color: Color, curve: f32) -> Layer {
    Layer::new(
        id,
        x,
        y,
        LayerKind::Text(TextLayer { text: s.into(), font, size_cm: size, color, outline: None, curve, align: Align::Center }),
    )
}

/// One design exercising every layer type.
fn fixture() -> DesignDoc {
    let yellow = Color::rgb(255, 223, 0);
    let mut d = DesignDoc::new("hoodie", "black");
    d.front.push(text("t1", 15.0, 6.0, "OʻZBEKISTON", FontId::Montserrat, 3.0, Color::WHITE, 50.0));
    let mut outlined = text("t2", 15.0, 14.0, "Gʻalaba\nПривет", FontId::Pacifico, 2.6, yellow, 0.0);
    if let LayerKind::Text(t) = &mut outlined.kind {
        t.outline = Some(Outline { color: Color::rgb(200, 30, 60), width_cm: 0.12 });
    }
    outlined.rotation = -8.0;
    d.front.push(outlined);
    d.front.push(Layer::new(
        "s1",
        6.0,
        24.0,
        LayerKind::Shape(ShapeLayer {
            shape: ShapeKind::Star,
            width_cm: 7.0,
            height_cm: 7.0,
            fill: Some(yellow),
            stroke: Some(Outline { color: Color::WHITE, width_cm: 0.2 }),
        }),
    ));
    d.front.push(Layer::new("k1", 24.0, 24.0, LayerKind::Sticker(StickerLayer { sticker: "smile".into(), width_cm: 7.0, color: Color::WHITE })));
    let mut img = Layer::new(
        "i1",
        15.0,
        31.0,
        LayerKind::Image(ImageLayer { asset: ASSET.into(), width_cm: 8.0, height_cm: 6.0, px_width: 64, px_height: 48 }),
    );
    img.rotation = 12.0;
    img.flip_x = true;
    d.front.push(img);
    d.front.push(Layer::new(
        "d1",
        15.0,
        20.0,
        LayerKind::Draw(DrawLayer {
            strokes: vec![Stroke {
                points: (0..40).map(|i| [i as f32 * 0.5 - 10.0, (i as f32 * 0.6).sin() * 1.5]).collect(),
                width_cm: 0.35,
                color: Color::rgb(0, 229, 255),
            }],
        }),
    ));
    // Deliberately hangs over the right edge to prove clipping.
    d.back.push(text("b1", 26.0, 10.0, "ORQA", FontId::RubikMono, 5.0, Color::rgb(17, 17, 17), 0.0));
    d
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(name)
}

/// Compare with the stored image, tolerating tiny anti-aliasing differences
/// between CPU architectures (different SIMD paths round differently).
fn assert_golden(name: &str, pm: &Pixmap) {
    let png = encode_png(pm, None).unwrap();
    let path = golden_path(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &png).unwrap();
        return;
    }
    let want = image::open(&path)
        .unwrap_or_else(|e| panic!("missing golden {path:?} ({e}); run with UPDATE_GOLDEN=1"))
        .to_rgba8();
    let got = image::load_from_memory(&png).unwrap().to_rgba8();
    assert_eq!((got.width(), got.height()), (want.width(), want.height()), "{name}: size changed");
    let mut differing = 0usize;
    for (a, b) in got.pixels().zip(want.pixels()) {
        let d = a.0.iter().zip(b.0.iter()).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        if d > 12 {
            differing += 1;
        }
    }
    let ratio = differing as f64 / (got.width() * got.height()) as f64;
    if ratio > 0.002 {
        let actual = path.with_extension("actual.png");
        std::fs::write(&actual, &png).unwrap();
        panic!("{name}: {:.3}% of pixels differ; wrote {actual:?}", ratio * 100.0);
    }
}

const HOODIE_FRONT: AreaGeom = AreaGeom { width_cm: 30.0, height_cm: 35.0, mock_x: 0.381, mock_y: 0.2727, mock_w: 0.238, mock_h: 0.2524 };
const TEE_BACK: AreaGeom = AreaGeom { width_cm: 30.0, height_cm: 40.0, mock_x: 0.3665, mock_y: 0.2, mock_w: 0.267, mock_h: 0.3236 };

#[test]
fn print_front_matches_golden() {
    let pm = render_print(&fixture(), Side::Front, 30.0, 35.0, 40, &EmbeddedFonts, &assets()).unwrap();
    assert_golden("print_front_40dpi.png", &pm);
}

#[test]
fn print_back_is_clipped_and_matches_golden() {
    let pm = render_print(&fixture(), Side::Back, 30.0, 40.0, 40, &EmbeddedFonts, &assets()).unwrap();
    assert_golden("print_back_40dpi.png", &pm);
}

#[test]
fn mockups_match_golden() {
    let doc = fixture();
    let pm = render_mockup(&doc, Side::Front, GarmentKind::Hoodie, Color::rgb(24, 24, 27), &HOODIE_FRONT, 500, &EmbeddedFonts, &assets()).unwrap();
    assert_golden("mockup_hoodie_front.png", &pm);
    let pm = render_mockup(&doc, Side::Back, GarmentKind::Tshirt, Color::WHITE, &TEE_BACK, 500, &EmbeddedFonts, &assets()).unwrap();
    assert_golden("mockup_tshirt_back.png", &pm);
    let pm = render_mockup(&DesignDoc::new("sweatshirt", "x"), Side::Front, GarmentKind::Sweatshirt, Color::rgb(120, 130, 140), &TEE_BACK, 500, &EmbeddedFonts, &assets()).unwrap();
    assert_golden("mockup_sweatshirt_blank.png", &pm);
    let pm = render_mockup(&DesignDoc::new("hoodie", "x"), Side::Back, GarmentKind::Hoodie, Color::rgb(255, 223, 0), &TEE_BACK, 500, &EmbeddedFonts, &assets()).unwrap();
    assert_golden("mockup_hoodie_back_blank.png", &pm);
}

#[test]
fn production_sheet_matches_golden_and_is_a_pdf() {
    let doc = fixture();
    let mock = |side, kind, area: &AreaGeom| render_mockup(&doc, side, kind, Color::rgb(24, 24, 27), area, 500, &EmbeddedFonts, &assets()).unwrap();
    let info = SheetInfo {
        brand: "QiyiqDoʻppi".into(),
        order_code: "QD-7K3F9A".into(),
        created_at: "2026-11-11 14:05".into(),
        garment: "Xudi".into(),
        color_name: "Qora".into(),
        color: Color::rgb(24, 24, 27),
        size: "XL".into(),
        quantity: 3,
        finish: "Oddiy (DTF)".into(),
        delivery: "Toshkent shahri".into(),
        notes: Some("Iltimos, yozuvni biroz yuqoriroq joylashtiring. Срочно, к пятнице! Please pack each piece separately.".into()),
        admin_url: "https://example.uz/admin/#/orders/0b9f6c0e-1111-4222-8333-444455556666".into(),
        sides: vec![
            SheetSide { label: "Old".into(), area_w_cm: 30.0, area_h_cm: 35.0, printed_cm: Some((27.4, 33.0)), mockup: Some(mock(Side::Front, GarmentKind::Hoodie, &HOODIE_FRONT)) },
            SheetSide { label: "Orqa".into(), area_w_cm: 30.0, area_h_cm: 40.0, printed_cm: None, mockup: Some(mock(Side::Back, GarmentKind::Hoodie, &TEE_BACK)) },
        ],
        lang: domain::Lang::Uz,
    };
    let pm = renderer::sheet::render_sheet(&info, &EmbeddedFonts).unwrap();
    assert_golden("production_sheet.png", &pm);
    let pdf = renderer::sheet::production_sheet(&info, &EmbeddedFonts).unwrap();
    assert!(pdf.starts_with(b"%PDF-1.4") && pdf.ends_with(b"%%EOF\n"));
    assert!(pdf.len() < 600_000, "sheet PDF is {} bytes", pdf.len());
}

#[test]
fn print_file_has_exact_physical_size_at_300_dpi() {
    // 30 x 40 cm at 300 DPI = 3543 x 4724 px.
    let pm = render_print(&fixture(), Side::Back, 30.0, 40.0, 300, &EmbeddedFonts, &assets()).unwrap();
    assert_eq!((pm.width(), pm.height()), (3543, 4724));
    // Transparent background where nothing is printed...
    assert_eq!(pm.pixel(5, 5).unwrap().alpha(), 0);
    // ...and opaque ink inside the letters somewhere on the text's row.
    let y = (10.0 / 40.0 * 4724.0) as u32;
    assert!((2000..3543).any(|x| pm.pixel(x, y).unwrap().alpha() == 255));

    let png = encode_png(&pm, Some(300)).unwrap();
    let dec = png::Decoder::new(std::io::Cursor::new(&png));
    let reader = dec.read_info().unwrap();
    let dims = reader.info().pixel_dims.expect("pHYs chunk");
    assert_eq!(dims.xppu, 11811); // 300 dpi in pixels per metre
    assert_eq!(dims.unit, png::Unit::Meter);
    assert_eq!(reader.info().color_type, png::ColorType::Rgba);
}

#[test]
fn rendering_is_deterministic() {
    let a = render_print(&fixture(), Side::Front, 30.0, 35.0, 60, &EmbeddedFonts, &assets()).unwrap();
    let b = render_print(&fixture(), Side::Front, 30.0, 35.0, 60, &EmbeddedFonts, &assets()).unwrap();
    assert_eq!(a.data(), b.data());
}

#[test]
fn missing_assets_and_bad_sizes_do_not_panic() {
    let empty: HashMap<String, Arc<image::RgbaImage>> = HashMap::new();
    assert!(render_print(&fixture(), Side::Front, 30.0, 35.0, 30, &EmbeddedFonts, &empty).is_ok());
    assert!(render_print(&fixture(), Side::Front, 0.0, 35.0, 300, &EmbeddedFonts, &empty).is_err());
    assert!(render_print(&fixture(), Side::Front, 3000.0, 35.0, 300, &EmbeddedFonts, &empty).is_err());
}
