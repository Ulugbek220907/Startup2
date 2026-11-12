//! Chat notifications (admin alerts, customer status updates).
//!
//! The Telegram implementation lives in the server crate next to the bot;
//! this mock records messages so tests can assert on them and the app runs
//! without a bot token.

use std::sync::Mutex;

use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
#[error("messenger error: {0}")]
pub struct MessengerError(pub String);

#[async_trait]
pub trait Messenger: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    /// Send plain text to a Telegram chat id.
    async fn send(&self, chat_id: i64, text: &str) -> Result<(), MessengerError>;
}

#[derive(Default)]
pub struct LogMessenger {
    sent: Mutex<Vec<(i64, String)>>,
}

impl LogMessenger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn sent(&self) -> Vec<(i64, String)> {
        self.sent.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl Messenger for LogMessenger {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn send(&self, chat_id: i64, text: &str) -> Result<(), MessengerError> {
        // Message bodies contain order details; log only their size.
        tracing::info!(chat_id, chars = text.chars().count(), "mock telegram message");
        if let Ok(mut v) = self.sent.lock() {
            v.push((chat_id, text.to_string()));
        }
        Ok(())
    }
}
