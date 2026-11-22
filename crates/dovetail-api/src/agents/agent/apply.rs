//! How `agent` events change the snapshot.

use super::AgentEvent;
use crate::apply::upsert;
use crate::snapshot::Snapshot;

pub(crate) fn apply(state: &mut Snapshot, event: AgentEvent) {
    match event {
        AgentEvent::Upserted(agent) => upsert(&mut state.agents, agent, |a| a.id.clone()),
        // An agent takes its per-agent state with it.
        AgentEvent::Removed(id) => {
            state.agents.retain(|a| a.id != id);
            state.previews.retain(|p| p.agent != id);
            state.variants.retain(|v| v.agent != id);
            state.conversations.remove(&id);
            state.worktrees.retain(|w| w.agent != id);
            state.commands.remove(&id);
        }
        AgentEvent::TodosReplaced { list, items } => {
            state.todos.retain(|t| t.list != list);
            state.todos.extend(items);
        }
        AgentEvent::CommandsChanged { agent, commands } => {
            state.commands.insert(agent, commands);
        }
    }
}
