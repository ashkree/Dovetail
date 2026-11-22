//! The `model` feature: the models a session can run and the reasoning effort it runs them at.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::ModelCommand;
pub use event::ModelEvent;

use serde::{Deserialize, Serialize};

/// How hard a session's model thinks: Claude Code's effort levels (`--effort`), lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    /// `low`.
    Low,
    /// `medium`.
    Medium,
    /// `high`.
    High,
    /// `xhigh`.
    Xhigh,
    /// `max`.
    Max,
}

impl ReasoningEffort {
    /// Every level, lowest first.
    pub const ALL: [ReasoningEffort; 5] = [
        ReasoningEffort::Low,
        ReasoningEffort::Medium,
        ReasoningEffort::High,
        ReasoningEffort::Xhigh,
        ReasoningEffort::Max,
    ];

    /// The name Claude Code uses, for `--effort` and the `effortLevel` setting.
    pub fn as_str(self) -> &'static str {
        match self {
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::Xhigh => "xhigh",
            ReasoningEffort::Max => "max",
        }
    }

    /// Parse Claude Code's name.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.as_str() == s)
    }
}

/// The models the selector always offers, before `config.toml`'s `extra_models` (task #39).
pub const BUILTIN_MODELS: [&str; 4] = ["opus", "sonnet", "haiku", "fable"];

/// A model the selector offers (task #39): the built-in aliases, then `config.toml`'s
/// `extra_models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelOption {
    /// The name passed to Claude Code (`opus`, `claude-haiku-4-5-20251001`).
    pub name: String,
    /// The full model name it stands for (`claude-opus-5-5`), once a session has listed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
    /// The effort levels it accepts, once a session has listed it; `None` means not known yet,
    /// so every level is offered. Empty: the model has no effort setting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub efforts: Option<Vec<ReasoningEffort>>,
    /// The driver that runs it (task #46): the picker groups by it, and picking another
    /// driver's model starts a new session.
    #[serde(default)]
    pub driver: DriverKind,
}

impl ModelOption {
    /// A Claude Code option nothing is known about yet.
    pub fn named(name: impl Into<String>) -> Self {
        Self::named_on(DriverKind::Claude, name)
    }

    /// An option on `driver` nothing else is known about yet.
    pub fn named_on(driver: DriverKind, name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            resolved: None,
            efforts: None,
            driver,
        }
    }

    /// Whether the model accepts `effort`.
    pub fn supports(&self, effort: ReasoningEffort) -> bool {
        self.efforts.as_ref().is_none_or(|e| e.contains(&effort))
    }
}

/// The index in `options` of the model a session on `driver` reports running
/// (`claude-opus-5-5`): that driver's option named so or resolving to it, else the first of its
/// aliases the name contains (`opus`). Another driver's options never match: agy also runs a
/// `claude-opus-5-5`.
pub fn model_option_index(
    options: &[ModelOption],
    driver: DriverKind,
    running: &str,
) -> Option<usize> {
    let own = |o: &&ModelOption| o.driver == driver;
    options
        .iter()
        .position(|o| own(&o) && (o.name == running || o.resolved.as_deref() == Some(running)))
        .or_else(|| {
            options
                .iter()
                .position(|o| own(&o) && running.contains(&o.name))
        })
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_option_index_isolates_identical_model_names_by_driver() {
        let options = vec![
            ModelOption {
                name: "opus".into(),
                resolved: Some("claude-opus-5-5".into()),
                efforts: None,
                driver: DriverKind::Claude,
            },
            ModelOption::named_on(DriverKind::Agy, "claude-opus-5-5"),
            ModelOption::named_on(DriverKind::Agy, "gemini-3.8-flash"),
        ];

        assert_eq!(
            model_option_index(&options, DriverKind::Claude, "claude-opus-5-5"),
            Some(0)
        );
        assert_eq!(
            model_option_index(&options, DriverKind::Agy, "claude-opus-5-5"),
            Some(1)
        );
        assert_eq!(
            model_option_index(&options, DriverKind::Agy, "gemini-3.8-flash-high"),
            Some(2)
        );
        assert_eq!(
            model_option_index(&options, DriverKind::Claude, "gemini-3.8-flash"),
            None
        );
    }
}
