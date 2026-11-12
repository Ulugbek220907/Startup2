//! Order lifecycle.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    New,
    Confirmed,
    InProduction,
    Ready,
    Delivered,
    Cancelled,
}

impl OrderStatus {
    pub const ALL: [OrderStatus; 6] =
        [OrderStatus::New, OrderStatus::Confirmed, OrderStatus::InProduction, OrderStatus::Ready, OrderStatus::Delivered, OrderStatus::Cancelled];

    /// The happy path, in order, for the customer's progress view.
    pub const FLOW: [OrderStatus; 5] = [OrderStatus::New, OrderStatus::Confirmed, OrderStatus::InProduction, OrderStatus::Ready, OrderStatus::Delivered];

    pub fn as_str(&self) -> &'static str {
        match self {
            OrderStatus::New => "new",
            OrderStatus::Confirmed => "confirmed",
            OrderStatus::InProduction => "in_production",
            OrderStatus::Ready => "ready",
            OrderStatus::Delivered => "delivered",
            OrderStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<OrderStatus> {
        Self::ALL.iter().copied().find(|x| x.as_str() == s)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, OrderStatus::Delivered | OrderStatus::Cancelled)
    }

    /// Translation key for the status name.
    pub fn i18n_key(&self) -> &'static str {
        match self {
            OrderStatus::New => "status.new",
            OrderStatus::Confirmed => "status.confirmed",
            OrderStatus::InProduction => "status.in_production",
            OrderStatus::Ready => "status.ready",
            OrderStatus::Delivered => "status.delivered",
            OrderStatus::Cancelled => "status.cancelled",
        }
    }

    /// Statuses an admin may move this order to. Orders only move forward
    /// one step at a time, or get cancelled before delivery.
    pub fn next_allowed(&self) -> &'static [OrderStatus] {
        match self {
            OrderStatus::New => &[OrderStatus::Confirmed, OrderStatus::Cancelled],
            OrderStatus::Confirmed => &[OrderStatus::InProduction, OrderStatus::Cancelled],
            OrderStatus::InProduction => &[OrderStatus::Ready, OrderStatus::Cancelled],
            OrderStatus::Ready => &[OrderStatus::Delivered, OrderStatus::Cancelled],
            OrderStatus::Delivered | OrderStatus::Cancelled => &[],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationStatus {
    /// Not checked yet.
    Pending,
    /// Automatic checks found nothing.
    Clean,
    /// Automatic checks or a report flagged it; a human must decide.
    Flagged,
    /// A human approved it.
    Approved,
    /// A human rejected it; it must not be produced.
    Rejected,
}

impl ModerationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModerationStatus::Pending => "pending",
            ModerationStatus::Clean => "clean",
            ModerationStatus::Flagged => "flagged",
            ModerationStatus::Approved => "approved",
            ModerationStatus::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Self::Pending, Self::Clean, Self::Flagged, Self::Approved, Self::Rejected].into_iter().find(|x| x.as_str() == s)
    }

    /// Only checked designs may go to the printer.
    pub fn allows_production(&self) -> bool {
        matches!(self, ModerationStatus::Clean | ModerationStatus::Approved)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionError {
    NotAllowed {
        from: OrderStatus,
        to: OrderStatus,
    },
    /// The design has not passed moderation, so it cannot be produced.
    NeedsModeration,
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransitionError::NotAllowed { from, to } => write!(f, "cannot move an order from {} to {}", from.as_str(), to.as_str()),
            TransitionError::NeedsModeration => f.write_str("the design must pass moderation before production"),
        }
    }
}

impl std::error::Error for TransitionError {}

/// Check a status change. Pure, so it is easy to test exhaustively; the
/// database update re-checks the current status in its WHERE clause.
pub fn check_transition(from: OrderStatus, to: OrderStatus, moderation: ModerationStatus) -> Result<(), TransitionError> {
    if !from.next_allowed().contains(&to) {
        return Err(TransitionError::NotAllowed { from, to });
    }
    if to == OrderStatus::InProduction && !moderation.allows_production() {
        return Err(TransitionError::NeedsModeration);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMethod {
    Delivery,
    Pickup,
}

impl DeliveryMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeliveryMethod::Delivery => "delivery",
            DeliveryMethod::Pickup => "pickup",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "delivery" => Some(DeliveryMethod::Delivery),
            "pickup" => Some(DeliveryMethod::Pickup),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn happy_path_is_allowed_step_by_step() {
        for w in OrderStatus::FLOW.windows(2) {
            assert!(check_transition(w[0], w[1], ModerationStatus::Clean).is_ok());
        }
    }

    #[test]
    fn exhaustive_transition_table() {
        for from in OrderStatus::ALL {
            for to in OrderStatus::ALL {
                let ok = check_transition(from, to, ModerationStatus::Approved).is_ok();
                let forward = OrderStatus::FLOW.iter().position(|s| *s == from).zip(OrderStatus::FLOW.iter().position(|s| *s == to));
                let expect = match (from, to) {
                    (f, _) if f.is_terminal() => false,
                    (_, OrderStatus::Cancelled) => true,
                    _ => matches!(forward, Some((a, b)) if b == a + 1),
                };
                assert_eq!(ok, expect, "{from:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn production_requires_moderation() {
        for m in [ModerationStatus::Pending, ModerationStatus::Flagged, ModerationStatus::Rejected] {
            assert_eq!(check_transition(OrderStatus::Confirmed, OrderStatus::InProduction, m), Err(TransitionError::NeedsModeration));
            // Confirming or cancelling is still possible.
            assert!(check_transition(OrderStatus::New, OrderStatus::Confirmed, m).is_ok());
            assert!(check_transition(OrderStatus::Confirmed, OrderStatus::Cancelled, m).is_ok());
        }
    }

    #[test]
    fn string_round_trip() {
        for s in OrderStatus::ALL {
            assert_eq!(OrderStatus::parse(s.as_str()), Some(s));
            assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{}\"", s.as_str()));
        }
        assert_eq!(OrderStatus::parse("shipped"), None);
    }
}
