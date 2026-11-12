//! Catalog, prices and settings.

use design::{GarmentKind, Side};
use domain::api::{AdminPrintAreaDto, ColorDto, GarmentDto, L10n, PrintAreaDto, RegionDto};
use domain::pricing::{AreaTier, PriceTable, QuantityDiscount};
use domain::Tiyin;
use sqlx::{PgConnection, PgExecutor, PgPool};

use crate::Result;

/// Active garments with colours, sizes and print areas.
pub async fn garments(pool: &PgPool) -> Result<Vec<GarmentDto>> {
    let rows = sqlx::query!("SELECT id, slug, kind, name_uz, name_ru, name_en, base_price_tiyin FROM garments WHERE active ORDER BY sort, slug")
        .fetch_all(pool)
        .await?;
    let colors = sqlx::query!("SELECT garment_id, slug, name_uz, name_ru, name_en, hex FROM garment_colors ORDER BY sort, slug").fetch_all(pool).await?;
    let sizes = sqlx::query!("SELECT garment_id, size FROM garment_sizes ORDER BY sort, size").fetch_all(pool).await?;
    let areas = sqlx::query!("SELECT garment_id, side, width_cm, height_cm, mock_x, mock_y, mock_w, mock_h FROM print_areas ORDER BY side DESC")
        .fetch_all(pool)
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for g in rows {
        // Unknown kinds cannot be inserted (CHECK constraint); skip rather
        // than panic if the constraint and the enum ever drift apart.
        let Some(kind) = GarmentKind::parse(&g.kind) else { continue };
        out.push(GarmentDto {
            slug: g.slug,
            kind,
            name: L10n { uz: g.name_uz, ru: g.name_ru, en: g.name_en },
            base_price: Tiyin(g.base_price_tiyin),
            colors: colors
                .iter()
                .filter(|c| c.garment_id == g.id)
                .map(|c| ColorDto {
                    slug: c.slug.clone(),
                    name: L10n { uz: c.name_uz.clone(), ru: c.name_ru.clone(), en: c.name_en.clone() },
                    hex: c.hex.clone(),
                })
                .collect(),
            sizes: sizes.iter().filter(|s| s.garment_id == g.id).map(|s| s.size.clone()).collect(),
            areas: areas
                .iter()
                .filter(|a| a.garment_id == g.id)
                .filter_map(|a| {
                    Some(PrintAreaDto {
                        side: Side::parse(&a.side)?,
                        width_cm: a.width_cm,
                        height_cm: a.height_cm,
                        mock_x: a.mock_x,
                        mock_y: a.mock_y,
                        mock_w: a.mock_w,
                        mock_h: a.mock_h,
                    })
                })
                .collect(),
        });
    }
    Ok(out)
}

pub async fn regions(pool: &PgPool) -> Result<Vec<RegionDto>> {
    let rows = sqlx::query!("SELECT slug, name_uz, name_ru, name_en, delivery_fee_tiyin FROM regions WHERE active ORDER BY sort, slug").fetch_all(pool).await?;
    Ok(rows
        .into_iter()
        .map(|r| RegionDto { slug: r.slug, name: L10n { uz: r.name_uz, ru: r.name_ru, en: r.name_en }, delivery_fee: Tiyin(r.delivery_fee_tiyin) })
        .collect())
}

pub async fn price_table(pool: &PgPool) -> Result<PriceTable> {
    let tiers = sqlx::query!("SELECT max_area_cm2, price_tiyin FROM price_tiers ORDER BY max_area_cm2").fetch_all(pool).await?;
    let discounts = sqlx::query!("SELECT min_qty, percent_bp FROM quantity_discounts ORDER BY min_qty").fetch_all(pool).await?;
    Ok(PriceTable {
        tiers: tiers.into_iter().map(|t| AreaTier { max_area_cm2: t.max_area_cm2 as u32, price: Tiyin(t.price_tiyin) }).collect(),
        discounts: discounts.into_iter().map(|d| QuantityDiscount { min_qty: d.min_qty as u32, percent_bp: d.percent_bp as u32 }).collect(),
    })
}

pub async fn setting(pool: &PgPool, key: &str) -> Result<Option<serde_json::Value>> {
    sqlx::query_scalar!("SELECT value FROM settings WHERE key = $1", key).fetch_optional(pool).await
}

pub async fn set_setting(ex: impl PgExecutor<'_>, key: &str, value: &serde_json::Value) -> Result<()> {
    sqlx::query!("INSERT INTO settings (key, value) VALUES ($1, $2) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value", key, value).execute(ex).await?;
    Ok(())
}

// ------------------------------------------------------- admin editing

pub async fn set_garment_price(ex: impl PgExecutor<'_>, slug: &str, price: Tiyin) -> Result<bool> {
    let r = sqlx::query!("UPDATE garments SET base_price_tiyin = $2 WHERE slug = $1", slug, price.0).execute(ex).await?;
    Ok(r.rows_affected() == 1)
}

pub async fn replace_tiers(conn: &mut PgConnection, tiers: &[AreaTier]) -> Result<()> {
    sqlx::query!("DELETE FROM price_tiers").execute(&mut *conn).await?;
    for t in tiers {
        sqlx::query!("INSERT INTO price_tiers (max_area_cm2, price_tiyin) VALUES ($1, $2)", t.max_area_cm2 as i32, t.price.0).execute(&mut *conn).await?;
    }
    Ok(())
}

pub async fn replace_discounts(conn: &mut PgConnection, discounts: &[QuantityDiscount]) -> Result<()> {
    sqlx::query!("DELETE FROM quantity_discounts").execute(&mut *conn).await?;
    for d in discounts {
        sqlx::query!("INSERT INTO quantity_discounts (min_qty, percent_bp) VALUES ($1, $2)", d.min_qty as i32, d.percent_bp as i32).execute(&mut *conn).await?;
    }
    Ok(())
}

pub async fn set_region_fee(ex: impl PgExecutor<'_>, slug: &str, fee: Tiyin) -> Result<bool> {
    let r = sqlx::query!("UPDATE regions SET delivery_fee_tiyin = $2 WHERE slug = $1", slug, fee.0).execute(ex).await?;
    Ok(r.rows_affected() == 1)
}

pub async fn set_print_area(ex: impl PgExecutor<'_>, a: &AdminPrintAreaDto) -> Result<bool> {
    let r = sqlx::query!(
        "UPDATE print_areas p SET width_cm = $3, height_cm = $4, mock_x = $5, mock_y = $6, mock_w = $7, mock_h = $8
         FROM garments g WHERE g.id = p.garment_id AND g.slug = $1 AND p.side = $2",
        a.garment,
        a.area.side.as_str(),
        a.area.width_cm,
        a.area.height_cm,
        a.area.mock_x,
        a.area.mock_y,
        a.area.mock_w,
        a.area.mock_h
    )
    .execute(ex)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn blocklist(pool: &PgPool) -> Result<Vec<String>> {
    sqlx::query_scalar!("SELECT term FROM blocklist_terms ORDER BY term").fetch_all(pool).await
}

pub async fn replace_blocklist(conn: &mut PgConnection, terms: &[String]) -> Result<()> {
    sqlx::query!("DELETE FROM blocklist_terms").execute(&mut *conn).await?;
    for t in terms {
        sqlx::query!("INSERT INTO blocklist_terms (term) VALUES ($1) ON CONFLICT DO NOTHING", t).execute(&mut *conn).await?;
    }
    Ok(())
}
