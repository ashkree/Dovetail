//! Tools of the `design` suite: variant declarations for a slice's previewed page.

use dovetail_api::design::{MAX_OPTIONS, MAX_SLOTS, VariantSlot};
use serde::{Deserialize, Serialize};

use crate::catalog::Role;
use crate::error::ToolError;

/// Tools in the `design` suite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignTool {
    /// `design_declare_variants` (ADR 0026): the slots the viewer's variant panel shows.
    DeclareVariants(DeclareVariants),
}

impl DesignTool {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            DesignTool::DeclareVariants(_) => "design_declare_variants",
        }
    }

    pub(crate) fn validate(&self, role: Role) -> Result<(), ToolError> {
        match self {
            DesignTool::DeclareVariants(d) => {
                if role != Role::Slice {
                    return Err(ToolError::DeclareFromNonSlice);
                }
                d.check().map_err(|p| ToolError::BadVariants(p.join("; ")))
            }
        }
    }
}

/// `design_declare_variants` (ADR 0026): replace the list of things the user can switch in the
/// slice's preview. Call it again whenever the list changes, and with `slots: []` when none is
/// left.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclareVariants {
    /// Every slot, in the order the panel shows them.
    pub slots: Vec<VariantSlot>,
}

impl DeclareVariants {
    /// Checks the whole list, so every problem comes back at once: at most
    /// [`MAX_SLOTS`] slots with unique non-empty names, each with 1 to
    /// [`MAX_OPTIONS`] unique non-empty options, and a `kept` that is one of them.
    pub fn check(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if self.slots.len() > MAX_SLOTS {
            problems.push(format!("at most {} slots", MAX_SLOTS));
        }
        let mut seen = std::collections::BTreeSet::new();
        for slot in &self.slots {
            let name = slot.name.trim();
            if name.is_empty() {
                problems.push("a slot has an empty name".to_owned());
            } else if !seen.insert(name) {
                problems.push(format!("slot `{name}` appears twice"));
            }
            if slot.options.is_empty() || slot.options.len() > MAX_OPTIONS {
                problems.push(format!("slot `{name}` needs 1 to {} options", MAX_OPTIONS));
            }
            let mut options = std::collections::BTreeSet::new();
            for o in &slot.options {
                if o.trim().is_empty() {
                    problems.push(format!("slot `{name}` has an empty option"));
                } else if !options.insert(o.as_str()) {
                    problems.push(format!("slot `{name}` has the option `{o}` twice"));
                }
            }
            if let Some(kept) = &slot.kept
                && !slot.options.contains(kept)
            {
                problems.push(format!("slot `{name}`: `kept` is not one of its options"));
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}
