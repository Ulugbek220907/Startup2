//! Every third-party integration sits behind a trait defined here, with a
//! mock that works offline. `Config` picks the implementation, so the whole
//! app runs with zero external API keys and tests never touch the network.
//!
//! | Trait | Mock (default) | Real implementation |
//! |---|---|---|
//! | [`tryon::TryOnProvider`] | `MockTryOn` (composited preview) | `FashnTryOn` (FASHN API) |
//! | [`bg::BackgroundRemover`] | `KeyColorRemover` (local, border colour key) | add a hosted matting API |
//! | [`moderation::ImageModerator`] | `MockModerator` | add a hosted moderation API |
//! | [`sms::SmsSender`] | `LogSms` | v2: Eskiz / Play Mobile |
//! | [`delivery::DeliveryQuoter`] | `TableDelivery` (admin fee table) | v2: courier API |
//! | [`messenger::Messenger`] | `LogMessenger` | Telegram (in the server crate) |

pub mod bg;
pub mod delivery;
pub mod messenger;
pub mod moderation;
pub mod sms;
pub mod tryon;
