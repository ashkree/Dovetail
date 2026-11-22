//! The `status` feature's commands.

use serde::{Deserialize, Serialize};

/// The status bar and usage menu: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusCommand {
    /// Read every driver's usage limits for the usage menu (task #46). The answer is
    /// [`super::StatusEvent::UsageLoaded`].
    LoadUsage,
}
