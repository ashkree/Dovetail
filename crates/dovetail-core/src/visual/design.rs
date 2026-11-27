//! A slice's declared variants and the user's picks (ADR 0026): the handlers of
//! `design_declare_variants` and of the two commands that pick and keep.
//!
//! The database is the only copy, as for canvases: every handler writes through the DB thread,
//! then announces the stored row.

use dovetail_api::Event;
use dovetail_api::agents::agent::AgentId;
use dovetail_api::visual::design::{DesignEvent, VariantSet};
use dovetail_tools::{SocketResponseBody, design::DeclareVariants};

use crate::service::Core;
use crate::tools::{error, ok};

impl Core {
    /// `design_declare_variants`: replaces the slice's list.
    pub(crate) async fn declare_variants_tool(
        &mut self,
        agent: &AgentId,
        call: DeclareVariants,
    ) -> SocketResponseBody {
        let slots = call.slots.len();
        match self.db.declare_variants(agent.clone(), call.slots).await {
            Ok(Some(set)) => {
                self.emit(Event::Design(DesignEvent::VariantsUpserted(set)));
                ok(format!(
                    "Declared {slots} variant slot(s). The viewer's panel shows them and posts the \
                     user's picks to your page as {{ dovetailVariants: {{ <slot>: <option> }} }}."
                ))
            }
            Ok(None) => {
                // An empty list: the client drops the set with an empty one.
                self.emit(Event::Design(DesignEvent::VariantsUpserted(VariantSet {
                    agent: agent.clone(),
                    slots: Vec::new(),
                    selection: Default::default(),
                })));
                ok("Cleared the declared variants.")
            }
            Err(e) => error(format!("could not store the variants: {e}")),
        }
    }

    /// `DesignCommand::SelectVariant`.
    pub(crate) async fn select_variant(&mut self, agent: AgentId, slot: String, option: String) {
        match self.db.select_variant(agent, slot, option).await {
            Ok(set) => self.emit(Event::Design(DesignEvent::VariantsUpserted(set))),
            Err(e) => self.fail(e),
        }
    }

    /// `DesignCommand::KeepVariants`: marks the picks as kept and asks the slice to make them
    /// permanent.
    pub(crate) async fn keep_variants(&mut self, agent: AgentId) {
        match self.db.keep_variants(agent.clone()).await {
            Ok((before, after)) => {
                self.emit(Event::Design(DesignEvent::VariantsUpserted(after)));
                self.prompt(&agent, keep_prompt(&before), Vec::new(), None)
                    .await;
            }
            Err(e) => self.fail(e),
        }
    }
}

/// The text that asks the slice to make the current selection permanent.
fn keep_prompt(set: &VariantSet) -> String {
    let chosen = set
        .slots
        .iter()
        .map(|s| {
            let pick = set.selection.get(&s.name).cloned().unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use super::*;
    use dovetail_api::visual::design::{VariantLevel, VariantSlot};
    use std::collections::BTreeMap;

    #[test]
    fn keep_prompt_lists_chosen_variants() {
        let set = VariantSet {
            agent: AgentId("slice-1".into()),
            slots: vec![
                VariantSlot {
                    name: "theme".into(),
                    level: VariantLevel::Global,
                    options: vec!["light".into(), "dark".into()],
                    kept: None,
                },
                VariantSlot {
                    name: "header".into(),
                    level: VariantLevel::Component,
                    options: vec!["plain".into(), "bold".into()],
                    kept: None,
                },
            ],
            selection: BTreeMap::from([("theme".into(), "dark".into())]),
        };
        let prompt = keep_prompt(&set);
        assert!(prompt.contains("theme (global): dark; header (component): "));
        assert!(prompt.contains("design_declare_variants"));
    }
}
