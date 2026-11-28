//! The usage limits menu (task #46): `/usage` in the chat bar and `:usage`. It wraps the one
//! [`Picker`], asking the core for [`dovetail_api::status::DriverUsage`] reports on open and on `Tab`
//! (`UsageCommand::LoadUsage`), showing one row per quota group grouped by driver (`Claude Code`, then
//! `agy`) and a failed report's reason when reading failed.

use crossterm::event::{KeyCode, KeyEvent};
use dovetail_api::status::DriverUsage;
use jiff::tz::TimeZone;
use ratatui::text::{Line, Span};

use crate::fmt;
use crate::picker::{Picker, PickerItem, PickerKey, PickerOutcome};
use crate::theme::Theme;
use crate::usage;

/// Width of each usage window's progress bar in terminal cells.
const BAR_WIDTH: usize = 10;

/// What a key did to the usage menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageStep {
    /// Still open.
    Pending,
    /// Reload usage limits (`Tab`).
    Refresh,
    /// Close the menu (`Esc`).
    Close,
}

#[derive(Debug, Clone, Copy)]
enum RowKind {
    Failed,
    Empty,
    Group(usize),
}

#[derive(Debug, Clone, Copy)]
struct UsageRow {
    report: usize,
    kind: RowKind,
}

/// The open usage limits menu.
pub struct UsageMenu {
    /// `None` until the first [`dovetail_api::status::StatusEvent::UsageLoaded`] arrives.
    reports: Option<Vec<DriverUsage>>,
    rows: Vec<UsageRow>,
    picker: Picker,
}

impl UsageMenu {
    /// A menu in the loading state; the caller sends [`dovetail_api::status::StatusCommand::LoadUsage`].
    pub fn new() -> Self {
        Self {
            reports: None,
            rows: Vec::new(),
            picker: Picker::new("Usage", Vec::new()).preview_from_top(),
        }
    }

    /// The underlying picker, for rendering.
    pub fn picker(&self) -> &Picker {
        &self.picker
    }

    /// Applies a key: `Tab` refreshes, `Esc` closes, everything else goes to the picker.
    pub fn key(&mut self, key: KeyEvent) -> UsageStep {
        if key.code == KeyCode::Tab {
            return UsageStep::Refresh;
        }
        match self.picker.key(key) {
            PickerOutcome::Pending | PickerOutcome::Chosen(_) => UsageStep::Pending,
            PickerOutcome::Closed => UsageStep::Close,
        }
    }

    /// Fills the menu with the core's reports.
    pub fn set_reports(&mut self, reports: Vec<DriverUsage>, theme: &Theme) {
        let reports = match self.reports.take() {
            Some(mut existing) => {
                for r in reports {
                    if let Some(pos) = existing.iter().position(|e| e.driver == r.driver) {
                        existing[pos] = r;
                    } else {
                        existing.push(r);
                    }
                }
                existing.sort_by_key(|r| r.driver);
                existing
            }
            None => reports,
        };
        let (items, rows) = build_rows(&reports, theme);
        self.rows = rows;
        self.reports = Some(reports);
        self.picker.set_items(items);
    }

    fn selected_row(&self) -> Option<(&DriverUsage, RowKind)> {
        let PickerKey::UsageGroup(idx) = &self.picker.selected()?.key else {
            return None;
        };
        let row = self.rows.get(*idx)?;
        let report = self.reports.as_ref()?.get(row.report)?;
        Some((report, row.kind))
    }

