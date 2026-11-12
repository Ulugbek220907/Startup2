//! HTTP handlers, grouped by audience.

pub mod admin;
pub mod ops;
pub mod public;
pub mod tryon;

use domain::api::{AdminOrderRow, OrderPublicDto, StatusEventDto};
use domain::order::{DeliveryMethod, ModerationStatus, OrderStatus};
use domain::pricing::Quote;
use domain::Tiyin;

use crate::error::{AppError, AppResult};

/// Crockford base32 without I, L, O, U: no look-alike characters, so codes
/// survive being read out over the phone.
const CODE_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

pub fn random_code(len: usize) -> AppResult<String> {
    let mut buf = vec![0u8; len];
    getrandom::fill(&mut buf).map_err(AppError::internal)?;
    // 256 is a multiple of 32, so `b & 31` is uniformly distributed.
    Ok(buf.iter().map(|b| CODE_ALPHABET[(b & 31) as usize] as char).collect())
}

pub fn parse_quote(v: &serde_json::Value) -> AppResult<Quote> {
    serde_json::from_value(v.clone()).map_err(AppError::internal)
}

pub fn parse_status(s: &str) -> AppResult<OrderStatus> {
    OrderStatus::parse(s).ok_or_else(|| AppError::internal(format!("unknown order status {s:?} in database")))
}

pub fn public_order(row: &db::orders::OrderRow, log: &[db::orders::StatusLogRow]) -> AppResult<OrderPublicDto> {
    Ok(OrderPublicDto {
        code: row.code.clone(),
        status: parse_status(&row.status)?,
        created_at: db::ts(row.created_at),
        garment_name: row.garment_name.clone(),
        color: row.color_name.clone(),
        size: row.size.clone(),
        quantity: row.quantity as u32,
        delivery_method: DeliveryMethod::parse(&row.delivery_method).unwrap_or(DeliveryMethod::Pickup),
        quote: parse_quote(&row.quote)?,
        timeline: log.iter().filter_map(|l| Some(StatusEventDto { status: OrderStatus::parse(&l.to_status)?, at: db::ts(l.created_at) })).collect(),
    })
}

pub fn admin_row(row: &db::orders::OrderRow) -> AppResult<AdminOrderRow> {
    Ok(AdminOrderRow {
        id: row.id.to_string(),
        code: row.code.clone(),
        status: parse_status(&row.status)?,
        created_at: db::ts(row.created_at),
        phone: row.phone.clone().unwrap_or_default(),
        customer_name: row.customer_name.clone(),
        garment_name: row.garment_name.clone(),
        size: row.size.clone(),
        quantity: row.quantity as u32,
        total: Tiyin(row.total_tiyin),
        moderation: ModerationStatus::parse(&row.moderation_status).unwrap_or(ModerationStatus::Pending),
        files_ready: row.files_status == "ready",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_use_the_unambiguous_alphabet() {
        let c = random_code(200).unwrap();
        assert_eq!(c.len(), 200);
        assert!(c.bytes().all(|b| CODE_ALPHABET.contains(&b)));
        assert!(!c.contains(['I', 'L', 'O', 'U']));
    }
}
