//! The `status` feature: the status bar (totals, warnings) and the Claude usage limits it shows.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::StatusCommand;
pub use event::StatusEvent;

use crate::model::DriverKind;
use serde::{Deserialize, Serialize};

/// Identifies a status-bar warning, so it can be replaced or cleared.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WarningId(pub String);

/// Id of the warning shown while the open folder is not a Dovetail project yet (ADR 0005). The
/// TUI shows it at the right of the status bar, ahead of the usage line.
pub const UNREGISTERED_WARNING: &str = "unregistered";

/// Status-bar totals.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Totals {
    /// Memory across all sessions, in MB.
    pub memory_mb: u32,
    /// Items in Needs you.
    pub needs_you: u32,
    /// Cost across all agents, in API-equivalent US dollars.
    pub cost_usd: f64,
}

/// A status-bar warning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    /// Warning id.
    pub id: WarningId,
    /// What to tell the user.
    pub text: String,
}

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

/// One driver's account limits, for the usage menu (task #46). Claude Code's come from its
/// `get_usage` control request, agy's from `agy --output-format json -p=/usage`; neither costs a
/// model call. Unlike [`UsageLimits`], which is the live figure from a session's own events, this
/// is a report read on demand.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DriverUsage {
    /// The driver they belong to.
    pub driver: DriverKind,
    /// The account's plan (`pro`), when the report names it.
    pub plan: Option<String>,
    /// The limit pools: one for Claude Code, one per agy quota group.
    pub groups: Vec<UsageGroup>,
    /// Anything else worth a line: extra-usage credits, or why the report could not be read.
    pub note: Option<String>,
    /// The report could not be read; `note` says why.
    pub failed: bool,
}

impl DriverUsage {
    /// A report that could not be read, with the reason.
    pub fn failed(driver: DriverKind, why: impl Into<String>) -> Self {
        Self {
            driver,
            plan: None,
            groups: Vec::new(),
            note: Some(why.into()),
            failed: true,
        }
    }
}

/// A set of models that share limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageGroup {
    /// Its name (`Claude and GPT models`).
    pub name: String,
    /// Which models it covers, as the driver words it; empty when it does not say.
    pub models: String,
    /// Its windows, shortest first.
    pub windows: Vec<LimitWindow>,
}

/// One limit window of a [`UsageGroup`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LimitWindow {
    /// Its name (`5-hour`, `Weekly`, `Weekly Opus`).
    pub name: String,
    /// How much is used, 0–1 (may exceed 1 over the limit).
    pub used_fraction: f64,
    /// When it resets, in Unix milliseconds; `None` when not reported.
    pub resets_at_ms: Option<i64>,
}
