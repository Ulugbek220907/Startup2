//! Glue between the async server and the CPU-bound renderer.

use std::collections::HashMap;
use std::sync::Arc;

use design::fonts::EmbeddedFonts;
use design::{Color, DesignDoc, GarmentKind, Side};
use domain::api::PrintAreaDto;
use renderer::AreaGeom;
use uuid::Uuid;

use crate::error::{AppError, AppResult};
use crate::state::AppState;

pub type Assets = HashMap<String, Arc<image::RgbaImage>>;

pub fn geom(a: &PrintAreaDto) -> AreaGeom {
    AreaGeom { width_cm: a.width_cm, height_cm: a.height_cm, mock_x: a.mock_x, mock_y: a.mock_y, mock_w: a.mock_w, mock_h: a.mock_h }
}

/// Load and decode every uploaded image a design uses. Missing assets are
/// skipped (the layer is simply not drawn) rather than failing the render.
pub async fn load_assets(state: &AppState, doc: &DesignDoc) -> AppResult<Assets> {
    let ids: Vec<Uuid> = doc.asset_ids().iter().filter_map(|s| Uuid::parse_str(s).ok()).collect();
    let mut out = Assets::new();
    if ids.is_empty() {
        return Ok(out);
    }
    for row in db::designs::assets_by_ids(&state.pool, &ids).await? {
        let Some(bytes) = state.store.get(&row.storage_key).await? else { continue };
        // Stored originals were written by us as PNG, but decode with the
        // same limits as uploads anyway.
        if let Ok(img) = crate::uploads::decode(bytes).await {
            out.insert(row.id.to_string(), Arc::new(img.to_rgba8()));
        }
    }
    Ok(out)
}

/// Render a garment mockup PNG on a blocking thread, holding the render gate.
pub async fn mockup_png(
    state: &AppState,
    doc: Arc<DesignDoc>,
    assets: Arc<Assets>,
    side: Side,
    kind: GarmentKind,
    color: Color,
    area: AreaGeom,
    width: u32,
) -> AppResult<Vec<u8>> {
    // The permit is released when `_permit` goes out of scope.
    let _permit = state.render_gate.acquire().await.map_err(AppError::internal)?;
    let png = tokio::task::spawn_blocking(move || {
        let pm = renderer::render_mockup(&doc, side, kind, color, &area, width, &EmbeddedFonts, &*assets)?;
        renderer::encode_png(&pm, None)
    })
    .await??;
    Ok(png)
}

/// Printed area per side in whole cm² (rounded up), for pricing.
pub fn side_areas_cm2(doc: &DesignDoc, areas: &[PrintAreaDto]) -> Vec<u32> {
    Side::ALL
        .iter()
        .map(|side| match areas.iter().find(|a| a.side == *side) {
            Some(a) => design::display::printed_area_cm2(doc.layers(*side), a.width_cm, a.height_cm, &EmbeddedFonts).ceil() as u32,
            // A side without a print area cannot be printed at all.
            None => 0,
        })
        .collect()
}
