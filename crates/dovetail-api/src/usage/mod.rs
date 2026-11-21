//! The `usage` feature: Claude usage limits.

mod apply;
mod event;
pub(crate) use apply::apply;
pub use event::UsageEvent;

use serde::{Deserialize, Serialize};

/// How close the account is to its Claude usage limits. Limits are per account, so this is the
/// latest report from any session. Nothing is known until a session finishes a turn.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageLimits {
    /// The rolling 5-hour window.
    pub five_hour: Option<UsageWindow>,
    /// The weekly (7-day) window.
    pub seven_day: Option<UsageWindow>,
}

/// One usage-limit window, from Claude Code's `rate_limit_event`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageWindow {
    /// How much of the window is used, 0–1 (may exceed 1 when over the limit).
    pub utilization: f64,
    /// When the window resets, in Unix milliseconds like every time in the API; `None` when not
    /// reported.
    pub resets_at_ms: Option<i64>,
    /// Claude Code's status for the window.
    pub status: UsageStatus,
}

/// Claude Code's status word for a usage window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageStatus {
    /// `allowed`: under the limit.
    Allowed,
    /// `allowed_warning`: under the limit, close to it.
    AllowedWarning,
    /// `rejected`: over the limit.
    Rejected,
    /// A word this version does not know (or none): new Claude Code values never break parsing.
    #[serde(other)]
    Unknown,
}

impl UsageStatus {
    /// Parses Claude Code's word; anything unrecognised is [`UsageStatus::Unknown`].
    pub fn from_word(word: &str) -> UsageStatus {
        serde_json::from_value(serde_json::Value::String(word.to_owned()))
            .unwrap_or(UsageStatus::Unknown)
    }
}
