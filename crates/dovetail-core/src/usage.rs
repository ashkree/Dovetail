//! Usage limits and the model in use (task #16).
//!
//! Every turn's `rate_limit_event` carries `rate_limit_info.unifiedWindows.five_hour` and
//! `.seven_day`. Limits are per account, so the core keeps the latest from any session. The
//! model comes from `system/init` and main-thread assistant messages.

use dovetail_api::{AgentId, Event, UsageLimits, UsageWindow};
use serde_json::Value;

use crate::service::Core;

/// Reads the windows out of a `rate_limit_info`. `None` when it carries neither window.
pub(crate) fn from_rate_limit_info(info: &Value) -> Option<UsageLimits> {
    let windows = info.get("unifiedWindows")?;
    let usage = UsageLimits {
        five_hour: windows.get("five_hour").and_then(window),
        seven_day: windows.get("seven_day").and_then(window),
    };
    (usage.five_hour.is_some() || usage.seven_day.is_some()).then_some(usage)
}

fn window(v: &Value) -> Option<UsageWindow> {
    Some(UsageWindow {
        utilization: v.get("utilization")?.as_f64()?,
        resets_at: v.get("resetsAt").and_then(Value::as_i64),
        status: v
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}

/// `next` over `current`: a window the event leaves out keeps its last value.
fn merged(current: &UsageLimits, next: UsageLimits) -> UsageLimits {
    UsageLimits {
        five_hour: next.five_hour.or_else(|| current.five_hour.clone()),
        seven_day: next.seven_day.or_else(|| current.seven_day.clone()),
    }
}

impl Core {
    /// A session reported its usage limits: tell the UI if anything changed.
    pub(crate) fn on_rate_limit(&mut self, info: &Value) {
        let Some(next) = from_rate_limit_info(info) else {
            return;
        };
        let usage = merged(&self.usage, next);
        if usage != self.usage {
            self.usage = usage.clone();
            self.emit(Event::UsageChanged(usage));
        }
    }

    /// The session reported its model. Claude Code's own placeholder messages (`<synthetic>`)
    /// are not a model.
    pub(crate) fn set_model(&mut self, agent: &AgentId, model: Option<&str>) {
        let Some(model) = model.filter(|m| !m.is_empty() && !m.starts_with('<')) else {
            return;
        };
        self.update_agent(agent, |row| {
            if row.agent.model.as_deref() != Some(model) {
                row.agent.model = Some(model.to_owned());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dovetail_driver::DriverEvent;

    /// The first `rate_limit_event` of a recorded fixture, through the driver's parser.
    fn fixture_info() -> Value {
        let text = include_str!("../../dovetail-driver/tests/fixtures/flags.ndjson");
        text.lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|l| l.get("msg").cloned())
            .flat_map(dovetail_driver::parse::map_value)
            .find_map(|e| match e {
                DriverEvent::RateLimit { info } => Some(info),
                _ => None,
            })
            .unwrap_or(Value::Null)
    }

    #[test]
    fn parses_both_windows_from_a_fixture() {
        let usage = from_rate_limit_info(&fixture_info());
        assert_eq!(
            usage,
            Some(UsageLimits {
                five_hour: Some(UsageWindow {
                    utilization: 0.13,
                    resets_at: Some(1_793_584_200),
                    status: "allowed".into(),
                }),
                seven_day: Some(UsageWindow {
                    utilization: 0.17,
                    resets_at: Some(1_794_128_400),
                    status: "allowed".into(),
                }),
            })
        );
    }

    #[test]
    fn info_without_windows_is_ignored_and_missing_windows_keep_their_value() {
        assert_eq!(
            from_rate_limit_info(&serde_json::json!({"status": "allowed"})),
            None
        );
        let full = from_rate_limit_info(&fixture_info()).unwrap_or_default();
        let only_5h = from_rate_limit_info(&serde_json::json!({
            "unifiedWindows": {"five_hour": {"utilization": 0.5, "resetsAt": 1, "status": "allowed"}}
        }))
        .unwrap_or_default();
        let m = merged(&full, only_5h);
        assert_eq!(m.five_hour.map(|w| w.utilization), Some(0.5));
        assert_eq!(m.seven_day, full.seven_day);
    }
}
