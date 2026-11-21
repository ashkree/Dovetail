//! Which CLI runs a session (task #46, `docs/spikes/agy-headless.md`).

use serde::{Deserialize, Serialize};

/// The CLI a session runs on. Fixed for the life of a session: switching means a new session.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum DriverKind {
    /// Claude Code (`claude`), over its stream-json protocol.
    #[default]
    Claude,
    /// Antigravity (`agy acp`), over the Agent Client Protocol.
    Agy,
}

impl DriverKind {
    /// Every driver, in the order the pickers show them.
    pub const ALL: [DriverKind; 2] = [DriverKind::Claude, DriverKind::Agy];

    /// The stored and serialized name.
    pub fn as_str(self) -> &'static str {
        match self {
            DriverKind::Claude => "claude",
            DriverKind::Agy => "agy",
        }
    }

    /// Parses [`DriverKind::as_str`]'s name.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.as_str() == s)
    }

    /// The name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            DriverKind::Claude => "Claude Code",
            DriverKind::Agy => "agy",
        }
    }
}

/// One driver's account limits, for the usage menu (task #46). Claude Code's come from its
/// `get_usage` control request, agy's from `agy --output-format json -p=/usage`; neither costs a
/// model call.
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
