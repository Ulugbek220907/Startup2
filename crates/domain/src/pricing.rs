//! Price calculation. One pure function, no I/O, no floats.
//!
//! The server calls [`quote`] with prices loaded from the database and never
//! accepts a price from the client. The browser may call the same function
//! to show an instant estimate, but the order is always re-priced on the
//! server.
//!
//! Formula:
//! ```text
//! unit      = garment base + sum over printed sides of tier(area of that side)
//! subtotal  = unit * quantity
//! discount  = floor_to_whole_soʻm(subtotal * best quantity discount)
//! total     = subtotal - discount + delivery fee
//! ```

use serde::{Deserialize, Serialize};

use crate::money::{MoneyError, Tiyin};

pub const MAX_QUANTITY: u32 = 500;

/// Print price for designs whose bounding box is at most `max_area_cm2`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AreaTier {
    pub max_area_cm2: u32,
    pub price: Tiyin,
}

/// `percent_bp` basis points off (100 bp = 1 %) from `min_qty` pieces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuantityDiscount {
    pub min_qty: u32,
    pub percent_bp: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceTable {
    pub tiers: Vec<AreaTier>,
    pub discounts: Vec<QuantityDiscount>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuoteInput {
    pub garment_base: Tiyin,
    /// Printed area of each side in cm², rounded up; 0 = nothing printed.
    pub side_areas_cm2: Vec<u32>,
    pub quantity: u32,
    /// 0 for pickup.
    pub delivery_fee: Tiyin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    pub unit_garment: Tiyin,
    pub unit_print: Tiyin,
    pub unit_price: Tiyin,
    pub quantity: u32,
    pub subtotal: Tiyin,
    pub discount_bp: u32,
    pub discount: Tiyin,
    pub delivery_fee: Tiyin,
    pub total: Tiyin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PricingError {
    InvalidQuantity,
    /// Nothing is printed on any side.
    EmptyDesign,
    /// The admin has not configured any print price tiers.
    NoTiers,
    InvalidTable,
    Money(MoneyError),
}

impl std::fmt::Display for PricingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PricingError::InvalidQuantity => write!(f, "quantity must be between 1 and {MAX_QUANTITY}"),
            PricingError::EmptyDesign => f.write_str("the design is empty"),
            PricingError::NoTiers => f.write_str("print prices are not configured"),
            PricingError::InvalidTable => f.write_str("price table contains invalid values"),
            PricingError::Money(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for PricingError {}

// `From` lets `?` convert a MoneyError into a PricingError automatically.
impl From<MoneyError> for PricingError {
    fn from(e: MoneyError) -> Self {
        PricingError::Money(e)
    }
}

impl PriceTable {
    /// Print price for one side: the smallest tier that fits the area. Areas
    /// larger than every tier cost the largest tier (the print area itself
    /// caps how big a design can physically be).
    pub fn print_price(&self, area_cm2: u32) -> Result<Tiyin, PricingError> {
        if area_cm2 == 0 {
            return Ok(Tiyin::ZERO);
        }
        let fitting = self.tiers.iter().filter(|t| t.max_area_cm2 >= area_cm2).min_by_key(|t| t.max_area_cm2);
        let tier = match fitting {
            Some(t) => t,
            None => self.tiers.iter().max_by_key(|t| t.max_area_cm2).ok_or(PricingError::NoTiers)?,
        };
        Ok(tier.price)
    }

    /// Best discount the quantity qualifies for, in basis points.
    pub fn discount_bp(&self, quantity: u32) -> u32 {
        self.discounts.iter().filter(|d| d.min_qty <= quantity).map(|d| d.percent_bp).max().unwrap_or(0)
    }

    pub fn validate(&self) -> Result<(), PricingError> {
        let bad_tier = self.tiers.iter().any(|t| t.price.0 < 0 || t.max_area_cm2 == 0);
        let bad_discount = self.discounts.iter().any(|d| d.percent_bp > 10_000 || d.min_qty == 0);
        if bad_tier || bad_discount {
            return Err(PricingError::InvalidTable);
        }
        Ok(())
    }
}

/// Price an order line. See the module docs for the formula.
pub fn quote(input: &QuoteInput, table: &PriceTable) -> Result<Quote, PricingError> {
    if input.quantity == 0 || input.quantity > MAX_QUANTITY {
        return Err(PricingError::InvalidQuantity);
    }
    table.validate()?;
    input.garment_base.require_non_negative()?;
    input.delivery_fee.require_non_negative()?;
    if input.side_areas_cm2.iter().all(|a| *a == 0) {
        return Err(PricingError::EmptyDesign);
    }

    let mut unit_print = Tiyin::ZERO;
    for area in &input.side_areas_cm2 {
        unit_print = unit_print.checked_add(table.print_price(*area)?)?;
    }
    let unit_price = input.garment_base.checked_add(unit_print)?;
    let subtotal = unit_price.checked_mul(input.quantity as i64)?;
    let discount_bp = table.discount_bp(input.quantity);
    let discount = subtotal.basis_points(discount_bp).floor_to_sum();
    let total = subtotal.checked_sub(discount)?.checked_add(input.delivery_fee)?;

    Ok(Quote {
        unit_garment: input.garment_base,
        unit_print,
        unit_price,
        quantity: input.quantity,
        subtotal,
        discount_bp,
        discount,
        delivery_fee: input.delivery_fee,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sum(v: i64) -> Tiyin {
        Tiyin::from_sum(v).unwrap()
    }

    fn table() -> PriceTable {
        PriceTable {
            tiers: vec![
                AreaTier { max_area_cm2: 100, price: sum(20_000) },
                AreaTier { max_area_cm2: 400, price: sum(35_000) },
                AreaTier { max_area_cm2: 1200, price: sum(60_000) },
            ],
            discounts: vec![QuantityDiscount { min_qty: 5, percent_bp: 500 }, QuantityDiscount { min_qty: 20, percent_bp: 1500 }],
        }
    }

    #[test]
    fn worked_example() {
        // Hoodie 260 000, front 350 cm² (35 000) + back 80 cm² (20 000),
        // 6 pieces (5 % off), delivery 25 000.
        let q = quote(&QuoteInput { garment_base: sum(260_000), side_areas_cm2: vec![350, 80], quantity: 6, delivery_fee: sum(25_000) }, &table()).unwrap();
        assert_eq!(q.unit_print, sum(55_000));
        assert_eq!(q.unit_price, sum(315_000));
        assert_eq!(q.subtotal, sum(1_890_000));
        assert_eq!(q.discount_bp, 500);
        assert_eq!(q.discount, sum(94_500));
        assert_eq!(q.total, sum(1_820_500));
    }

    #[test]
    fn tier_selection() {
        let t = table();
        assert_eq!(t.print_price(0), Ok(Tiyin::ZERO));
        assert_eq!(t.print_price(1), Ok(sum(20_000)));
        assert_eq!(t.print_price(100), Ok(sum(20_000)));
        assert_eq!(t.print_price(101), Ok(sum(35_000)));
        assert_eq!(t.print_price(5000), Ok(sum(60_000)));
        assert_eq!(PriceTable::default().print_price(10), Err(PricingError::NoTiers));
    }

    #[test]
    fn rejects_invalid_input() {
        let base = QuoteInput { garment_base: sum(100_000), side_areas_cm2: vec![50], quantity: 1, delivery_fee: Tiyin::ZERO };
        assert_eq!(quote(&QuoteInput { quantity: 0, ..base.clone() }, &table()), Err(PricingError::InvalidQuantity));
        assert_eq!(quote(&QuoteInput { quantity: 501, ..base.clone() }, &table()), Err(PricingError::InvalidQuantity));
        assert_eq!(quote(&QuoteInput { side_areas_cm2: vec![0, 0], ..base.clone() }, &table()), Err(PricingError::EmptyDesign));
        assert_eq!(quote(&QuoteInput { garment_base: Tiyin(-1), ..base.clone() }, &table()), Err(PricingError::Money(MoneyError::Negative)));
        assert_eq!(quote(&QuoteInput { garment_base: Tiyin(i64::MAX), ..base.clone() }, &table()), Err(PricingError::Money(MoneyError::Overflow)));
        let bad = PriceTable { discounts: vec![QuantityDiscount { min_qty: 1, percent_bp: 10_001 }], ..table() };
        assert_eq!(quote(&base, &bad), Err(PricingError::InvalidTable));
    }

    // proptest generates hundreds of random inputs per test and shrinks any
    // failure to a minimal counter-example.
    fn arb_table() -> impl Strategy<Value = PriceTable> {
        (prop::collection::vec((1u32..5000, 0i64..100_000_000), 1..6), prop::collection::vec((1u32..600, 0u32..=10_000), 0..5)).prop_map(
            |(tiers, discounts)| PriceTable {
                tiers: tiers.into_iter().map(|(a, p)| AreaTier { max_area_cm2: a, price: Tiyin(p) }).collect(),
                discounts: discounts.into_iter().map(|(q, bp)| QuantityDiscount { min_qty: q, percent_bp: bp }).collect(),
            },
        )
    }

    fn arb_input() -> impl Strategy<Value = QuoteInput> {
        (0i64..1_000_000_000, prop::collection::vec(0u32..6000, 1..3), 1u32..=MAX_QUANTITY, 0i64..100_000_000)
            .prop_map(|(base, areas, quantity, fee)| QuoteInput { garment_base: Tiyin(base), side_areas_cm2: areas, quantity, delivery_fee: Tiyin(fee) })
    }

    proptest! {
        #[test]
        fn totals_add_up(input in arb_input(), table in arb_table()) {
            match quote(&input, &table) {
                Ok(q) => {
                    prop_assert_eq!(q.unit_price.0, q.unit_garment.0 + q.unit_print.0);
                    prop_assert_eq!(q.subtotal.0, q.unit_price.0 * q.quantity as i64);
                    prop_assert_eq!(q.total.0, q.subtotal.0 - q.discount.0 + q.delivery_fee.0);
                    // A discount never exceeds the goods and never goes negative.
                    prop_assert!(q.discount.0 >= 0 && q.discount.0 <= q.subtotal.0);
                    // The customer never pays less than the delivery fee.
                    prop_assert!(q.total.0 >= q.delivery_fee.0);
                    // Discounts are whole soʻm.
                    prop_assert_eq!(q.discount.0 % 100, 0);
                }
                Err(e) => prop_assert_eq!(e, PricingError::EmptyDesign),
            }
        }

        #[test]
        fn delivery_fee_is_added_exactly(input in arb_input(), table in arb_table(), extra in 0i64..10_000_000) {
            let with = QuoteInput { delivery_fee: Tiyin(input.delivery_fee.0 + extra), ..input.clone() };
            if let (Ok(a), Ok(b)) = (quote(&input, &table), quote(&with, &table)) {
                prop_assert_eq!(b.total.0 - a.total.0, extra);
                prop_assert_eq!(a.subtotal, b.subtotal);
                prop_assert_eq!(a.discount, b.discount);
            }
        }

        #[test]
        fn bigger_print_never_costs_less_with_monotone_tiers(a in 1u32..3000, b in 1u32..3000) {
            // Holds for sane tables where larger tiers cost more.
            let t = table();
            let (lo, hi) = (a.min(b), a.max(b));
            prop_assert!(t.print_price(lo).unwrap() <= t.print_price(hi).unwrap());
        }

        #[test]
        fn effective_unit_price_never_rises_with_quantity(q1 in 1u32..=MAX_QUANTITY, q2 in 1u32..=MAX_QUANTITY, base in 0i64..50_000_000) {
            // With discounts that grow with quantity, buying more is never
            // more expensive per piece (allowing 1 soʻm of rounding).
            let (lo, hi) = (q1.min(q2), q1.max(q2));
            let mk = |quantity| QuoteInput { garment_base: Tiyin(base), side_areas_cm2: vec![200], quantity, delivery_fee: Tiyin::ZERO };
            let a = quote(&mk(lo), &table()).unwrap();
            let b = quote(&mk(hi), &table()).unwrap();
            // Compare a.total/lo >= b.total/hi without dividing.
            prop_assert!(a.total.0 as i128 * hi as i128 + 100 * lo as i128 * hi as i128 >= b.total.0 as i128 * lo as i128);
        }

        #[test]
        fn quote_is_deterministic(input in arb_input(), table in arb_table()) {
            prop_assert_eq!(quote(&input, &table), quote(&input, &table));
        }
    }
}
