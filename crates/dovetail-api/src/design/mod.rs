//! The `design` feature: design previews and variants a slice declares.

mod apply;
mod command;
mod event;
pub(crate) use apply::apply;
pub use command::DesignCommand;
pub use event::DesignEvent;

use crate::agent::AgentId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Most slots a slice may declare in one call.
pub const MAX_SLOTS: usize = 12;

/// Most options one slot may have.
pub const MAX_OPTIONS: usize = 8;

/// The last lines of a failed preview's output that the state carries.
pub const OUTPUT_LINES: usize = 20;

/// Where a slice's preview stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewState {
    /// Not started: nobody opened the viewer on this slice yet, or it was stopped.
    NotStarted,
    /// The recipe has no `preview` key for this slice's repo.
    NoCommand,
    /// The command runs; nothing listens on its port yet.
    Starting,
    /// Something accepts connections on the port.
    Running {
        /// `http://127.0.0.1:<port>/`.
        url: String,
    },
    /// The command could not start, exited, or never listened.
    Failed {
        /// Why, then the last lines it printed.
        output: String,
    },
}

/// One slice's preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preview {
    /// The slice.
    pub agent: AgentId,
    /// Its state.
    pub state: PreviewState,
}

/// How wide a variant reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantLevel {
    /// The whole page: a theme, a layout.
    Global,
    /// One part: the header, the footer.
    Component,
    /// One piece of content: a line of text.
    Content,
}

impl VariantLevel {
    /// The word in the tool's arguments.
    pub fn as_str(self) -> &'static str {
        match self {
            VariantLevel::Global => "global",
            VariantLevel::Component => "component",
            VariantLevel::Content => "content",
        }
    }
}

/// One thing the user can switch, with its options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantSlot {
    /// The slot's key, unique in its list; the page reads it.
    pub name: String,
    /// How wide it reaches.
    pub level: VariantLevel,
    /// The choices, at least one.
    pub options: Vec<String>,
    /// The option kept as the permanent one, when the user has kept one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kept: Option<String>,
}

/// The variants one slice declared, and the user's pick in each.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantSet {
    /// The slice.
    pub agent: AgentId,
    /// Its slots, in the order declared.
    pub slots: Vec<VariantSlot>,
    /// The chosen option of every slot, by slot name.
    pub selection: BTreeMap<String, String>,
}

impl VariantSet {
    /// The pick of `slot`: the stored one, else the kept option, else the first.
    pub fn resolve(slot: &VariantSlot, picked: Option<&String>) -> String {
        picked
            .filter(|p| slot.options.contains(p))
            .or(slot.kept.as_ref().filter(|k| slot.options.contains(k)))
            .or(slot.options.first())
            .cloned()
            .unwrap_or_default()
    }

    /// A set for `slots` that keeps `previous` picks that are still valid options of a slot of
    /// the same name; every other slot starts at its kept option, else its first.
    pub fn declared(
        agent: AgentId,
        slots: Vec<VariantSlot>,
        previous: &BTreeMap<String, String>,
    ) -> Self {
        let selection = slots
            .iter()
            .map(|s| (s.name.clone(), Self::resolve(s, previous.get(&s.name))))
            .collect();
        Self {
            agent,
            slots,
            selection,
        }
    }

    /// The text that asks the slice to make the current selection permanent.
    pub fn keep_prompt(&self) -> String {
        let chosen = self
            .slots
            .iter()
            .map(|s| {
                let pick = self.selection.get(&s.name).cloned().unwrap_or_default();
                format!("{} ({}): {}", s.name, s.level.as_str(), pick)
            })
            .collect::<Vec<_>>()
            .join("; ");
        format!(
            "Keep this combination of variants: {chosen}. Make it permanent in the code, \
             delete the other options and the code that switches them (including the \
             `dovetailVariants` message handling when nothing is left to switch), then call \
             `design_declare_variants` again with the slots that remain, or an empty list."
        )
    }
}
