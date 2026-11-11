//! The versioned design JSON document.
//!
//! Stored in Postgres as JSONB, in the browser as a draft, and frozen into
//! every order. Bump [`SCHEMA_VERSION`] and extend [`migrate`] whenever the
//! shape changes; old documents must keep loading forever because orders
//! reference them.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::fonts::FontId;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DesignDoc {
    pub version: u32,
    /// Catalog slug of the garment, e.g. `"hoodie"`.
    pub garment: String,
    /// Colour slug within that garment, e.g. `"black"`.
    pub color: String,
    #[serde(default)]
    pub front: Vec<Layer>,
    #[serde(default)]
    pub back: Vec<Layer>,
}

impl DesignDoc {
    pub fn new(garment: impl Into<String>, color: impl Into<String>) -> Self {
        DesignDoc { version: SCHEMA_VERSION, garment: garment.into(), color: color.into(), front: vec![], back: vec![] }
    }

    pub fn layers(&self, side: Side) -> &Vec<Layer> {
        match side {
            Side::Front => &self.front,
            Side::Back => &self.back,
        }
    }

    pub fn layers_mut(&mut self, side: Side) -> &mut Vec<Layer> {
        match side {
            Side::Front => &mut self.front,
            Side::Back => &mut self.back,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.front.is_empty() && self.back.is_empty()
    }

    /// Asset ids of every uploaded image used by the design.
    pub fn asset_ids(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .front
            .iter()
            .chain(self.back.iter())
            .filter_map(|l| match &l.kind {
                LayerKind::Image(i) => Some(i.asset.clone()),
                _ => None,
            })
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// All user-entered text, for the moderation blocklist.
    pub fn all_text(&self) -> String {
        self.front
            .iter()
            .chain(self.back.iter())
            .filter_map(|l| match &l.kind {
                LayerKind::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Upgrade any stored document to the current schema.
///
/// There is only version 1 today, so this validates the version and parses.
/// When version 2 arrives, transform the raw `Value` here step by step
/// (1 -> 2 -> 3 ...) before the final `from_value`.
pub fn migrate(value: serde_json::Value) -> Result<DesignDoc, String> {
    let version = value.get("version").and_then(|v| v.as_u64()).ok_or("design document has no version")?;
    if version == 0 || version > SCHEMA_VERSION as u64 {
        return Err(format!("unsupported design version {version}"));
    }
    serde_json::from_value(value).map_err(|e| format!("invalid design document: {e}"))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Front,
    Back,
}

impl Side {
    pub const ALL: [Side; 2] = [Side::Front, Side::Back];

    pub fn as_str(&self) -> &'static str {
        match self {
            Side::Front => "front",
            Side::Back => "back",
        }
    }

    pub fn parse(s: &str) -> Option<Side> {
        match s {
            "front" => Some(Side::Front),
            "back" => Some(Side::Back),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GarmentKind {
    Tshirt,
    Hoodie,
    Sweatshirt,
}

impl GarmentKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            GarmentKind::Tshirt => "tshirt",
            GarmentKind::Hoodie => "hoodie",
            GarmentKind::Sweatshirt => "sweatshirt",
        }
    }

    pub fn parse(s: &str) -> Option<GarmentKind> {
        match s {
            "tshirt" => Some(GarmentKind::Tshirt),
            "hoodie" => Some(GarmentKind::Hoodie),
            "sweatshirt" => Some(GarmentKind::Sweatshirt),
            _ => None,
        }
    }
}

/// Print finish of a layer. Foil is modelled now so stored documents and
/// order rows do not need a migration when the foil pipeline ships in v2;
/// v1 validation rejects it (see `validate`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finish {
    #[default]
    Standard,
    FoilGold,
    FoilSilver,
}

impl Finish {
    pub fn as_str(&self) -> &'static str {
        match self {
            Finish::Standard => "standard",
            Finish::FoilGold => "foil_gold",
            Finish::FoilSilver => "foil_silver",
        }
    }
}

/// One element of the design. Position is the layer centre in cm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: String,
    pub x: f32,
    pub y: f32,
    /// Degrees, clockwise.
    #[serde(default)]
    pub rotation: f32,
    #[serde(default = "one")]
    pub scale: f32,
    #[serde(default)]
    pub flip_x: bool,
    #[serde(default)]
    pub flip_y: bool,
    #[serde(default)]
    pub finish: Finish,
    // `flatten` + an internally tagged enum gives compact JSON:
    // {"id":"a","x":1,"y":2,"type":"text","text":"Salom",...}
    #[serde(flatten)]
    pub kind: LayerKind,
}

fn one() -> f32 {
    1.0
}

impl Layer {
    pub fn new(id: impl Into<String>, x: f32, y: f32, kind: LayerKind) -> Self {
        Layer { id: id.into(), x, y, rotation: 0.0, scale: 1.0, flip_x: false, flip_y: false, finish: Finish::Standard, kind }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayerKind {
    Text(TextLayer),
    Image(ImageLayer),
    Draw(DrawLayer),
    Shape(ShapeLayer),
    Sticker(StickerLayer),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextLayer {
    pub text: String,
    pub font: FontId,
    /// Font size (em height) in cm.
    pub size_cm: f32,
    pub color: Color,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outline: Option<Outline>,
    /// -100..=100. Positive arches the text upwards (rainbow), negative
    /// downwards (smile); 100 bends a line into a half circle.
    #[serde(default)]
    pub curve: f32,
    #[serde(default)]
    pub align: Align,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outline {
    pub color: Color,
    pub width_cm: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageLayer {
    /// Asset id returned by the upload endpoint.
    pub asset: String,
    pub width_cm: f32,
    pub height_cm: f32,
    /// Pixel size of the uploaded original, used for the DPI warning.
    pub px_width: u32,
    pub px_height: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawLayer {
    pub strokes: Vec<Stroke>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    /// Points in cm relative to the layer centre.
    pub points: Vec<[f32; 2]>,
    pub width_cm: f32,
    pub color: Color,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShapeLayer {
    pub shape: ShapeKind,
    pub width_cm: f32,
    pub height_cm: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Outline>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Rect,
    Ellipse,
    Triangle,
    Star,
    Heart,
}

impl ShapeKind {
    pub const ALL: [ShapeKind; 5] = [ShapeKind::Rect, ShapeKind::Ellipse, ShapeKind::Triangle, ShapeKind::Star, ShapeKind::Heart];
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StickerLayer {
    /// Id from `stickers::STICKERS`.
    pub sticker: String,
    pub width_cm: f32,
    pub color: Color,
}

/// RGBA colour, serialised as `#rrggbb` or `#rrggbbaa`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color(pub [u8; 4]);

impl Color {
    pub const BLACK: Color = Color([0, 0, 0, 255]);
    pub const WHITE: Color = Color([255, 255, 255, 255]);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color([r, g, b, 255])
    }

    pub fn parse(s: &str) -> Option<Color> {
        let h = s.strip_prefix('#')?;
        if !h.is_ascii() {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        match h.len() {
            6 => Some(Color([byte(0)?, byte(2)?, byte(4)?, 255])),
            8 => Some(Color([byte(0)?, byte(2)?, byte(4)?, byte(6)?])),
            _ => None,
        }
    }

    pub fn to_hex(&self) -> String {
        let [r, g, b, a] = self.0;
        if a == 255 {
            format!("#{r:02x}{g:02x}{b:02x}")
        } else {
            format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
        }
    }

    /// CSS `rgba()` string for the HTML canvas.
    pub fn to_css(&self) -> String {
        let [r, g, b, a] = self.0;
        format!("rgba({r},{g},{b},{:.3})", a as f32 / 255.0)
    }

    /// Perceived brightness 0..=255 (ITU-R BT.601 weights, integer math).
    pub fn luma(&self) -> u8 {
        let [r, g, b, _] = self.0;
        ((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000) as u8
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Color::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("invalid colour {s:?}")))
    }
}

/// Print quality of an uploaded image at its current printed size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DpiQuality {
    Good,
    /// Will look soft when printed.
    Low,
    /// Will look visibly pixelated.
    Bad,
}

pub const DPI_GOOD: f32 = 150.0;
pub const DPI_MIN: f32 = 90.0;

/// Effective DPI of an image `px` pixels wide printed `cm` wide.
pub fn effective_dpi(px: u32, cm: f32) -> f32 {
    if cm <= 0.0 {
        return 0.0;
    }
    px as f32 / (cm / 2.54)
}

pub fn dpi_quality(dpi: f32) -> DpiQuality {
    if dpi >= DPI_GOOD {
        DpiQuality::Good
    } else if dpi >= DPI_MIN {
        DpiQuality::Low
    } else {
        DpiQuality::Bad
    }
}

impl ImageLayer {
    /// Lowest DPI over both axes at the given layer scale.
    pub fn dpi(&self, scale: f32) -> f32 {
        effective_dpi(self.px_width, self.width_cm * scale).min(effective_dpi(self.px_height, self.height_cm * scale))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip_and_shape() {
        let mut doc = DesignDoc::new("hoodie", "black");
        doc.front.push(Layer::new(
            "t1",
            15.0,
            10.0,
            LayerKind::Text(TextLayer {
                text: "Oʻzbekiston".into(),
                font: FontId::Montserrat,
                size_cm: 3.0,
                color: Color::WHITE,
                outline: None,
                curve: 0.0,
                align: Align::Center,
            }),
        ));
        let json = serde_json::to_value(&doc).unwrap();
        assert_eq!(json["front"][0]["type"], "text");
        assert_eq!(json["front"][0]["color"], "#ffffff");
        let back: DesignDoc = migrate(json).unwrap();
        assert_eq!(back, doc);
    }

    #[test]
    fn rejects_future_versions() {
        let v = serde_json::json!({"version": 99, "garment": "x", "color": "y"});
        assert!(migrate(v).is_err());
    }

    #[test]
    fn colour_parsing() {
        assert_eq!(Color::parse("#FFdf00"), Some(Color::rgb(255, 223, 0)));
        assert_eq!(Color::parse("#00000080"), Some(Color([0, 0, 0, 128])));
        assert_eq!(Color::parse("red"), None);
        assert_eq!(Color::parse("#ффффф"), None);
        assert_eq!(Color([1, 2, 3, 4]).to_hex(), "#01020304");
    }

    #[test]
    fn dpi_thresholds() {
        // 3000 px across 25.4 cm (10 inch) = 300 dpi.
        assert!((effective_dpi(3000, 25.4) - 300.0).abs() < 0.01);
        assert_eq!(dpi_quality(300.0), DpiQuality::Good);
        assert_eq!(dpi_quality(120.0), DpiQuality::Low);
        assert_eq!(dpi_quality(50.0), DpiQuality::Bad);
    }
}