    /// The preview pane's lines for the loading state or the selected row.
    pub fn preview(
        &self,
        width: usize,
        now_ms: i64,
        tz: &TimeZone,
        theme: &Theme,
    ) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        match (&self.reports, self.selected_row()) {
            (None, _) => {
                out.push(Line::from(Span::styled(
                    "loading usage limits…",
                    theme.dim(),
                )));
            }
            (Some(reports), None) if reports.is_empty() => {
                out.push(Line::from(Span::styled("no usage reports", theme.dim())));
            }
            (Some(_), None) => {
                out.push(Line::from(Span::styled(
                    "no matching usage groups",
                    theme.dim(),
                )));
            }
            (Some(_), Some((report, RowKind::Failed))) => {
                out.push(Line::from(vec![
                    Span::styled("✗ ", theme.fg(theme.error)),
                    Span::styled(report.driver.label(), theme.accent()),
                    Span::styled("  unavailable", theme.fg(theme.error)),
                ]));
                out.push(Line::default());
                let reason = report
                    .note
                    .as_deref()
                    .unwrap_or("could not read usage limits");
                for line in fmt::wrap(reason, width) {
                    out.push(Line::from(Span::styled(line, theme.fg(theme.error))));
                }
            }
            (Some(_), Some((report, RowKind::Empty))) => {
                out.push(driver_plan_line(report, theme, true));
                out.push(Line::default());
                out.push(Line::from(Span::styled("no limits reported", theme.dim())));
                if let Some(note) = &report.note {
                    out.push(Line::default());
                    for line in fmt::wrap(note, width) {
                        out.push(Line::from(Span::styled(line, theme.body())));
                    }
                }
            }
            (Some(_), Some((report, RowKind::Group(g_idx)))) => {
                let group = &report.groups[g_idx];
                out.push(Line::from(Span::styled(group.name.clone(), theme.accent())));
                out.push(driver_plan_line(report, theme, false));
                if !group.models.is_empty() {
                    out.push(Line::default());
                    for line in fmt::wrap(&group.models, width) {
                        out.push(Line::from(Span::styled(line, theme.body())));
                    }
                }
                if !group.windows.is_empty() {
                    out.push(Line::default());
                    for window in &group.windows {
                        let pct = usage::pct(window.used_fraction);
                        let style = theme.usage(pct);
                        let mut spans = vec![
                            Span::styled(format!("{:<14}", window.name), theme.body()),
                            Span::raw(" "),
                            Span::styled(usage_bar(window.used_fraction), style),
                            Span::raw(" "),
                            Span::styled(format!("{pct:>3}%"), style),
                        ];
                        if let Some(ms) = window.resets_at_ms {
                            let reset = usage::reset_label(ms, now_ms, tz);
                            spans.push(Span::styled(format!("  (resets {reset})"), theme.dim()));
                        }
                        out.push(Line::from(spans));
                    }
                }
                if let Some(note) = &report.note {
                    out.push(Line::default());
                    for line in fmt::wrap(note, width) {
                        out.push(Line::from(Span::styled(line, theme.dim())));
                    }
                }
            }
        }
        out.push(Line::default());
        out.push(Line::from(Span::styled(
            "tab refresh · esc close",
            theme.dim(),
        )));
        out
    }
}

impl Default for UsageMenu {
    fn default() -> Self {
        Self::new()
    }
}

/// Picker rows and the row index table, built together from `reports`.
fn build_rows(reports: &[DriverUsage], theme: &Theme) -> (Vec<PickerItem>, Vec<UsageRow>) {
    let mut items = Vec::new();
    let mut rows = Vec::new();
    for (r_idx, report) in reports.iter().enumerate() {
        let section = report.driver.label();
        if report.failed {
            let idx = rows.len();
            rows.push(UsageRow {
                report: r_idx,
                kind: RowKind::Failed,
            });
            items.push(PickerItem {
                key: PickerKey::UsageGroup(idx),
                label: "unavailable".into(),
                haystack: format!(
                    "{section} unavailable failed {}",
                    report.note.as_deref().unwrap_or("")
                ),
                detail: "failed".into(),
                glyph: "✗",
                color: theme.error,
                depth: 0,
                section,
            });
            continue;
        }
        if report.groups.is_empty() {
            let idx = rows.len();
            rows.push(UsageRow {
                report: r_idx,
                kind: RowKind::Empty,
            });
            items.push(PickerItem {
                key: PickerKey::UsageGroup(idx),
                label: "no limits reported".into(),
                haystack: format!(
                    "{section} no limits reported {} {}",
                    report.plan.as_deref().unwrap_or(""),
                    report.note.as_deref().unwrap_or("")
                ),
                detail: report.plan.clone().unwrap_or_default(),
                glyph: "○",
                color: theme.dim,
                depth: 0,
                section,
            });
            continue;
        }
        for (g_idx, group) in report.groups.iter().enumerate() {
            let max_pct = group
                .windows
                .iter()
                .map(|w| usage::pct(w.used_fraction))
                .max();
            let detail = max_pct.map_or_else(String::new, |p| format!("{p}%"));
            let (glyph, color) = match max_pct {
                Some(p) => {
                    let glyph = if p >= 90 {
                        "⚠"
                    } else if p >= 70 {
                        "◐"
                    } else {
                        "●"
                    };
                    (glyph, theme.usage(p).fg.unwrap_or(theme.dim))
                }
                None => ("○", theme.dim),
            };
            let idx = rows.len();
            rows.push(UsageRow {
                report: r_idx,
                kind: RowKind::Group(g_idx),
            });
            items.push(PickerItem {
                key: PickerKey::UsageGroup(idx),
                label: group.name.clone(),
                haystack: format!("{section} {} {} {detail}", group.name, group.models),
                detail,
                glyph,
                color,
                depth: 0,
                section,
            });
        }
    }
    (items, rows)
}

