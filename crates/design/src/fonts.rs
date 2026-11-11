//! The bundled print fonts.
//!
//! All six are SIL OFL 1.1 licensed with no Reserved Font Name clause, so
//! shipping subsetted/instanced builds under the original names is allowed.
//! Licences are in `crates/design/fonts/OFL-*.txt`; `scripts/build-fonts.sh`
//! documents how the files were produced. Every font covers Uzbek Latin and
//! Cyrillic; the ones without a U+02BB glyph fall back to U+2018, which has
//! the same shape (see `text::glyph_for`).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontId {
    #[default]
    Montserrat,
    Oswald,
    NotoSerif,
    Caveat,
    Pacifico,
    RubikMono,
}

impl FontId {
    pub const ALL: [FontId; 6] =
        [FontId::Montserrat, FontId::Oswald, FontId::NotoSerif, FontId::Caveat, FontId::Pacifico, FontId::RubikMono];

    /// File name under `/fonts/` (web) and `crates/design/fonts/` (server).
    pub fn file_name(&self) -> &'static str {
        match self {
            FontId::Montserrat => "Montserrat-Bold.ttf",
            FontId::Oswald => "Oswald-SemiBold.ttf",
            FontId::NotoSerif => "NotoSerif-Bold.ttf",
            FontId::Caveat => "Caveat-Bold.ttf",
            FontId::Pacifico => "Pacifico-Regular.ttf",
            FontId::RubikMono => "RubikMonoOne-Regular.ttf",
        }
    }

    /// Human-readable family name shown in the font picker.
    pub fn label(&self) -> &'static str {
        match self {
            FontId::Montserrat => "Montserrat",
            FontId::Oswald => "Oswald",
            FontId::NotoSerif => "Noto Serif",
            FontId::Caveat => "Caveat",
            FontId::Pacifico => "Pacifico",
            FontId::RubikMono => "Rubik Mono",
        }
    }
}

/// Where font bytes come from. The browser implements this with fonts it has
/// fetched so far; the server uses [`EmbeddedFonts`].
pub trait FontSource {
    /// Raw TTF bytes, or `None` if the font is not loaded (yet).
    fn font_data(&self, id: FontId) -> Option<&[u8]>;
}

/// Fonts compiled into the binary.
#[cfg(feature = "embed-fonts")]
pub struct EmbeddedFonts;

#[cfg(feature = "embed-fonts")]
impl FontSource for EmbeddedFonts {
    fn font_data(&self, id: FontId) -> Option<&[u8]> {
        // `include_bytes!` embeds the file at compile time; the path is
        // relative to this source file.
        Some(match id {
            FontId::Montserrat => include_bytes!("../fonts/Montserrat-Bold.ttf"),
            FontId::Oswald => include_bytes!("../fonts/Oswald-SemiBold.ttf"),
            FontId::NotoSerif => include_bytes!("../fonts/NotoSerif-Bold.ttf"),
            FontId::Caveat => include_bytes!("../fonts/Caveat-Bold.ttf"),
            FontId::Pacifico => include_bytes!("../fonts/Pacifico-Regular.ttf"),
            FontId::RubikMono => include_bytes!("../fonts/RubikMonoOne-Regular.ttf"),
        })
    }
}

/// A source with no fonts; text layers produce no outlines.
pub struct NoFonts;

impl FontSource for NoFonts {
    fn font_data(&self, _: FontId) -> Option<&[u8]> {
        None
    }
}
