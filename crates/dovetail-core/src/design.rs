//! A slice's declared variants and the user's picks (ADR 0026): the handlers of
//! `design_declare_variants` and of the two commands that pick and keep.
//!
//! The database is the only copy, as for canvases: every handler writes through the DB thread,
//! then announces the stored row.

use dovetail_api::socket::SocketResponseBody;
use dovetail_api::tools::DeclareVariants;
use dovetail_api::{AgentId, Event, VariantSet};

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
                self.emit(Event::VariantsUpserted(set));
                ok(format!(
                    "Declared {slots} variant slot(s). The viewer's panel shows them and posts the \
                     user's picks to your page as {{ dovetailVariants: {{ <slot>: <option> }} }}."
                ))
            }
            Ok(None) => {
                // An empty list: the client drops the set with an empty one.
                self.emit(Event::VariantsUpserted(VariantSet {
                    agent: agent.clone(),
                    slots: Vec::new(),
                    selection: Default::default(),
                }));
                ok("Cleared the declared variants.")
            }
            Err(e) => error(format!("could not store the variants: {e}")),
        }
    }

    /// `Command::SelectVariant`.
    pub(crate) async fn select_variant(&mut self, agent: AgentId, slot: String, option: String) {
        match self.db.select_variant(agent, slot, option).await {
            Ok(set) => self.emit(Event::VariantsUpserted(set)),
            Err(e) => self.fail(e),
        }
    }

    /// `Command::KeepVariants`: marks the picks as kept and asks the slice to make them
    /// permanent.
    pub(crate) async fn keep_variants(&mut self, agent: AgentId) {
        match self.db.keep_variants(agent.clone()).await {
            Ok((before, after)) => {
                self.emit(Event::VariantsUpserted(after));
                self.prompt(&agent, before.keep_prompt(), Vec::new(), None)
                    .await;
            }
            Err(e) => self.fail(e),
        }
    }
}
