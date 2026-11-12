//! Delivery fee lookup.
//!
//! v1 uses the admin-editable fee table (one fee per region). The trait
//! leaves room for a courier API later: such a provider would receive the
//! table fee as a fallback and may return a live quote instead.

use async_trait::async_trait;
use domain::Tiyin;

#[derive(Debug, thiserror::Error)]
#[error("delivery provider error: {0}")]
pub struct DeliveryError(pub String);

#[async_trait]
pub trait DeliveryQuoter: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    /// Fee for delivering to `region_slug`; `table_fee` is the configured
    /// fee for that region.
    async fn fee(&self, region_slug: &str, table_fee: Tiyin) -> Result<Tiyin, DeliveryError>;
}

/// Returns the fee from the admin table unchanged.
pub struct TableDelivery;

#[async_trait]
impl DeliveryQuoter for TableDelivery {
    fn name(&self) -> &'static str {
        "table"
    }

    async fn fee(&self, _region_slug: &str, table_fee: Tiyin) -> Result<Tiyin, DeliveryError> {
        Ok(table_fee)
    }
}

/// Always charges the same fee; handy in tests and demos.
pub struct FixedDelivery(pub Tiyin);

#[async_trait]
impl DeliveryQuoter for FixedDelivery {
    fn name(&self) -> &'static str {
        "fixed"
    }

    async fn fee(&self, _region_slug: &str, _table_fee: Tiyin) -> Result<Tiyin, DeliveryError> {
        Ok(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn table_and_fixed() {
        assert_eq!(TableDelivery.fee("tashkent", Tiyin(2_500_000)).await.unwrap(), Tiyin(2_500_000));
        assert_eq!(FixedDelivery(Tiyin(100)).fee("x", Tiyin(5)).await.unwrap(), Tiyin(100));
    }
}
