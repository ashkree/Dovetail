//! The model picker (tasks #39, #46): `:model [name]` and `/model [name]`. It wraps the one
//! [`Picker`], grouping rows by driver (`Claude Code`, then `agy`) with one row per `(model,
//! effort)` pair. Picking an own-driver row applies [`dovetail_api::Command::SetModel`]; picking
//! another driver's row on the coordinator opens the switch confirm modal. Non-coordinator
//! sessions hide the other driver's rows.

use crossterm::event::KeyEvent;
use dovetail_api::{
    Agent, AgentId, AgentKind, DriverKind, ModelOption, ReasoningEffort, model_option_index,
};
use ratatui::text::{Line, Span};

use crate::fmt;
use crate::picker::{Picker, PickerItem, PickerKey, PickerOutcome};
use crate::theme::Theme;

/// What a key did to the model picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelOutcome {
    /// Still open.
    Pending,
    /// Closed without a change.
    Cancel,
    /// Switch `agent` to `model` at `effort` on its current driver.
    SetModel {
        /// The session.
        agent: AgentId,
        /// The model, as [`ModelOption::name`].
        model: String,
        /// The effort; `None` when the model has no levels.
        effort: Option<ReasoningEffort>,
    },
    /// Start a new coordinator session on `driver` with `model` at `effort`.
    SwitchDriver {
        /// The coordinator.
        agent: AgentId,
        /// The driver to switch to.
        driver: DriverKind,
        /// The model, as that driver's [`ModelOption::name`].
        model: String,
        /// The effort; `None` when the model has no levels.
        effort: Option<ReasoningEffort>,
    },
}

/// The open model picker.
pub struct ModelPicker {
    agent: AgentId,
    agent_kind: AgentKind,
    agent_driver: DriverKind,
    options: Vec<ModelOption>,
    picker: Picker,
}

impl ModelPicker {
    /// A picker for `agent` over `options`, pre-filtered by `preselect` when non-empty.
    pub fn new(
        agent: &Agent,
        options: Vec<ModelOption>,
        preselect: Option<&str>,
        theme: &Theme,
    ) -> Self {
        let items = model_items(agent, &options, theme);
        let mut picker = Picker::new(format!("model · {}", agent.name), items).preview_from_top();
        if let Some(query) = preselect.map(str::trim).filter(|q| !q.is_empty()) {
            picker = picker.with_query(query);
        }
        Self {
            agent: agent.id.clone(),
            agent_kind: agent.kind,
            agent_driver: agent.driver,
            options,
            picker,
        }
    }

    /// The underlying picker, for rendering.
    pub fn picker(&self) -> &Picker {
        &self.picker
    }

    /// Handles one key.
    pub fn key(&mut self, event: KeyEvent) -> ModelOutcome {
        match self.picker.key(event) {
            PickerOutcome::Pending => ModelOutcome::Pending,
            PickerOutcome::Closed => ModelOutcome::Cancel,
            PickerOutcome::Chosen(PickerKey::Model {
                driver,
                model,
                effort,
            }) => {
                if driver == self.agent_driver {
                    ModelOutcome::SetModel {
                        agent: self.agent.clone(),
                        model,
                        effort,
                    }
                } else if self.agent_kind == AgentKind::Coordinator {
                    ModelOutcome::SwitchDriver {
                        agent: self.agent.clone(),
                        driver,
                        model,
                        effort,
                    }
                } else {
                    ModelOutcome::Cancel
                }
            }
            PickerOutcome::Chosen(_) => ModelOutcome::Pending,
        }
    }

