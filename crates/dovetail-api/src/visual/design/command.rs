//! The `design` feature's commands.

use crate::agent::AgentId;
use serde::{Deserialize, Serialize};

/// Design previews and variants: what the UI asks the core to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignCommand {
    /// Start a slice's preview (ADR 0026) unless it already starts or runs. The viewer sends it
    /// when it opens on the slice. Answered with [`super::DesignEvent::PreviewChanged`].
    StartPreview {
        /// The slice.
        agent: AgentId,
    },
    /// Stop a slice's preview and start it again.
    RestartPreview {
        /// The slice.
        agent: AgentId,
    },
    /// Pick an option of a declared variant slot. Refused for an unknown slot or option.
    SelectVariant {
        /// The slice.
        agent: AgentId,
        /// The slot's name.
        slot: String,
        /// The option.
        option: String,
    },
    /// "Keep this combination": mark the picked options as kept and prompt the slice to make
    /// them permanent and delete the rest.
    KeepVariants {
        /// The slice.
        agent: AgentId,
    },
}
