//! SMS sending. v1 does not send SMS (orders are confirmed by phone call or
//! Telegram); the trait and mock exist so v2 can add Eskiz/Play Mobile
//! without touching callers.

use std::sync::Mutex;

use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
#[error("sms provider error: {0}")]
pub struct SmsError(pub String);

#[async_trait]
pub trait SmsSender: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    /// `phone` is normalised (`+998XXXXXXXXX`).
    async fn send(&self, phone: &str, text: &str) -> Result<(), SmsError>;
}

/// Logs the message (with the number masked) instead of sending it.
#[derive(Default)]
pub struct LogSms {
    sent: Mutex<Vec<(String, String)>>,
}

impl LogSms {
    pub fn new() -> Self {
        Self::default()
    }

    /// Messages "sent" so far, for tests.
    pub fn sent(&self) -> Vec<(String, String)> {
        self.sent.lock().map(|v| v.clone()).unwrap_or_default()
    }
}

#[async_trait]
impl SmsSender for LogSms {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn send(&self, phone: &str, text: &str) -> Result<(), SmsError> {
        // Never log full phone numbers or message bodies.
        tracing::info!(phone = %domain::phone::mask(phone), chars = text.chars().count(), "mock sms");
        if let Ok(mut v) = self.sent.lock() {
            v.push((phone.to_string(), text.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_records_messages() {
        let s = LogSms::new();
        s.send("+998901234567", "Salom").await.unwrap();
        assert_eq!(s.sent(), vec![("+998901234567".to_string(), "Salom".to_string())]);
    }
}