    /// The preview pane's lines for the highlighted model and effort.
    pub fn preview(&self, width: usize, theme: &Theme) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        match self.picker.selected().map(|item| &item.key) {
            Some(PickerKey::Model {
                driver,
                model,
                effort,
            }) => {
                let option = self
                    .options
                    .iter()
                    .find(|o| o.driver == *driver && o.name == *model);
                let full_name = option
                    .and_then(|o| o.resolved.as_deref())
                    .unwrap_or(model.as_str());
                let mut head = vec![Span::styled(full_name.to_owned(), theme.accent())];
                if full_name != model {
                    head.push(Span::styled(format!("  ({model})"), theme.dim()));
                }
                out.push(Line::from(head));
                out.push(Line::from(Span::styled(driver.label(), theme.dim())));
                if let Some(effort) = effort {
                    out.push(Line::from(vec![
                        Span::styled("effort: ", theme.dim()),
                        Span::styled(effort.as_str(), theme.body()),
                    ]));
                }
                out.push(Line::default());
                let efforts_text = match option.and_then(|o| o.efforts.as_ref()) {
                    Some(v) if v.is_empty() => "supported efforts: none".to_owned(),
                    Some(v) => format!(
                        "supported efforts: {}",
                        v.iter().map(|e| e.as_str()).collect::<Vec<_>>().join(", ")
                    ),
                    None => format!(
                        "supported efforts: {}",
                        ReasoningEffort::ALL
                            .into_iter()
                            .map(ReasoningEffort::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                };
                for line in fmt::wrap(&efforts_text, width) {
                    out.push(Line::from(Span::styled(line, theme.body())));
                }
                if *driver != self.agent_driver {
                    out.push(Line::default());
                    let switch_note =
                        format!("Starts a new coordinator session on {}.", driver.label());
                    for line in fmt::wrap(&switch_note, width) {
                        out.push(Line::from(Span::styled(line, theme.fg(theme.needs_you))));
                    }
                }
            }
            _ => {
                out.push(Line::from(Span::styled("no matching models", theme.dim())));
            }
        }
        if self.agent_kind != AgentKind::Coordinator {
            out.push(Line::default());
            for line in fmt::wrap("Only the coordinator switches driver.", width) {
                out.push(Line::from(Span::styled(line, theme.dim())));
            }
        } else if !self.options.iter().any(|o| o.driver == DriverKind::Agy) {
            out.push(Line::default());
            for line in fmt::wrap("agy models appear once an agy session has run.", width) {
                out.push(Line::from(Span::styled(line, theme.dim())));
            }
        }
        out
    }
}

/// Picker rows grouped by [`DriverKind::ALL`], one row per `(model, effort)`.
pub fn model_items(agent: &Agent, options: &[ModelOption], theme: &Theme) -> Vec<PickerItem> {
    let current_idx = agent
        .model
        .as_deref()
        .and_then(|m| model_option_index(options, agent.driver, m));
    let mut items = Vec::new();
    for driver in DriverKind::ALL {
        if agent.kind != AgentKind::Coordinator && driver != agent.driver {
            continue;
        }
        for (idx, opt) in options
            .iter()
            .enumerate()
            .filter(|(_, o)| o.driver == driver)
        {
            for effort in option_efforts(opt) {
                let is_current = current_idx == Some(idx) && effort == agent.effort;
                let label = match effort {
                    Some(e) => format!("{} · {}", opt.name, e.as_str()),
                    None => opt.name.clone(),
                };
                let haystack = format!(
                    "{} {} {} {}",
                    driver.label(),
                    opt.name,
                    opt.resolved.as_deref().unwrap_or(""),
                    effort.map_or("", ReasoningEffort::as_str),
                );
                let resolved_diff = opt.resolved.as_deref().filter(|r| *r != opt.name);
                let detail = match (is_current, resolved_diff) {
                    (true, Some(r)) => format!("{r}  now"),
                    (true, None) => "now".into(),
                    (false, Some(r)) => r.to_owned(),
                    (false, None) => String::new(),
                };
                let (glyph, color) = if is_current {
                    ("●", theme.ok)
                } else {
                    (" ", theme.dim)
                };
                items.push(PickerItem {
                    key: PickerKey::Model {
                        driver,
                        model: opt.name.clone(),
                        effort,
                    },
                    label,
                    haystack,
                    detail,
                    glyph,
                    color,
                    depth: 0,
                    section: driver.label(),
                });
            }
        }
    }
    items
}

fn option_efforts(option: &ModelOption) -> Vec<Option<ReasoningEffort>> {
    match &option.efforts {
        Some(v) if v.is_empty() => vec![None],
        Some(v) => v.iter().copied().map(Some).collect(),
        None => ReasoningEffort::ALL.into_iter().map(Some).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};
    use dovetail_api::{AgentStatus, PermissionMode, SessionLock};

    fn agent_fixture(kind: AgentKind, driver: DriverKind) -> Agent {
        Agent {
            id: AgentId(if kind == AgentKind::Coordinator {
                "coordinator".into()
            } else {
                "slice-1".into()
            }),
            name: if kind == AgentKind::Coordinator {
                "coordinator".into()
            } else {
                "slice-1".into()
            },
            kind,
            parent: None,
            task: None,
            worktree: None,
            status: AgentStatus::Idle,
            status_line: String::new(),
            memory_mb: None,
            cost_usd: 0.0,
            permission_mode: PermissionMode::Auto,
            context_pct: None,
            progress: None,
            last_activity_ms: 0,
            lock: SessionLock::Headless,
            model: Some("claude-opus-5-5".into()),
            effort: Some(ReasoningEffort::High),
            subagent_tokens: 0,
            tool: None,
            driver,
        }
    }

    fn options() -> Vec<ModelOption> {
        vec![
            ModelOption {
                name: "opus".into(),
                resolved: Some("claude-opus-5-5".into()),
                efforts: Some(vec![ReasoningEffort::Medium, ReasoningEffort::High]),
                driver: DriverKind::Claude,
            },
            ModelOption {
                name: "old".into(),
                resolved: None,
                efforts: Some(Vec::new()),
                driver: DriverKind::Claude,
            },
            ModelOption {
                name: "unknown-efforts".into(),
                resolved: None,
                efforts: None,
                driver: DriverKind::Claude,
            },
            ModelOption {
                name: "gemini-3.8-flash".into(),
                resolved: Some("Gemini 3.8 Flash".into()),
                efforts: Some(vec![ReasoningEffort::Low, ReasoningEffort::High]),
                driver: DriverKind::Agy,
            },
        ]
    }

    #[test]
    fn rows_expand_efforts_per_spec_and_mark_current_session_row() {
        let agent = agent_fixture(AgentKind::Coordinator, DriverKind::Claude);
        let items = model_items(&agent, &options(), &Theme::default());
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "opus · medium",
                "opus · high",
                "old",
                "unknown-efforts · low",
                "unknown-efforts · medium",
                "unknown-efforts · high",
                "unknown-efforts · xhigh",
                "unknown-efforts · max",
                "gemini-3.8-flash · low",
                "gemini-3.8-flash · high",
            ]
        );
        let sections: Vec<&str> = items.iter().map(|i| i.section).collect();
        assert!(sections[..8].iter().all(|s| *s == "Claude Code"));
        assert!(sections[8..].iter().all(|s| *s == "agy"));
        assert_eq!(items[1].glyph, "●");
        assert!(items[1].detail.contains("now"));
        assert_eq!(items[0].glyph, " ");
    }

    #[test]
    fn slices_hide_other_driver_rows_and_note_coordinator_only_in_preview() {
        let slice = agent_fixture(AgentKind::Slice, DriverKind::Claude);
        let picker = ModelPicker::new(&slice, options(), None, &Theme::default());
        assert!(picker.picker().shown().iter().all(|i| matches!(
            i.key,
            PickerKey::Model {
                driver: DriverKind::Claude,
                ..
            }
        )));
        let preview: Vec<String> = picker
            .preview(60, &Theme::default())
            .into_iter()
            .map(|l| l.to_string())
            .collect();
        assert!(
            preview
                .iter()
                .any(|l| l.contains("Only the coordinator switches driver."))
        );
    }

    #[test]
    fn coordinator_preview_notes_empty_agy_catalog_and_driver_switch_warning() {
        let coord = agent_fixture(AgentKind::Coordinator, DriverKind::Claude);
        let claude_only: Vec<ModelOption> = options()
            .into_iter()
            .filter(|o| o.driver == DriverKind::Claude)
            .collect();
        let no_agy = ModelPicker::new(&coord, claude_only, None, &Theme::default());
        let preview: Vec<String> = no_agy
            .preview(60, &Theme::default())
            .into_iter()
            .map(|l| l.to_string())
            .collect();
        assert!(
            preview
                .iter()
                .any(|l| l.contains("agy models appear once an agy session has run")),
            "{preview:?}"
        );

        let mut with_agy = ModelPicker::new(&coord, options(), Some("gemini"), &Theme::default());
        let preview_agy: Vec<String> = with_agy
            .preview(60, &Theme::default())
            .into_iter()
            .map(|l| l.to_string())
            .collect();
        assert!(
            preview_agy
                .iter()
                .any(|l| l.contains("Starts a new coordinator session on agy.")),
            "{preview_agy:?}"
        );
        assert!(
            !preview_agy
                .iter()
                .any(|l| l.contains("agy models appear once")),
            "{preview_agy:?}"
        );
        assert_eq!(
            with_agy.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            ModelOutcome::SwitchDriver {
                agent: AgentId("coordinator".into()),
                driver: DriverKind::Agy,
                model: "gemini-3.8-flash".into(),
                effort: Some(ReasoningEffort::Low),
            }
        );
    }
}