fn driver_plan_line(report: &DriverUsage, theme: &Theme, accent: bool) -> Line<'static> {
    let driver_style = if accent { theme.accent() } else { theme.dim() };
    let mut spans = vec![Span::styled(report.driver.label(), driver_style)];
    if let Some(plan) = &report.plan {
        spans.push(Span::styled(format!(" · plan: {plan}"), theme.dim()));
    }
    Line::from(spans)
}

fn usage_bar(used_fraction: f64) -> String {
    let filled =
        ((used_fraction.clamp(0.0, 1.0) * BAR_WIDTH as f64).round() as usize).min(BAR_WIDTH);
    format!("{}{}", "█".repeat(filled), "░".repeat(BAR_WIDTH - filled))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use dovetail_api::{
        model::DriverKind,
        status::{LimitWindow, UsageGroup},
    };

    // 2026-11-09 12:00 UTC, a Monday.
    const NOW_MS: i64 = 1_794_225_600_000;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn claude_report() -> DriverUsage {
        DriverUsage {
            driver: DriverKind::Claude,
            plan: Some("pro".into()),
            groups: vec![UsageGroup {
                name: "Claude Code".into(),
                models: String::new(),
                windows: vec![
                    LimitWindow {
                        name: "5-hour".into(),
                        used_fraction: 0.43,
                        resets_at_ms: Some(NOW_MS + 2 * 3_600_000),
                    },
                    LimitWindow {
                        name: "Weekly".into(),
                        used_fraction: 0.91,
                        resets_at_ms: Some(NOW_MS + 3 * 86_400_000),
                    },
                ],
            }],
            note: Some("extra usage off (out of credits)".into()),
            failed: false,
        }
    }

    fn agy_report() -> DriverUsage {
        DriverUsage {
            driver: DriverKind::Agy,
            plan: None,
            groups: vec![
                UsageGroup {
                    name: "Gemini Models".into(),
                    models: "Gemini 4 Neon, Gemini 3.8 Flash".into(),
                    windows: vec![LimitWindow {
                        name: "5-hour".into(),
                        used_fraction: 0.72,
                        resets_at_ms: Some(NOW_MS + 3_600_000),
                    }],
                },
                UsageGroup {
                    name: "Claude and GPT models".into(),
                    models: "Claude Opus 5.5".into(),
                    windows: vec![LimitWindow {
                        name: "Weekly".into(),
                        used_fraction: 0.25,
                        resets_at_ms: Some(NOW_MS + 4 * 86_400_000),
                    }],
                },
            ],
            note: None,
            failed: false,
        }
    }

    #[test]
    fn loading_state_shows_loading_and_hint() {
        let menu = UsageMenu::new();
        let text: Vec<String> = menu
            .preview(60, NOW_MS, &TimeZone::UTC, &Theme::default())
            .into_iter()
            .map(|l| l.to_string())
            .collect();
        assert!(text.iter().any(|l| l.contains("loading usage limits…")));
        assert!(text.iter().any(|l| l.contains("tab refresh · esc close")));
    }

    #[test]
    fn rows_group_by_driver_and_show_max_percentage_and_colours() {
        let theme = Theme::default();
        let items = build_rows(&[claude_report(), agy_report()], &theme).0;
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].section, "Claude Code");
        assert_eq!(items[0].label, "Claude Code");
        assert_eq!(items[0].detail, "91%");
        assert_eq!(items[0].glyph, "⚠");
        assert_eq!(items[0].color, theme.error);

        assert_eq!(items[1].section, "agy");
        assert_eq!(items[1].label, "Gemini Models");
        assert_eq!(items[1].detail, "72%");
        assert_eq!(items[1].glyph, "◐");
        assert_eq!(items[1].color, theme.needs_you);

        assert_eq!(items[2].section, "agy");
        assert_eq!(items[2].label, "Claude and GPT models");
        assert_eq!(items[2].detail, "25%");
        assert_eq!(items[2].glyph, "●");
        assert_eq!(items[2].color, theme.dim);
    }

    #[test]
    fn preview_shows_windows_bars_reset_times_plan_models_and_note() {
        let theme = Theme::default();
        let mut menu = UsageMenu::new();
        menu.set_reports(vec![claude_report(), agy_report()], &theme);

        let claude_preview: Vec<String> = menu
            .preview(70, NOW_MS, &TimeZone::UTC, &theme)
            .into_iter()
            .map(|l| l.to_string())
            .collect();
        assert!(claude_preview.iter().any(|l| l.contains("Claude Code")));
        assert!(claude_preview.iter().any(|l| l.contains("plan: pro")));
        assert!(
            claude_preview
                .iter()
                .any(|l| l.contains("5-hour") && l.contains("43%") && l.contains("resets 14:00")),
            "{claude_preview:?}"
        );
        assert!(
            claude_preview
                .iter()
                .any(|l| l.contains("Weekly") && l.contains("91%") && l.contains("resets Thu")),
            "{claude_preview:?}"
        );
        assert!(
            claude_preview
                .iter()
                .any(|l| l.contains("extra usage off (out of credits)"))
        );

        // Move to agy's first group and verify models are rendered.
        menu.key(key(KeyCode::Down));
        let agy_preview: Vec<String> = menu
            .preview(70, NOW_MS, &TimeZone::UTC, &theme)
            .into_iter()
            .map(|l| l.to_string())
            .collect();
        assert!(agy_preview.iter().any(|l| l.contains("Gemini Models")));
        assert!(
            agy_preview
                .iter()
                .any(|l| l.contains("Gemini 4 Neon, Gemini 3.8 Flash"))
        );
        assert!(
            agy_preview
                .iter()
                .any(|l| l.contains("5-hour") && l.contains("72%") && l.contains("resets 13:00"))
        );
    }

    #[test]
    fn failed_report_and_empty_report_show_single_row_with_reason() {
        let theme = Theme::default();
        let mut menu = UsageMenu::new();
        menu.set_reports(
            vec![
                DriverUsage::failed(DriverKind::Claude, "claude exited 1: not logged in"),
                DriverUsage {
                    driver: DriverKind::Agy,
                    plan: Some("free".into()),
                    groups: Vec::new(),
                    note: Some("no quota pools active".into()),
                    failed: false,
                },
            ],
            &theme,
        );
        let shown = menu.picker().shown();
        assert_eq!(shown.len(), 2);
        assert_eq!(shown[0].label, "unavailable");
        assert_eq!(shown[0].detail, "failed");
        assert_eq!(shown[0].glyph, "✗");
        assert_eq!(shown[0].color, theme.error);
        assert_eq!(shown[1].label, "no limits reported");

        let fail_preview: Vec<String> = menu
            .preview(70, NOW_MS, &TimeZone::UTC, &theme)
            .into_iter()
            .map(|l| l.to_string())
            .collect();
        assert!(
            fail_preview
                .iter()
                .any(|l| l.contains("claude exited 1: not logged in"))
        );

        menu.key(key(KeyCode::Down));
        let empty_preview: Vec<String> = menu
            .preview(70, NOW_MS, &TimeZone::UTC, &theme)
            .into_iter()
            .map(|l| l.to_string())
            .collect();
        assert!(
            empty_preview
                .iter()
                .any(|l| l.contains("no limits reported"))
        );
        assert!(
            empty_preview
                .iter()
                .any(|l| l.contains("no quota pools active"))
        );
    }

    #[test]
    fn tab_refreshes_and_esc_closes() {
        let mut menu = UsageMenu::new();
        assert_eq!(menu.key(key(KeyCode::Tab)), UsageStep::Refresh);
        assert_eq!(menu.key(key(KeyCode::Enter)), UsageStep::Pending);
        assert_eq!(menu.key(key(KeyCode::Esc)), UsageStep::Close);
    }
}
