//! Usage limits and the model in use (task #16).
//!
//! Every turn's `rate_limit_event` carries a 5-hour and a weekly window, parsed by the driver.
//! Limits are per account, so the core keeps the latest from any session. Here the driver's
//! windows become the API's: reset times in milliseconds, the status word as [`UsageStatus`].
//! The model comes from `system/init` and main-thread assistant messages.

use dovetail_api::{AgentId, Event, UsageLimits, UsageStatus, UsageWindow};
use dovetail_driver::{RateLimitInfo, RateLimitWindow};

use crate::service::Core;

/// The API's view of a `rate_limit_event`. `None` when it carries neither window.
pub(crate) fn from_rate_limit(info: &RateLimitInfo) -> Option<UsageLimits> {
    let usage = UsageLimits {
        five_hour: info.five_hour.as_ref().map(window),
        seven_day: info.seven_day.as_ref().map(window),
    };
    (usage.five_hour.is_some() || usage.seven_day.is_some()).then_some(usage)
}

fn window(w: &RateLimitWindow) -> UsageWindow {
    UsageWindow {
        utilization: w.utilization,
        resets_at_ms: w.resets_at_s.map(|s| s.saturating_mul(1_000)),
        status: UsageStatus::from_word(&w.status),
    }
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
    pub(crate) fn on_rate_limit(&mut self, info: &RateLimitInfo) {
        let Some(next) = from_rate_limit(info) else {
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
    use serde_json::{Value, json};

    /// The first `rate_limit_event` of a recorded fixture, through the driver's parser.
    fn fixture_info() -> RateLimitInfo {
        first_rate_limit(include_str!(
            "../../dovetail-driver/tests/fixtures/flags.ndjson"
        ))
    }

    fn first_rate_limit(text: &str) -> RateLimitInfo {
        text.lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter_map(|l| l.get("msg").cloned())
            .flat_map(dovetail_driver::parse::map_value)
            .find_map(|e| match e {
                DriverEvent::RateLimit(info) => Some(info),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// A `rate_limit_info`, through the driver's parser.
    fn info(rate_limit_info: Value) -> RateLimitInfo {
        let line = json!({"type": "rate_limit_event", "rate_limit_info": rate_limit_info});
        match dovetail_driver::parse::map_value(line).pop() {
            Some(DriverEvent::RateLimit(info)) => info,
            _ => RateLimitInfo::default(),
        }
    }

    #[test]
    fn parses_both_windows_from_a_fixture() {
        let usage = from_rate_limit(&fixture_info());
        assert_eq!(
            usage,
            Some(UsageLimits {
                five_hour: Some(UsageWindow {
                    utilization: 0.13,
                    resets_at_ms: Some(1_793_584_200_000),
                    status: UsageStatus::Allowed,
                }),
                seven_day: Some(UsageWindow {
                    utilization: 0.17,
                    resets_at_ms: Some(1_794_128_400_000),
                    status: UsageStatus::Allowed,
                }),
            })
        );
    }

    #[test]
    fn an_unknown_status_word_still_parses() {
        let usage = from_rate_limit(&info(json!({
            "unifiedWindows": {"five_hour": {"utilization": 0.5, "status": "throttled_soon"}}
        })));
        let five = usage.and_then(|u| u.five_hour);
        assert_eq!(five.as_ref().map(|w| w.status), Some(UsageStatus::Unknown));
        assert_eq!(five.and_then(|w| w.resets_at_ms), None);
    }

    #[test]
    fn info_without_windows_is_ignored_and_missing_windows_keep_their_value() {
        assert_eq!(from_rate_limit(&info(json!({"status": "allowed"}))), None);
        let full = from_rate_limit(&fixture_info()).unwrap_or_default();
        let only_5h = from_rate_limit(&info(json!({
            "unifiedWindows": {"five_hour": {"utilization": 0.5, "resetsAt": 1, "status": "allowed"}}
        })))
        .unwrap_or_default();
        let m = merged(&full, only_5h);
        assert_eq!(m.five_hour.map(|w| w.utilization), Some(0.5));
        assert_eq!(m.seven_day, full.seven_day);
    }

    /// Task #26: Claude Code 2.1.319's week-only event updates the week and keeps the last 5h.
    #[test]
    fn a_week_only_event_keeps_the_earlier_5h_window() {
        let week_only = first_rate_limit(include_str!(
            "../../dovetail-driver/tests/fixtures/rate_limit_week_only.ndjson"
        ));
        let next = from_rate_limit(&week_only).unwrap_or_default();
        assert_eq!(next.five_hour, None);
        assert_eq!(next.seven_day.as_ref().map(|w| w.utilization), Some(0.06));
        let full = from_rate_limit(&fixture_info()).unwrap_or_default();
        let m = merged(&full, next);
        assert_eq!(m.five_hour, full.five_hour);
        assert_eq!(m.seven_day.map(|w| w.utilization), Some(0.06));
    }
}
