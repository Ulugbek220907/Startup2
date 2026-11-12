//! Image moderation.
//!
//! A verdict never rejects an order by itself: flagged designs go to the
//! admin's manual review queue before production.

use std::collections::HashSet;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub flagged: bool,
    pub reason: Option<String>,
}

impl Verdict {
    pub fn clean() -> Self {
        Verdict { flagged: false, reason: None }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("moderation provider error: {0}")]
pub struct ModerationError(pub String);

#[async_trait]
pub trait ImageModerator: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    /// `image` is the encoded file (PNG/JPEG/WebP).
    async fn check(&self, image: &[u8]) -> Result<Verdict, ModerationError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MockMode {
    /// Everything passes unless its hash is listed.
    AllowAll,
    /// Everything is flagged (to demo the review queue).
    FlagAll,
}

/// Offline moderator. Flags images whose SHA-256 is in a configured list
/// (so a specific test image can be made to fail deterministically) or, in
/// `FlagAll` mode, every image.
pub struct MockModerator {
    mode: MockMode,
    flagged_sha256: HashSet<String>,
}

impl MockModerator {
    pub fn new(mode: MockMode, flagged_sha256: impl IntoIterator<Item = String>) -> Self {
        MockModerator { mode, flagged_sha256: flagged_sha256.into_iter().map(|s| s.to_ascii_lowercase()).collect() }
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

#[async_trait]
impl ImageModerator for MockModerator {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn check(&self, image: &[u8]) -> Result<Verdict, ModerationError> {
        if self.mode == MockMode::FlagAll {
            return Ok(Verdict { flagged: true, reason: Some("mock: flag_all mode".into()) });
        }
        if self.flagged_sha256.contains(&sha256_hex(image)) {
            return Ok(Verdict { flagged: true, reason: Some("mock: image hash is on the flag list".into()) });
        }
        Ok(Verdict::clean())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_modes() {
        let bad = b"bad image bytes";
        let m = MockModerator::new(MockMode::AllowAll, [sha256_hex(bad).to_uppercase()]);
        assert!(m.check(bad).await.unwrap().flagged);
        assert!(!m.check(b"fine").await.unwrap().flagged);
        let all = MockModerator::new(MockMode::FlagAll, []);
        assert!(all.check(b"fine").await.unwrap().flagged);
    }
}
