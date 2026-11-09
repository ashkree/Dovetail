//! Context fill (decision #22, M13c): how full a session's context window is.
//!
//! Every main-thread assistant line carries the API call's `usage`; the tokens the conversation
//! holds after the call are its input, cache read and cache creation tokens plus its output
//! ([`dovetail_driver::TokenUsage::context_tokens`]). The latest call wins, so a compaction shows
//! at once. The window comes from the `contextWindow` a `result` reports for the model, kept per
//! model for the app's run so later sessions of that model know it from their first call; before
//! any result, from [`table_window`]. Subagents run their own context and get no fill.

use dovetail_api::AgentId;
use dovetail_driver::{TokenUsage, TurnResult};

use crate::service::Core;

/// The window assumed for a model the stream has not reported and the table does not know.
/// The smallest current window, so an unknown model reads too full rather than too empty.
pub(crate) const DEFAULT_WINDOW: u64 = 200_000;

/// 1M-token windows, by model id prefix (Claude API model table, 2026-10-06). Everything else,
/// Haiku 4.5 included, is 200K.
const MILLION: &[&str] = &[
    "claude-fable-5",
    "claude-mythos-5",
    "claude-opus-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-sonnet-5",
    "claude-sonnet-4-6",
    "claude-haiku-5",
];

/// The window of `model` as far as the table knows. A `[1m]` suffix (Claude Code's 1M opt-in)
/// means 1M whatever the model.
pub(crate) fn table_window(model: Option<&str>) -> u64 {
    match model {
        Some(m) if m.ends_with("[1m]") || MILLION.iter().any(|p| m.starts_with(p)) => 1_000_000,
        _ => DEFAULT_WINDOW,
    }
}

/// Percent of `window` that `tokens` fill, rounded to the nearest point and capped at 100.
/// A zero window reads as full.
pub(crate) fn fill_pct(tokens: u64, window: u64) -> u8 {
    if window == 0 {
        return 100;
    }
    let pct = (u128::from(tokens) * 100 + u128::from(window) / 2) / u128::from(window);
    u8::try_from(pct.min(100)).unwrap_or(100)
}

/// The window a `result` reports for `model`: its own `modelUsage` entry, else the only entry
/// with a window (a result names the session's model by the same id the session reports).
fn reported_window(result: &TurnResult, model: Option<&str>) -> Option<(String, u64)> {
    let with_window = || result.model_usage.iter().filter(|m| m.context_window.is_some());
    let entry = match model {
        Some(model) => with_window().find(|m| m.model == model),
        None => None,
    }
    .or_else(|| {
        let mut all = with_window();
        let first = all.next();
        first.filter(|_| all.next().is_none())
    })?;
    Some((entry.model.clone(), entry.context_window?))
}

impl Core {
    /// A main-thread assistant line's usage: the session's context now holds this many tokens.
    pub(crate) fn on_context_usage(&mut self, agent: &AgentId, usage: TokenUsage) {
        let Some(live) = self.live.get_mut(agent) else {
            return;
        };
        live.context_tokens = Some(usage.context_tokens());
        self.refresh_context(agent);
    }

    /// A `result`: learn the model's window from it.
    pub(crate) fn learn_context_window(&mut self, agent: &AgentId, result: &TurnResult) {
        let model = self.agents.get(agent).and_then(|r| r.agent.model.clone());
        if let Some((model, window)) = reported_window(result, model.as_deref()) {
            self.context_windows.insert(model, window);
            self.refresh_context(agent);
        }
    }

    /// The window of the agent's model: reported by the stream, else from the table.
    fn context_window(&self, agent: &AgentId) -> u64 {
        let model = self.agents.get(agent).and_then(|r| r.agent.model.as_deref());
        model
            .and_then(|m| self.context_windows.get(m).copied())
            .unwrap_or_else(|| table_window(model))
    }

