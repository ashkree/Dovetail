//! Claude Code's account limits: the `get_usage` control request, asked of an idle `claude -p`
//! that is sent no prompt, so no model call is made (spike `docs/spikes/agy-headless.md`).

use crate::query::{self, UsageQuery, rfc3339_ms};
use dovetail_api::model::DriverKind;
use dovetail_api::status::{DriverUsage, LimitWindow, UsageGroup};
use serde_json::{Value, json};
use std::ffi::OsString;

/// The request id the answer is matched by.
const REQUEST_ID: &str = "dovetail_usage";

/// The windows `rate_limits` may carry, in the order shown, with their names.
const WINDOWS: [(&str, &str); 4] = [
    ("five_hour", "5-hour"),
    ("seven_day", "Weekly"),
    ("seven_day_opus", "Weekly Opus"),
    ("seven_day_sonnet", "Weekly Sonnet"),
];

/// Asks `program` for the account's usage.
pub(crate) async fn usage(program: OsString, query: UsageQuery) -> DriverUsage {
    let args = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--no-session-persistence",
    ]
    .map(OsString::from)
    .to_vec();
    let control = |id: &str, subtype: &str| json!({"type": "control_request", "request_id": id, "request": {"subtype": subtype}});
    let input = vec![
        control("dovetail_init", "initialize"),
        control(REQUEST_ID, "get_usage"),
    ];
    let answer = query::ask(program, args, &query, input, true, |v| {
        let response = v.get("response")?;
        (v.get("type")?.as_str()? == "control_response"
            && response.get("request_id")?.as_str()? == REQUEST_ID)
            .then(|| response.clone())
    })
    .await;
    match answer {
        Ok(response) if response.get("subtype").and_then(Value::as_str) == Some("error") => {
            let why = response.get("error").and_then(Value::as_str);
            DriverUsage::failed(DriverKind::Claude, why.unwrap_or("Claude Code refused"))
        }
        Ok(response) => report(response.get("response").unwrap_or(&Value::Null)),
        Err(why) => DriverUsage::failed(DriverKind::Claude, why.to_string()),
    }
}

/// `get_usage`'s answer as a report: one group with each window present.
pub(crate) fn report(answer: &Value) -> DriverUsage {
    let limits = answer.get("rate_limits").unwrap_or(&Value::Null);
    let windows: Vec<LimitWindow> = WINDOWS
        .iter()
        .filter_map(|(key, name)| {
            let w = limits.get(*key).filter(|w| w.is_object())?;
            Some(LimitWindow {
                name: (*name).to_owned(),
                used_fraction: w.get("utilization")?.as_f64()? / 100.0,
                resets_at_ms: rfc3339_ms(w.get("resets_at")),
            })
        })
        .collect();
    let groups = if windows.is_empty() {
        Vec::new()
    } else {
        vec![UsageGroup {
            name: "Claude Code".into(),
            models: String::new(),
            windows,
        }]
    };
    let note = match answer.get("rate_limits_available").and_then(Value::as_bool) {
        Some(false) => Some("no usage limits on this account".to_owned()),
        _ => limits.get("extra_usage").and_then(extra_usage),
    };
    DriverUsage {
        driver: DriverKind::Claude,
        plan: answer
            .get("subscription_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        groups,
        note,
        failed: false,
    }
}

/// The extra-usage line: on with what it has used, or off and why.
fn extra_usage(extra: &Value) -> Option<String> {
    let used = extra.get("used_credits").and_then(Value::as_f64);
    let places = extra
        .get("decimal_places")
        .and_then(Value::as_i64)
        .unwrap_or(2);
    let currency = extra
        .get("currency")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let amount = used.map(|u| {
        let places = usize::try_from(places).unwrap_or(2);
        format!("{:.*} {currency}", places, u / 10f64.powi(places as i32))
    });
    if extra.get("is_enabled").and_then(Value::as_bool)? {
        return Some(match amount {
            Some(a) => format!("extra usage on, {a} used"),
            None => "extra usage on".to_owned(),
        });
    }
    Some(match extra.get("disabled_reason").and_then(Value::as_str) {
        Some(why) => format!("extra usage off ({})", why.replace('_', " ")),
        None => "extra usage off".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spike's answer (`out/c1-usage.ndjson`), trimmed.
    fn answer() -> Value {
        json!({
            "subscription_type": "pro",
            "rate_limits_available": true,
            "rate_limits": {
                "five_hour": {"utilization": 43, "resets_at": "2026-11-21T13:59:59.789974+00:00"},
                "seven_day": {"utilization": 91, "resets_at": "2026-11-22T08:59:59.789991+00:00"},
                "seven_day_opus": null,
                "seven_day_sonnet": null,
                "extra_usage": {"is_enabled": false, "used_credits": 9314, "currency": "USD",
                    "disabled_reason": "out_of_credits", "decimal_places": 2},
            },
        })
    }

    #[test]
    fn the_windows_become_one_group() {
        let usage = report(&answer());
        assert_eq!(usage.plan.as_deref(), Some("pro"));
        assert!(!usage.failed);
        let group = &usage.groups[0];
        assert_eq!(group.name, "Claude Code");
        let names: Vec<_> = group.windows.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["5-hour", "Weekly"]);
        assert!((group.windows[0].used_fraction - 0.43).abs() < 1e-9);
        assert_eq!(group.windows[1].resets_at_ms, Some(1_795_337_999_789));
        assert_eq!(
            usage.note.as_deref(),
            Some("extra usage off (out of credits)")
        );
    }

    #[test]
    fn extra_usage_on_says_what_it_used() {
        let mut a = answer();
        a["rate_limits"]["extra_usage"]["is_enabled"] = json!(true);
        assert_eq!(
            report(&a).note.as_deref(),
            Some("extra usage on, 93.14 USD used")
        );
    }

    #[test]
    fn an_account_without_limits_says_so() {
        let usage = report(&json!({"rate_limits_available": false, "rate_limits": null}));
        assert!(usage.groups.is_empty());
        assert_eq!(
            usage.note.as_deref(),
            Some("no usage limits on this account")
        );
    }
}
