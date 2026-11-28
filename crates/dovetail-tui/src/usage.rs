//! The usage indicator at the bottom right, on the status row (task #16):
//! `maveron · opus-5-5 high · ctx 24% · 5h 34% (resets 14:20) · week 18% (resets Thu)`. The
//! model's effort follows it once the session has confirmed one (task #39); the coordinator's
//! context fill comes right after the model (task #40).
//!
//! The project's Claude profile leads (ADR 0009): limits are per account, so the figures are that
//! profile's. Parts drop as the row narrows: the reset times, then the profile and model, then the
//! week, then the context fill; the 5-hour figure goes last. A window whose reset time has passed, or that the report left out, shows `—`.

use dovetail_api::{
    model::ReasoningEffort,
    status::{UsageGroup, UsageLimits, UsageStatus, UsageWindow},
};
use jiff::{Timestamp, tz::TimeZone};
use ratatui::text::Span;

use crate::theme::Theme;

const DAY_MS: i64 = 86_400_000;

/// The coordinator context fill above which the line hints at `:handoff`.
const FRESH_HINT_PCT: u8 = 80;

/// `claude-opus-5-5` → `opus-5-5`; a trailing date (`-20251001`) goes too.
pub fn short_model(model: &str) -> &str {
    let m = model.strip_prefix("claude-").unwrap_or(model);
    match m.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => m,
    }
}

/// The model's family for a tight spot: `claude-sonnet-5-5` → `sonnet`. Any other id is returned
/// as [`short_model`] has it.
pub fn model_family(model: &str) -> &str {
    let short = short_model(model);
    ["opus", "sonnet", "haiku", "fable"]
        .into_iter()
        .find(|f| {
            short
                .strip_prefix(f)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
        })
        .unwrap_or(short)
}

/// `low` → `l`, `medium` → `m`, `high` → `h`, `xhigh` → `xh`, `max` → `max`.
pub fn effort_shorthand(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::Low => "l",
        ReasoningEffort::Medium => "m",
        ReasoningEffort::High => "h",
        ReasoningEffort::Xhigh => "xh",
        ReasoningEffort::Max => "max",
    }
}

/// A usage fraction (`0.0..=1.0`, possibly above `1.0`) rounded to whole percent.
pub fn pct(used_fraction: f64) -> u32 {
    (used_fraction * 100.0).round().max(0.0) as u32
}

/// A reset time in `tz`: `14:20` within 24 hours of `now_ms`, otherwise the weekday (`Thu`).
pub fn reset_label(resets_at_ms: i64, now_ms: i64, tz: &TimeZone) -> String {
    let Ok(ts) = Timestamp::from_millisecond(resets_at_ms) else {
        return "?".into();
    };
    let format = if resets_at_ms - now_ms < DAY_MS {
        "%H:%M"
    } else {
        "%a"
    };
    ts.to_zoned(tz.clone()).strftime(format).to_string()
}

/// Matches the agent's model to the matching quota group in an agy usage report (task #58).
///
/// Falls back to the first group if there is only one group or no model/name match.
pub fn match_usage_group<'a>(
    groups: &'a [UsageGroup],
    model: Option<&str>,
) -> Option<&'a UsageGroup> {
    if groups.is_empty() {
        return None;
    }
    if groups.len() == 1 {
        return Some(&groups[0]);
    }
    let Some(raw_model) = model.filter(|m| !m.is_empty()) else {
        return Some(&groups[0]);
    };

    // Strip trailing effort suffix if present (e.g. gemini-4-neon-medium -> gemini-4-neon)
    let clean = ["-low", "-medium", "-high", "-xhigh", "-max"]
        .into_iter()
        .find_map(|eff| raw_model.strip_suffix(eff))
        .unwrap_or(raw_model);

    let clean_lower = clean.to_ascii_lowercase();
    let clean_spaced = clean_lower.replace(['-', '_'], " ");

    // 1. Direct or spaced match in group.models or group.name
    if let Some(g) = groups.iter().find(|g| {
        let m = g.models.to_ascii_lowercase();
        let n = g.name.to_ascii_lowercase();
        m.contains(&clean_lower)
            || m.contains(&clean_spaced)
            || n.contains(&clean_lower)
            || n.contains(&clean_spaced)
    }) {
        return Some(g);
    }

    // 2. Distinguishing sub-family or specific model keywords:
    // e.g. "argon", "neon", "flash", "opus", "sonnet", "haiku", "fable"
    const SUB_FAMILIES: &[&str] = &["argon", "neon", "flash", "opus", "sonnet", "haiku", "fable"];
    for &sub in SUB_FAMILIES {
        if clean_lower.contains(sub)
            && let Some(g) = groups.iter().find(|g| {
                g.name.to_ascii_lowercase().contains(sub)
                    || g.models.to_ascii_lowercase().contains(sub)
            })
        {
            return Some(g);
        }
    }

    // 3. Top-level family keywords: "gemini", "claude", "gpt"
    const FAMILIES: &[&str] = &["gemini", "claude", "gpt"];
    for &fam in FAMILIES {
        if clean_lower.contains(fam)
            && let Some(g) = groups.iter().find(|g| {
                g.name.to_ascii_lowercase().contains(fam)
                    || g.models.to_ascii_lowercase().contains(fam)
            })
        {
            return Some(g);
        }
    }

    // Fallback: first group
    Some(&groups[0])
}