    /// Recomputes the agent's fill from its latest usage; no change, no event.
    fn refresh_context(&mut self, agent: &AgentId) {
        let Some(tokens) = self.live.get(agent).and_then(|l| l.context_tokens) else {
            return;
        };
        let pct = fill_pct(tokens, self.context_window(agent));
        self.update_agent(agent, |row| row.agent.context_pct = Some(pct));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dovetail_driver::DriverEvent;
    use serde_json::Value;

    fn events(fixture: &str) -> Vec<DriverEvent> {
        fixture
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|l| l.get("dir").and_then(Value::as_str) != Some("in"))
            .filter_map(|l| l.get("msg").cloned())
            .flat_map(dovetail_driver::parse::map_value)
            .collect()
    }

    const PLAIN_TURN: &str = include_str!("../../dovetail-driver/tests/fixtures/plain_turn.ndjson");

    #[test]
    fn the_fill_rounds_to_the_nearest_point_and_caps_at_100() {
        assert_eq!(fill_pct(0, 200_000), 0);
        assert_eq!(fill_pct(120_000, 200_000), 60);
        assert_eq!(fill_pct(160_999, 200_000), 80);
        assert_eq!(fill_pct(161_000, 200_000), 81);
        assert_eq!(fill_pct(19_564, 1_000_000), 2);
        assert_eq!(fill_pct(4_999, 1_000_000), 0);
        assert_eq!(fill_pct(250_000, 200_000), 100);
        assert_eq!(fill_pct(u64::MAX, 1), 100);
        assert_eq!(fill_pct(5, 0), 100);
    }

    #[test]
    fn the_table_knows_1m_models_and_defaults_to_200k() {
        assert_eq!(table_window(Some("claude-opus-5-5")), 1_000_000);
        assert_eq!(table_window(Some("claude-sonnet-5-5")), 1_000_000);
        assert_eq!(table_window(Some("claude-fable-5-1")), 1_000_000);
        assert_eq!(table_window(Some("claude-haiku-5-5")), 1_000_000);
        assert_eq!(table_window(Some("claude-haiku-4-5")), DEFAULT_WINDOW);
        assert_eq!(table_window(Some("claude-sonnet-4-5[1m]")), 1_000_000);
        assert_eq!(table_window(Some("some-new-model")), DEFAULT_WINDOW);
        assert_eq!(table_window(None), DEFAULT_WINDOW);
    }

    /// `plain_turn`: input 2 + cache read 10 562 + cache creation 8 995 + output 5, and the
    /// result reports a 1M window for the session's model.
    #[test]
    fn a_recorded_turn_gives_its_tokens_and_window() {
        let events = events(PLAIN_TURN);
        let usage = events
            .iter()
            .find_map(|e| match e {
                DriverEvent::Assistant(m) => m.usage,
                _ => None,
            })
            .expect("assistant usage");
        assert_eq!(usage.context_tokens(), 19_564);
        let result = events
            .iter()
            .find_map(|e| match e {
                DriverEvent::TurnResult(r) => Some(r),
                _ => None,
            })
            .expect("result");
        assert_eq!(
            reported_window(result, Some("claude-sonnet-5-5")),
            Some(("claude-sonnet-5-5".into(), 1_000_000))
        );
        // The session's model unknown: the result's only model is taken.
        assert_eq!(
            reported_window(result, None).map(|(_, w)| w),
            Some(1_000_000)
        );
        assert_eq!(fill_pct(usage.context_tokens(), 1_000_000), 2);
    }

    #[test]
    fn a_result_with_several_models_gives_only_the_sessions_window() {
        let mut events = events(PLAIN_TURN);
        let Some(DriverEvent::TurnResult(mut result)) = events
            .drain(..)
            .find(|e| matches!(e, DriverEvent::TurnResult(_)))
        else {
            panic!("result");
        };
        let mut other = result.model_usage[0].clone();
        other.model = "claude-haiku-4-5".into();
        other.context_window = Some(200_000);
        result.model_usage.push(other);
        assert_eq!(
            reported_window(&result, Some("claude-haiku-4-5")).map(|(_, w)| w),
            Some(200_000)
        );
        // Two models and none is the session's: no guess.
        assert_eq!(reported_window(&result, Some("claude-opus-5-5")), None);
        assert_eq!(reported_window(&result, None), None);
    }
}
