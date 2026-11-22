//! agy's quota: `agy --output-format json -p=/usage`, a one-shot that makes no model call and
//! answers with quota groups, each with a weekly and a 5-hour bucket (spike
//! `docs/spikes/agy-headless.md`).

use crate::query::{self, UsageQuery, rfc3339_ms};
use dovetail_api::model::DriverKind;
use dovetail_api::status::{DriverUsage, LimitWindow, UsageGroup};
use serde_json::Value;
use std::ffi::OsString;

/// What agy's group descriptions start with before the model list.
const MODELS_PREFIX: &str = "Models within this group: ";

/// Asks `program` for the account's quota.
pub(crate) async fn usage(program: OsString, query: UsageQuery) -> DriverUsage {
    let args = ["--output-format", "json", "-p=/usage"]
        .map(OsString::from)
        .to_vec();
    let answer = query::ask(program, args, &query, Vec::new(), false, |v| {
        v.get("command").cloned()
    })
    .await;
    match answer {
        Ok(command) => report(&command),
        Err(why) => DriverUsage::failed(DriverKind::Agy, why.to_string()),
    }
}

/// The `command` object of `/usage`'s answer as a report.
pub(crate) fn report(command: &Value) -> DriverUsage {
    let Some(groups) = command.pointer("/data/groups").and_then(Value::as_array) else {
        return DriverUsage::failed(DriverKind::Agy, "agy's /usage answer had no quota groups");
    };
    let mut groups: Vec<UsageGroup> = groups.iter().filter_map(group).collect();
    for g in &mut groups {
        g.windows.sort_by_key(|w| !w.name.starts_with('5'));
    }
    DriverUsage {
        driver: DriverKind::Agy,
        plan: None,
        groups,
        note: None,
        failed: false,
    }
}

fn group(g: &Value) -> Option<UsageGroup> {
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_owned);
    let description = text(g, "description").unwrap_or_default();
    let windows = g
        .get("buckets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|b| {
            let remaining = b.get("remaining_fraction")?.as_f64()?;
            let name = match b.get("window").and_then(Value::as_str) {
                Some("5h") => "5-hour".to_owned(),
                Some("weekly") => "Weekly".to_owned(),
                _ => text(b, "name").unwrap_or_default(),
            };
            Some(LimitWindow {
                name,
                used_fraction: (1.0 - remaining).max(0.0),
                resets_at_ms: rfc3339_ms(b.get("reset_time")),
            })
        })
        .collect();
    Some(UsageGroup {
        name: text(g, "name")?,
        models: description
            .strip_prefix(MODELS_PREFIX)
            .unwrap_or(&description)
            .to_owned(),
        windows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The spike's answer (`out/p-usage.json`), trimmed.
    fn command() -> Value {
        json!({"name": "usage", "data": {"groups": [
            {"name": "Gemini Models",
             "description": "Models within this group: Gemini 4 Neon, Gemini 3.8 Flash",
             "buckets": [
                {"id": "gemini-weekly", "name": "Weekly Limit Remaining", "window": "weekly",
                 "remaining_fraction": 0.75, "reset_time": "2026-11-28T09:19:46Z"},
                {"id": "gemini-5h", "name": "Five Hour Limit Remaining", "window": "5h",
                 "remaining_fraction": 1, "reset_time": "2026-11-21T14:19:46Z"}]},
            {"name": "Gemini 4 Argon",
             "description": "For paid Pro plans, Argon quota is separate.",
             "buckets": [
                {"id": "argon", "name": "Gemini 4 Argon Weekly Limit", "window": "weekly",
                 "remaining_fraction": 1, "reset_time": "2026-11-28T09:25:40Z"}]},
        ]}})
    }

    #[test]
    fn groups_keep_their_models_and_windows_shortest_first() {
        let usage = report(&command());
        assert!(!usage.failed);
        assert_eq!(usage.groups.len(), 2);
        let gemini = &usage.groups[0];
        assert_eq!(gemini.models, "Gemini 4 Neon, Gemini 3.8 Flash");
        let names: Vec<_> = gemini.windows.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["5-hour", "Weekly"]);
        assert!((gemini.windows[1].used_fraction - 0.25).abs() < 1e-9);
        assert_eq!(gemini.windows[1].resets_at_ms, Some(1_795_857_586_000));
        assert_eq!(
            usage.groups[1].models,
            "For paid Pro plans, Argon quota is separate."
        );
    }

    #[test]
    fn an_answer_without_groups_fails() {
        let usage = report(&json!({"name": "usage"}));
        assert!(usage.failed);
    }
}
