//! The usage indicator on the right of the header row (task #16):
//! `opus-5-5 · 5h 34% (resets 14:20) · week 18% (resets Thu)`.
//!
//! Parts drop as the row narrows: the reset times, then the model, then the week; the 5-hour
//! figure goes last. A window whose reset time has passed shows `—` until the next report.

use dovetail_api::{UsageLimits, UsageWindow};
use jiff::{Timestamp, tz::TimeZone};
use ratatui::text::Span;

use crate::theme::Theme;

const DAY_MS: i64 = 86_400_000;

/// `claude-opus-5-5` → `opus-5-5`; a trailing date (`-20251001`) goes too.
pub fn short_model(model: &str) -> &str {
    let m = model.strip_prefix("claude-").unwrap_or(model);
    match m.rsplit_once('-') {
        Some((head, date)) if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => m,
    }
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

/// One window as shown, or `None` when unknown.
struct Part {
    /// `5h 34%` or `5h —`.
    figure: String,
    /// ` (resets 14:20)`, empty when expired or not reported.
    resets: String,
    pct: Option<u32>,
}

fn part(label: &str, window: Option<&UsageWindow>, now_ms: i64, tz: &TimeZone) -> Option<Part> {
    let w = window?;
    if w.resets_at_ms.is_some_and(|t| t <= now_ms) {
        return Some(Part {
            figure: format!("{label} —"),
            resets: String::new(),
            pct: None,
        });
    }
    // Rounded, and never negative: the colour thresholds work on whole percent.
    let pct = (w.utilization * 100.0).round().max(0.0) as u32;
    Some(Part {
        figure: format!("{label} {pct}%"),
        resets: w
            .resets_at_ms
            .map(|t| format!(" (resets {})", reset_label(t, now_ms, tz)))
            .unwrap_or_default(),
        pct: Some(pct),
    })
}

/// Versions of the indicator, fullest first and each ending in a space, for
/// [`crate::buffers::spread_fit`]. Empty when nothing is known yet.
pub fn variants(
    usage: &UsageLimits,
    model: Option<&str>,
    now_ms: i64,
    tz: &TimeZone,
    theme: &Theme,
) -> Vec<Vec<Span<'static>>> {
    let five = part("5h", usage.five_hour.as_ref(), now_ms, tz);
    let week = part("week", usage.seven_day.as_ref(), now_ms, tz);
    let model = model.map(short_model);
    let build = |model: Option<&str>, with_week: bool, resets: bool| {
        let mut items: Vec<Span<'static>> = Vec::new();
        let mut push = |spans: Vec<Span<'static>>| {
            if !items.is_empty() {
                items.push(Span::styled(" · ", theme.dim()));
            }
            items.extend(spans);
        };
        if let Some(m) = model {
            push(vec![Span::styled(m.to_owned(), theme.dim())]);
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
        build(model, true, true),
        build(model, true, false),
        build(None, true, false),
        build(None, false, false),
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
    use dovetail_api::UsageStatus;

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
    }

    #[test]
    fn variants_drop_resets_then_model_then_week() {
        let theme = Theme::dusk();
        let v = variants(
            &usage(0.34, 0.18, NOW_S),
            Some("claude-opus-5-5"),
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
    fn limits_are_coloured_by_threshold() {
        let theme = Theme::dusk();
        let v = variants(
            &usage(0.72, 0.95, NOW_S),
            None,
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
            None,
            NOW_S * 1_000,
            &TimeZone::UTC,
            &theme,
        );
        assert_eq!(low[0][0].style.fg, Some(theme.dim));
    }

    #[test]
    fn a_passed_reset_shows_a_dash_and_nothing_known_shows_nothing() {
        let theme = Theme::dusk();
        let later = (NOW_S + 3 * 3_600) * 1_000;
        let v = variants(
            &usage(0.34, 0.18, NOW_S),
            None,
            later,
            &TimeZone::UTC,
            &theme,
        );
        assert_eq!(text(&v[0]), "5h — · week 18% (resets Thu) ");
        let none = variants(&UsageLimits::default(), None, later, &TimeZone::UTC, &theme);
        assert!(none.is_empty());
        let model_only = variants(
            &UsageLimits::default(),
            Some("claude-opus-5-5"),
            later,
            &TimeZone::UTC,
            &theme,
        );
        assert_eq!(model_only.len(), 1);
        assert_eq!(text(&model_only[0]), "opus-5-5 ");
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
}
