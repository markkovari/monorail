//! Identifier newtypes used across the system.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Stable identifier for one erg/Pi pairing. Appears in NATS subjects
/// (ADR 0004), so it must stay token-safe: lowercase alphanumerics and
/// dashes only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RowerId(String);

impl RowerId {
    /// Returns `None` when the id contains characters that are not valid in
    /// a NATS subject token.
    pub fn new(id: impl Into<String>) -> Option<Self> {
        let id = id.into();
        let valid = !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        valid.then_some(Self(id))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RowerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One workout session, minted by the publisher when a workout is detected.
/// Combined with [`crate::wire::Envelope::seq`] it forms the system-wide
/// idempotency key (ADRs 0004/0005/0006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// One generated workout plan (ADR 0009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlanId(pub Uuid);

impl fmt::Display for PlanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rower_id_rejects_subject_unsafe_characters() {
        assert!(RowerId::new("erg-1").is_some());
        assert!(RowerId::new("").is_none());
        assert!(RowerId::new("Erg 1").is_none());
        assert!(RowerId::new("erg.1").is_none());
        assert!(RowerId::new("erg>*").is_none());
    }
}