/// Converts an agy `UsageGroup` into `UsageLimits` for status bar rendering and reset timers (task #58).
pub fn group_to_usage_limits(group: &UsageGroup) -> UsageLimits {
    let five_hour = group
        .windows
        .iter()
        .find(|w| {
            w.name.starts_with('5')
                || w.name.eq_ignore_ascii_case("5-hour")
                || w.name.eq_ignore_ascii_case("5h")
        })
        .map(|w| UsageWindow {
            utilization: w.used_fraction,
            resets_at_ms: w.resets_at_ms,
            status: UsageStatus::Allowed,
        });
    let seven_day = group
        .windows
        .iter()
        .find(|w| w.name.to_ascii_lowercase().starts_with("week"))
        .map(|w| UsageWindow {
            utilization: w.used_fraction,
            resets_at_ms: w.resets_at_ms,
            status: UsageStatus::Allowed,
        });
    UsageLimits {
        five_hour,
        seven_day,
    }
}

/// When the indicator next changes on its own, in Unix ms: a reset time passing, or one coming
/// within 24 hours (weekday → `hh:mm`). `None` when nothing is pending, so the UI sleeps.
pub fn next_change_ms(usage: &UsageLimits, now_ms: i64) -> Option<i64> {
    [&usage.five_hour, &usage.seven_day]
        .into_iter()
        .flatten()
        .filter_map(|w| w.resets_at_ms)
        .flat_map(|t| [t - DAY_MS, t])
        .filter(|&t| t > now_ms)
        .min()
}

/// One window as shown.
struct Part {
    /// `5h 34%` or `5h —`.
    figure: String,
    /// ` (resets 14:20)`, empty when expired or not reported.
    resets: String,
    pct: Option<u32>,
}

/// One window as shown. A window not reported, or whose reset has passed, is a dash (`5h —`):
/// Claude Code 2.1.319 can send a `rate_limit_event` with only the weekly window (task #26).
fn part(label: &str, window: Option<&UsageWindow>, now_ms: i64, tz: &TimeZone) -> Part {
    let dash = || Part {
        figure: format!("{label} —"),
        resets: String::new(),
        pct: None,
    };
    let Some(w) = window else {
        return dash();
    };
    if w.resets_at_ms.is_some_and(|t| t <= now_ms) {
        return dash();
    }
    // Rounded, and never negative: the colour thresholds work on whole percent.
    let pct = pct(w.utilization);
    Part {
        figure: format!("{label} {pct}%"),
        resets: w
            .resets_at_ms
            .map(|t| format!(" (resets {})", reset_label(t, now_ms, tz)))
            .unwrap_or_default(),
        pct: Some(pct),
    }
}

/// What leads the line, before the limits: the account, the model in view and the coordinator's
/// context fill.
#[derive(Debug, Clone, Copy, Default)]
pub struct Lead<'a> {
    /// The Claude profile of the project (ADR 0009).
    pub profile: Option<&'a str>,
    /// The model of the conversation in view, as Claude Code names it.
    pub model: Option<&'a str>,
    /// The model's confirmed effort.
    pub effort: Option<ReasoningEffort>,
    /// The coordinator's context fill, in percent.
    pub context_pct: Option<u8>,
}

