//! Tests for the `usage` feature.

use crate::common::{Case, roundtrip};
use dovetail_api::usage::{UsageEvent, UsageLimits, UsageStatus, UsageWindow};
use dovetail_api::{Event, ServerMsg, Snapshot};
use serde_json::json;

pub fn cases() -> Vec<Case> {
    vec![Case::new(
        "usage",
        Snapshot::default(),
        vec![(
            1,
            Event::Usage(UsageEvent::Changed(UsageLimits {
                five_hour: Some(UsageWindow {
                    utilization: 0.5,
                    resets_at_ms: None,
                    status: UsageStatus::Allowed,
                }),
                seven_day: None,
            })),
        )],
    )]
}

#[test]
fn usage_events_roundtrip() {
    roundtrip(&ServerMsg::Event {
        seq: 0,
        event: Event::Usage(UsageEvent::Changed(UsageLimits::default())),
    });
}

#[test]
fn a_usage_status_word_this_version_does_not_know_parses_as_unknown() {
    let w: UsageWindow = serde_json::from_value(json!({
        "utilization": 0.5, "resets_at_ms": null, "status": "throttled_soon"
    }))
    .expect("an unknown word still parses");
    assert_eq!(w.status, UsageStatus::Unknown);
    assert_eq!(
        UsageStatus::from_word("allowed_warning"),
        UsageStatus::AllowedWarning
    );
    assert_eq!(UsageStatus::from_word(""), UsageStatus::Unknown);
}