/// Versions of the indicator, fullest first and each ending in a space, for
/// [`crate::buffers::spread_fit`]. Empty when nothing is known yet.
pub fn variants(
    usage: &UsageLimits,
    lead: &Lead,
    now_ms: i64,
    tz: &TimeZone,
    theme: &Theme,
) -> Vec<Vec<Span<'static>>> {
    let Lead {
        profile,
        model,
        effort,
        context_pct,
    } = *lead;
    // Nothing shows until a first report; after it, both windows always do.
    let known = usage.five_hour.is_some() || usage.seven_day.is_some();
    let five = known.then(|| part("5h", usage.five_hour.as_ref(), now_ms, tz));
    let week = known.then(|| part("week", usage.seven_day.as_ref(), now_ms, tz));
    let model = model.map(|m| match effort {
        Some(e) => format!("{} {}", short_model(m), e.as_str()),
        None => short_model(m).to_owned(),
    });
    let build = |lead: bool, with_ctx: bool, with_week: bool, resets: bool| {
        let mut items: Vec<Span<'static>> = Vec::new();
        let mut push = |spans: Vec<Span<'static>>| {
            if !items.is_empty() {
                items.push(Span::styled(" · ", theme.dim()));
            }
            items.extend(spans);
        };
        for name in [profile, model.as_deref()]
            .into_iter()
            .flatten()
            .filter(|_| lead)
        {
            push(vec![Span::styled(name.to_owned(), theme.dim())]);
        }
        if let Some(pct) = context_pct.filter(|_| with_ctx) {
            let mut spans = vec![Span::styled(format!("ctx {pct}%"), theme.context(pct))];
            // A manual fresh start, text only: `:handoff` is the command (ADR 0012 amendment).
            if pct > FRESH_HINT_PCT {
                spans.push(Span::styled(" :handoff", theme.dim()));
            }
            push(spans);
        }
        for p in [five.as_ref(), week.as_ref().filter(|_| with_week)]
            .into_iter()
            .flatten()
        {
            let style = p.pct.map_or_else(|| theme.dim(), |pct| theme.usage(pct));
            let mut spans = vec![Span::styled(p.figure.clone(), style)];
            if resets && !p.resets.is_empty() {
                spans.push(Span::styled(p.resets.clone(), style));
            }
            push(spans);
        }
        if !items.is_empty() {
            items.push(Span::raw(" "));
        }
        items
    };
    let mut out: Vec<Vec<Span<'static>>> = Vec::new();
    for v in [
        build(true, true, true, true),
        build(true, true, true, false),
        build(false, true, true, false),
        build(false, true, false, false),
        build(false, false, false, false),
    ] {
        // A variant that drops nothing new (no resets known, no model yet) is skipped.
        if !v.is_empty() && out.last() != Some(&v) {
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dovetail_api::status::UsageStatus;

    fn text(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn usage(five: f64, week: f64, now_s: i64) -> UsageLimits {
        UsageLimits {
            five_hour: Some(UsageWindow {
                utilization: five,
                resets_at_ms: Some((now_s + 2 * 3_600) * 1_000),
                status: UsageStatus::Allowed,
            }),
            seven_day: Some(UsageWindow {
                utilization: week,
                resets_at_ms: Some((now_s + 3 * 86_400) * 1_000),
                status: UsageStatus::Allowed,
            }),
        }
    }

    // 2026-11-09 12:00 UTC, a Monday.
    const NOW_S: i64 = 1_794_225_600;

    #[test]
    fn model_names_lose_the_claude_prefix_and_dates() {
        assert_eq!(short_model("claude-opus-5-5"), "opus-5-5");
        assert_eq!(short_model("claude-haiku-4-5-20251001"), "haiku-4-5");
        assert_eq!(short_model("opus"), "opus");
        assert_eq!(model_family("claude-sonnet-5-5"), "sonnet");
        assert_eq!(model_family("claude-haiku-4-5-20251001"), "haiku");
        assert_eq!(model_family("claude-opus-5-5"), "opus");
        assert_eq!(model_family("claude-fable-5-2"), "fable");
        assert_eq!(model_family("sonnet"), "sonnet");
        assert_eq!(model_family("claude-sonnetish-1"), "sonnetish-1");
        assert_eq!(model_family("gpt-x"), "gpt-x");
        let shorthand: Vec<_> = ReasoningEffort::ALL.map(effort_shorthand).to_vec();
        assert_eq!(shorthand, ["l", "m", "h", "xh", "max"]);
    }

    #[test]
    fn variants_drop_resets_then_model_then_week() {
        let theme = Theme::default();
        let v = variants(
            &usage(0.34, 0.18, NOW_S),
            &Lead {
                model: Some("claude-opus-5-5"),
                ..Lead::default()
            },
            NOW_S * 1_000,
            &TimeZone::UTC,
            &theme,
        );
        let texts: Vec<String> = v.iter().map(|s| text(s)).collect();
        assert_eq!(
            texts,
            vec![
                "opus-5-5 · 5h 34% (resets 14:00) · week 18% (resets Thu) ",
                "opus-5-5 · 5h 34% · week 18% ",
                "5h 34% · week 18% ",
                "5h 34% ",
            ]
        );
    }

    #[test]
    fn the_profile_leads_and_drops_with_the_model() {
        let theme = Theme::default();
        let v = variants(
            &usage(0.34, 0.18, NOW_S),
            &Lead {
                profile: Some("maveron"),
                model: Some("claude-opus-5-5"),
                ..Lead::default()
            },
            NOW_S * 1_000,
            &TimeZone::UTC,
            &theme,
        );
        let texts: Vec<String> = v.iter().map(|s| text(s)).collect();
        assert_eq!(texts[1], "maveron · opus-5-5 · 5h 34% · week 18% ");
        assert_eq!(texts[2], "5h 34% · week 18% ");
    }

    #[test]
    fn the_effort_follows_the_model_once_confirmed() {
        let theme = Theme::default();
        let lead = Lead {
            model: Some("claude-sonnet-5-5"),
            effort: Some(ReasoningEffort::Xhigh),
            ..Lead::default()
        };
        let v = variants(
            &UsageLimits::default(),
            &lead,
            NOW_S * 1_000,
            &TimeZone::UTC,
            &theme,
        );
        assert_eq!(text(&v[0]), "sonnet-5-5 xhigh ");
    }

    #[test]
    fn limits_are_coloured_by_threshold() {
        let theme = Theme::default();
        let v = variants(
            &usage(0.72, 0.95, NOW_S),
            &Lead::default(),
            NOW_S * 1_000,
            &TimeZone::UTC,
            &theme,
        );
        let colour_of = |needle: &str| {
            v[0].iter()
                .find(|s| s.content.contains(needle))
                .and_then(|s| s.style.fg)
        };
        assert_eq!(colour_of("5h 72%"), Some(theme.needs_you));
        assert_eq!(colour_of("week 95%"), Some(theme.error));
        let low = variants(
            &usage(0.1, 0.1, NOW_S),
            &Lead::default(),
            NOW_S * 1_000,
            &TimeZone::UTC,
            &theme,
        );
        assert_eq!(low[0][0].style.fg, Some(theme.dim));
    }

    #[test]
    fn a_passed_reset_shows_a_dash_and_nothing_known_shows_nothing() {
        let theme = Theme::default();
        let later = (NOW_S + 3 * 3_600) * 1_000;
        let v = variants(
            &usage(0.34, 0.18, NOW_S),
            &Lead::default(),
            later,
            &TimeZone::UTC,
            &theme,
        );
        assert_eq!(text(&v[0]), "5h — · week 18% (resets Thu) ");
        let none = variants(
            &UsageLimits::default(),
            &Lead::default(),
            later,
            &TimeZone::UTC,
            &theme,
        );
        assert!(none.is_empty());
        let lead = Lead {
            model: Some("claude-opus-5-5"),
            ..Lead::default()
        };
        let model_only = variants(
            &UsageLimits::default(),
            &lead,
            later,
            &TimeZone::UTC,
            &theme,
        );
        assert_eq!(model_only.len(), 1);
        assert_eq!(text(&model_only[0]), "opus-5-5 ");
    }

    #[test]
    fn a_window_left_out_of_the_report_shows_a_dash_and_drops_last() {
        let theme = Theme::default();
        let mut u = usage(0.34, 0.06, NOW_S);
        u.five_hour = None;
        let v = variants(&u, &Lead::default(), NOW_S * 1_000, &TimeZone::UTC, &theme);
        let texts: Vec<String> = v.iter().map(|s| text(s)).collect();
        assert_eq!(
            texts,
            vec!["5h — · week 6% (resets Thu) ", "5h — · week 6% ", "5h — "]
        );
    }

    #[test]
    fn context_fill_follows_the_model_and_hints_at_handoff_above_80() {
        let theme = Theme::default();
        let at = |pct| {
            let v = variants(
                &usage(0.54, 0.76, NOW_S),
                &Lead {
                    model: Some("claude-opus-5-5"),
                    effort: Some(ReasoningEffort::Medium),
                    context_pct: Some(pct),
                    ..Lead::default()
                },
                NOW_S * 1_000,
                &TimeZone::UTC,
                &theme,
            );
            v.iter().map(|s| text(s)).collect::<Vec<_>>()
        };
        let texts = at(24);
        assert_eq!(
            texts[0],
            "opus-5-5 medium · ctx 24% · 5h 54% (resets 14:00) · week 76% (resets Thu) "
        );
        // The context fill outlives the model and the week as the row narrows.
        assert_eq!(texts[2], "ctx 24% · 5h 54% · week 76% ");
        assert_eq!(texts[3], "ctx 24% · 5h 54% ");
        assert_eq!(texts[4], "5h 54% ");
        assert!(!texts[0].contains(":handoff"));
        assert!(at(81)[0].contains("ctx 81% :handoff · 5h"));
    }

    #[test]
    fn the_next_change_is_a_reset_or_a_reset_coming_within_a_day() {
        let u = usage(0.34, 0.18, NOW_S);
        let now = NOW_S * 1_000;
        assert_eq!(next_change_ms(&u, now), Some(now + 2 * 3_600_000));
        let after_5h = now + 2 * 3_600_000;
        assert_eq!(next_change_ms(&u, after_5h), Some(now + 2 * DAY_MS));
        assert_eq!(next_change_ms(&UsageLimits::default(), now), None);
    }

    #[test]
    fn match_usage_group_finds_matching_group_and_handles_edge_cases() {
        use dovetail_api::status::LimitWindow;

        let groups = vec![
            UsageGroup {
                name: "Gemini Models".into(),
                models: "Gemini 4 Neon, Gemini 3.8 Flash".into(),
                windows: vec![LimitWindow {
                    name: "5-hour".into(),
                    used_fraction: 0.1,
                    resets_at_ms: Some(1_000),
                }],
            },
            UsageGroup {
                name: "Gemini 4 Argon".into(),
                models: String::new(),
                windows: vec![LimitWindow {
                    name: "5-hour".into(),
                    used_fraction: 0.4,
                    resets_at_ms: Some(2_000),
                }],
            },
            UsageGroup {
                name: "Claude Models".into(),
                models: "Claude Opus, Claude Sonnet".into(),
                windows: vec![LimitWindow {
                    name: "Weekly".into(),
                    used_fraction: 0.7,
                    resets_at_ms: Some(3_000),
                }],
            },
        ];

        // Empty groups returns None
        assert!(match_usage_group(&[], Some("gemini-4-neon")).is_none());

        // Single group returns it
        assert_eq!(
            match_usage_group(&groups[..1], Some("claude-opus-5-5")),
            Some(&groups[0])
        );

        // None or empty model falls back to first group
        assert_eq!(match_usage_group(&groups, None), Some(&groups[0]));
        assert_eq!(match_usage_group(&groups, Some("")), Some(&groups[0]));

        // Direct/keyword matching with effort stripped:
        assert_eq!(
            match_usage_group(&groups, Some("gemini-4-neon-medium")),
            Some(&groups[0])
        );
        assert_eq!(
            match_usage_group(&groups, Some("gemini-3.8-flash-low")),
            Some(&groups[0])
        );
        assert_eq!(
            match_usage_group(&groups, Some("gemini-4-argon")),
            Some(&groups[1])
        );
        assert_eq!(
            match_usage_group(&groups, Some("gemini-4-argon-high")),
            Some(&groups[1])
        );
        assert_eq!(
            match_usage_group(&groups, Some("claude-opus-5-5")),
            Some(&groups[2])
        );

        // Unknown model falls back to first group
        assert_eq!(
            match_usage_group(&groups, Some("unknown-custom-model")),
            Some(&groups[0])
        );
    }

    #[test]
    fn group_to_usage_limits_converts_windows_correctly() {
        use dovetail_api::status::LimitWindow;

        let g = UsageGroup {
            name: "Gemini".into(),
            models: "".into(),
            windows: vec![
                LimitWindow {
                    name: "5-hour".into(),
                    used_fraction: 0.25,
                    resets_at_ms: Some(1_000_000),
                },
                LimitWindow {
                    name: "Weekly".into(),
                    used_fraction: 0.50,
                    resets_at_ms: Some(2_000_000),
                },
            ],
        };
        let limits = group_to_usage_limits(&g);
        assert_eq!(
            limits.five_hour,
            Some(UsageWindow {
                utilization: 0.25,
                resets_at_ms: Some(1_000_000),
                status: UsageStatus::Allowed,
            })
        );
        assert_eq!(
            limits.seven_day,
            Some(UsageWindow {
                utilization: 0.50,
                resets_at_ms: Some(2_000_000),
                status: UsageStatus::Allowed,
            })
        );
    }
}
