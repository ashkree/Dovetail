//! Overlays: the pickers and quick capture float over the layout and take every key until they
//! close.

use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::{
    Command,
    activity::inbox::InboxId,
    activity::status::{DriverUsage, StatusCommand},
    agents::agent::AgentId,
    agents::mcp::McpCommand,
    agents::model::ModelCommand,
    agents::tangent::TangentCommand,
    research::ResearchCommand,
    tracking::project::{ProjectCommand, ProjectId},
    tracking::task::{TaskChange, TaskCommand, TaskId},
};

use super::App;
use super::ChatSite;
use super::confirm::{ConfirmOutcome, SwitchConfirm};
use crate::capture::{Capture, CaptureOutcome, LinkTarget};
use crate::keymap::help;
use crate::keymap::mode::Mode;
use crate::keymap::{Key, Scope};
use crate::layout::region::RegionId;
use crate::mcp_menu::{McpMenu, Step};
use crate::model_select::{ModelOutcome, ModelPicker};
use crate::picker::{Picker, PickerKey, PickerOutcome};
use crate::switcher;
use crate::usage_menu::{UsageMenu, UsageStep};

/// What choosing a picker row does.
pub(super) enum Purpose {
    /// Open what the row stands for.
    Open,
    /// The command picker `/` opened over the chat bar: a space hands the typing back to the bar,
    /// a choice puts the command in it.
    SlashBar,
    /// Relate this task to the chosen one, or unrelate them.
    Relate(TaskId),
    /// File this unfiled finding (an inbox item) under the chosen task.
    FileUnfiled(InboxId),
    /// Browse the chosen project's library in reading mode (`<leader>fr`).
    Browse,
    /// File this tangent into the chosen project (ADR 0017).
    FileTangent(TaskId),
    /// Start a task from this tangent in the chosen project.
    StartTangent(TaskId),
    /// Attach this tangent to the chosen task of the open project.
    AttachTangent(TaskId),
    /// Set the chosen profile in the starter recipe `:project add` just wrote (task #31).
    Profile {
        /// The project just added.
        project: ProjectId,
        /// Its starter recipe.
        path: PathBuf,
    },
}

/// The note when the profile picker keeps the default (task #31).
fn no_profile_note(path: &std::path::Path) -> String {
    format!("default profile kept; starter recipe at {}", path.display())
}

/// The chat bar an overlay was opened from while typing in it: where focus was, which chat view
/// and which agent. Closing the overlay hands typing back only if all three still hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChatMark {
    focus: RegionId,
    site: ChatSite,
    agent: Option<AgentId>,
}

/// What floats over the layout.
pub(super) enum Overlay {
    /// The agent switcher, the everything picker or the relate picker.
    Picker(Picker, Purpose),
    /// Quick capture.
    Capture(Capture),
    /// The MCP server menu of a session (task #44).
    Mcp(McpMenu),
    /// Account usage limits of Claude Code and agy (task #46).
    Usage(UsageMenu),
    /// The model picker (tasks #39, #46).
    Model(ModelPicker),
    /// Confirm modal before switching the coordinator's driver (task #46).
    Confirm(SwitchConfirm),
}

impl App {
    pub(super) fn open_overlay(&mut self, overlay: Overlay) {
        self.resolver.reset();
        self.overlay_chat = self.chat_mark();
        self.overlay = Some(overlay);
        // Typing goes to the overlay, so the status bar reads INSERT.
        self.mode = Mode::Insert;
    }

    /// The chat bar being typed in right now, if any.
    fn chat_mark(&self) -> Option<ChatMark> {
        let chat = self.chat_view();
        (self.mode == Mode::Insert && chat.is_chatting()).then(|| ChatMark {
            focus: self.regions.focus(),
            site: self.chat_site,
            agent: chat.agent().cloned(),
        })
    }

    /// Closes the overlay, back in Normal mode. Returns the chat bar it was opened from; give it
    /// to [`restore_chat`](Self::restore_chat) once what the overlay did is applied.
    fn close_overlay(&mut self) -> Option<ChatMark> {
        self.overlay = None;
        self.mode = Mode::Normal;
        self.overlay_chat.take()
    }

    /// Closes the overlay and hands typing back to the chat bar it was opened from, if unchanged.
    fn close_overlay_restoring_chat(&mut self) {
        let from = self.close_overlay();
        self.restore_chat(from);
    }

    /// Hands typing back to the chat bar an overlay was opened from, if it is still the one
    /// focused and open (choosing an agent or task may have moved focus elsewhere).
    fn restore_chat(&mut self, from: Option<ChatMark>) {
        if from.is_some() && from == self.chat_mark_ignoring_mode() {
            self.mode = Mode::Insert;
        }
    }

    /// [`chat_mark`](Self::chat_mark) without the mode test, for after a choice.
    fn chat_mark_ignoring_mode(&self) -> Option<ChatMark> {
        let chat = self.chat_view();
        chat.is_chatting().then(|| ChatMark {
            focus: self.regions.focus(),
            site: self.chat_site,
            agent: chat.agent().cloned(),
        })
    }

    /// `<leader>ss`: Dovetail's agents only.
    pub(super) fn open_switcher(&mut self) {
        let items = switcher::agent_items(&self.snap, self.now_ms, &self.theme);
        self.open_overlay(Overlay::Picker(Picker::new("Switch", items), Purpose::Open));
    }

    /// `/` at the start of the chat bar, `:skills`: the slash commands and skills of the agent
    /// the chat bar talks to, Dovetail's own marked.
    pub(super) fn open_command_picker(&mut self, from_slash: bool) {
        let Some(agent) = self.chat_target() else {
            return;
        };
        let commands = super::commands_for(&self.snap, &agent);
        let items = switcher::command_items(commands, &self.theme);
        let purpose = if from_slash {
            Purpose::SlashBar
        } else {
            Purpose::Open
        };
        self.open_overlay(Overlay::Picker(Picker::new("Skills", items), purpose));
    }

    /// `:model [name]`, `/model [name]`: the focused session's model and effort
    /// (task #39). A subagent's is its session's.
    pub(super) fn open_model_selector(&mut self, preselect: Option<&str>) {
        let Some((session, _)) = self.focused_agent().and_then(|id| self.session_of(&id)) else {
            self.note = Some("no session to switch".into());
            return;
        };
        self.open_model_selector_for(session, preselect);
    }

    /// The selector for `session`.
    pub(super) fn open_model_selector_for(&mut self, session: AgentId, preselect: Option<&str>) {
        let Some(agent) = self.snap.agents.iter().find(|a| a.id == session) else {
            return;
        };
        if self.snap.models.is_empty() {
            self.note = Some("no models to offer yet".into());
            return;
        }
        let picker = ModelPicker::new(agent, self.snap.models.clone(), preselect, &self.theme);
        self.open_overlay(Overlay::Model(picker));
    }

    /// `:mcp`, `/mcp`: the MCP servers of `agent`'s session (`None`: the focused one), asked for
    /// at once.
    pub(super) fn open_mcp_menu(&mut self, agent: Option<AgentId>) {
        let Some(agent) = agent.or_else(|| self.focused_agent()) else {
            self.note = Some("no session to show MCP servers for".into());
            return;
        };
        let Some((session, _)) = self.session_of(&agent) else {
            self.note = Some("no session to show MCP servers for".into());
            return;
        };
        let name = self
            .snap
            .agents
            .iter()
            .find(|a| a.id == session)
            .map_or_else(|| session.0.clone(), |a| a.name.clone());
        self.outbox.push(Command::Mcp(McpCommand::Request {
            agent: session.clone(),
            request: dovetail_api::agents::mcp::McpRequest::Status,
        }));
        self.open_overlay(Overlay::Mcp(McpMenu::new(session, name)));
    }

    /// The session's answer to a menu request: into the open menu, if it is that session's.
    pub(super) fn mcp_reply(
        &mut self,
        agent: &AgentId,
        request: &dovetail_api::agents::mcp::McpRequest,
        outcome: dovetail_api::agents::mcp::McpOutcome,
    ) {
        let Some(Overlay::Mcp(menu)) = &mut self.overlay else {
            return;
        };
        if menu.session() != agent {
            return;
        }
        let steps = menu.reply(request, outcome, &self.theme);
        self.mcp_steps(steps);
    }

    /// What the menu asked for: the session, the browser, or to close.
    fn mcp_steps(&mut self, steps: Vec<Step>) {
        for step in steps {
            match step {
                Step::Ask(request) => {
                    let Some(Overlay::Mcp(menu)) = &self.overlay else {
                        continue;
                    };
                    self.outbox.push(Command::Mcp(McpCommand::Request {
                        agent: menu.session().clone(),
                        request,
                    }));
                }
                Step::Open(url) => self.open_external(url),
                Step::Close => self.close_overlay_restoring_chat(),
            }
        }
    }

    /// `:usage`, `/usage`: account usage limits of Claude Code and agy (task #46).
    pub(super) fn open_usage_menu(&mut self) {
        self.outbox.push(Command::Status(StatusCommand::LoadUsage));
        self.open_overlay(Overlay::Usage(UsageMenu::new()));
    }

    /// Fills the open usage menu with the core's reports.
    pub(super) fn usage_loaded(&mut self, reports: Vec<DriverUsage>) {
        if let Some(Overlay::Usage(menu)) = &mut self.overlay {
            menu.set_reports(reports, &self.theme);
        }
    }

    /// `<leader><leader>`: tasks, agents and inbox items.
    pub(super) fn open_finder(&mut self) {
        let items = switcher::all_items(&self.snap, self.now_ms, &self.theme);
        self.open_overlay(Overlay::Picker(Picker::new("Find", items), Purpose::Open));
    }

    /// The scopes in force for the focused buffer, most specific first.
    pub(super) fn scopes(&self) -> [Scope; 2] {
        [Scope::Buffer(self.regions.focused_buffer()), Scope::Global]
    }

    /// `:keys`: every action with its keys here and its description.
    pub(super) fn open_keys_picker(&mut self) {
        let items = help::keys_items(&self.keymap, &self.scopes(), &self.theme);
        self.open_overlay(Overlay::Picker(Picker::new("Keys", items), Purpose::Open));
    }

    /// `:research`: the tasks that have research; choosing one opens its detail.
    pub(super) fn open_research_list(&mut self) {
        let items = switcher::research_items(&self.snap, &self.theme);
        if items.is_empty() {
            self.note = Some("no research yet · :round <task> <brief> opens a round".into());
            return;
        }
        self.open_overlay(Overlay::Picker(
            Picker::new("Research", items),
            Purpose::Open,
        ));
    }

    /// `R` in the task detail: pick a task to relate this one to (or unrelate).
    pub(super) fn open_relate_picker(&mut self, task: TaskId) {
        let items = switcher::relate_items(&self.snap, &self.theme, task);
        self.open_overlay(Overlay::Picker(
            Picker::new("Relate to", items),
            Purpose::Relate(task),
        ));
    }

    /// `y` or Enter on an unfiled finding: pick the task to file it under.
    pub(super) fn open_file_picker(&mut self, item: InboxId) {
        let items = switcher::task_items(&self.snap, &self.theme);
        if items.is_empty() {
            self.note = Some("there is no task to file it under: add one first (<leader>t)".into());
            return;
        }
        self.open_overlay(Overlay::Picker(
            Picker::new("File finding under", items),
            Purpose::FileUnfiled(item),
        ));
    }

    /// `<leader>t`: one line that becomes a task, linked to the task in focus.
    pub(super) fn open_capture(&mut self) {
        let link = self.focused_task().and_then(|id| {
            let task = self.snap.tasks.iter().find(|t| t.id == id)?;
            Some(LinkTarget {
                id,
                title: task.title.clone(),
            })
        });
        self.open_overlay(Overlay::Capture(Capture::new(link)));
    }

    pub(super) fn overlay_key(&mut self, event: KeyEvent) {
        let Some(overlay) = self.overlay.take() else {
            return;
        };
        match overlay {
            // A space in the `/` picker ends the filter: what was typed goes on in the bar.
            Overlay::Picker(picker, Purpose::SlashBar)
                if event.code == KeyCode::Char(' ')
                    && !event
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                let from = self.close_overlay();
                if from.is_some() && from == self.chat_mark_ignoring_mode() {
                    self.chat_view_mut().continue_slash(picker.query());
                }
                self.restore_chat(from);
            }
            Overlay::Picker(mut picker, purpose) => match picker.key(event) {
                PickerOutcome::Pending => self.overlay = Some(Overlay::Picker(picker, purpose)),
                PickerOutcome::Closed => {
                    self.close_overlay_restoring_chat();
                    if let Purpose::Profile { path, .. } = purpose {
                        self.note = Some(no_profile_note(&path));
                    }
                }
                PickerOutcome::Chosen(key) => {
                    let from = self.close_overlay();
                    match (purpose, key) {
                        (Purpose::Relate(task), PickerKey::Task(other)) => {
                            self.toggle_related(task, other)
                        }
                        (Purpose::Browse, PickerKey::Project(id)) => self.browse_project(id),
                        (Purpose::FileUnfiled(item), PickerKey::Task(task)) => {
                            self.outbox
                                .push(Command::Research(ResearchCommand::FileUnfiled {
                                    item,
                                    task,
                                }));
                            self.note = Some(format!("filing the finding under #{}", task.0));
                        }
                        (Purpose::FileTangent(tangent), PickerKey::Project(project)) => {
                            self.note = Some(format!("filing the tangent into {}", project.0));
                            self.outbox
                                .push(Command::Tangent(TangentCommand::File { tangent, project }));
                        }
                        (Purpose::StartTangent(tangent), PickerKey::Project(project)) => {
                            self.note = Some(format!("starting a task in {}", project.0));
                            self.outbox
                                .push(Command::Tangent(TangentCommand::StartFrom {
                                    tangent,
                                    project,
                                }));
                        }
                        (Purpose::AttachTangent(tangent), PickerKey::Task(task)) => {
                            self.note = Some(format!("attaching the tangent to #{}", task.0));
                            self.outbox
                                .push(Command::Tangent(TangentCommand::Attach { tangent, task }));
                        }
                        (Purpose::Profile { project, path }, PickerKey::Profile(profile)) => {
                            match profile {
                                Some(profile) => {
                                    self.note = Some(format!(
                                        "profile {profile} set in {}",
                                        path.display()
                                    ));
                                    self.outbox.push(Command::Project(
                                        ProjectCommand::SetRecipeProfile { project, profile },
                                    ));
                                }
                                None => self.note = Some(no_profile_note(&path)),
                            }
                        }
                        (_, key) => self.choose(key),
                    }
                    self.restore_chat(from);
                }
            },
            Overlay::Mcp(mut menu) => {
                let steps = menu.key(event);
                self.overlay = Some(Overlay::Mcp(menu));
                self.mcp_steps(steps);
            }
            Overlay::Usage(mut menu) => match menu.key(event) {
                UsageStep::Pending => self.overlay = Some(Overlay::Usage(menu)),
                UsageStep::Refresh => {
                    self.outbox.push(Command::Status(StatusCommand::LoadUsage));
                    self.overlay = Some(Overlay::Usage(menu));
                }
                UsageStep::Close => self.close_overlay_restoring_chat(),
            },
            Overlay::Model(mut picker) => match picker.key(event) {
                ModelOutcome::Pending => self.overlay = Some(Overlay::Model(picker)),
                ModelOutcome::Cancel => self.close_overlay_restoring_chat(),
                ModelOutcome::SetModel {
                    agent,
                    model,
                    effort,
                } => {
                    self.close_overlay_restoring_chat();
                    let level =
                        effort.map_or(String::new(), |e| format!(" at {} effort", e.as_str()));
                    self.note = Some(format!("switching to {model}{level}…"));
                    self.outbox.push(Command::Model(ModelCommand::Set {
                        agent,
                        model,
                        effort,
                    }));
                }
                ModelOutcome::SwitchDriver {
                    agent,
                    driver,
                    model,
                    effort,
                } => {
                    self.mode = Mode::Normal;
                    self.overlay = Some(Overlay::Confirm(SwitchConfirm::new(
                        agent, driver, model, effort,
                    )));
                }
            },
            Overlay::Confirm(mut confirm) => {
                let action =
                    self.keymap
                        .chord(Mode::Normal, Scope::Confirm, Key::from_event(event));
                match action.map_or(ConfirmOutcome::Pending, |a| confirm.on_action(a)) {
                    ConfirmOutcome::Pending => self.overlay = Some(Overlay::Confirm(confirm)),
                    ConfirmOutcome::Cancel => self.close_overlay_restoring_chat(),
                    ConfirmOutcome::Confirmed(cmd) => {
                        self.close_overlay_restoring_chat();
                        self.note = Some(format!(
                            "switching coordinator to {}…",
                            confirm.driver().label()
                        ));
                        self.outbox.push(cmd);
                    }
                }
            }
            Overlay::Capture(mut capture) => match capture.key(event) {
                CaptureOutcome::Pending => self.overlay = Some(Overlay::Capture(capture)),
                CaptureOutcome::Cancel => self.close_overlay_restoring_chat(),
                CaptureOutcome::Save { open_detail } => {
                    self.close_overlay_restoring_chat();
                    match capture.build() {
                        Some(task) => {
                            if open_detail {
                                self.open_after_create = Some(task.title.clone());
                            }
                            self.outbox.push(Command::Task(TaskCommand::Create(task)));
                        }
                        None => self.note = Some("a task needs a title".into()),
                    }
                }
            },
        }
    }

    fn toggle_related(&mut self, task: TaskId, other: TaskId) {
        let linked = self
            .snap
            .tasks
            .iter()
            .find(|t| t.id == task)
            .is_some_and(|t| t.related.contains(&other));
        let change = if linked {
            TaskChange::Unrelate(other)
        } else {
            TaskChange::Relate(other)
        };
        self.outbox.push(Command::Task(TaskCommand::Update {
            id: task,
            patch: vec![change],
        }));
    }

    /// `:project add` wrote a starter recipe (task #31): offer config.toml's profiles, the
    /// default first. No profiles: just say where the recipe is.
    pub(super) fn offer_profile(
        &mut self,
        project: ProjectId,
        name: &str,
        path: PathBuf,
        profiles: &[String],
    ) {
        if profiles.is_empty() {
            self.note = Some(format!("starter recipe written: {}", path.display()));
            return;
        }
        let items = switcher::profile_items(profiles, &self.theme);
        self.open_overlay(Overlay::Picker(
            Picker::new(format!("Profile for {name}"), items),
            Purpose::Profile { project, path },
        ));
    }

    /// Opens what a picker row stands for.
    fn choose(&mut self, key: PickerKey) {
        match key {
            // Handled directly by the picker's own overlay.
            PickerKey::Profile(_)
            | PickerKey::McpServer(_)
            | PickerKey::Model { .. }
            | PickerKey::UsageGroup(_) => {}
            // `:keys`: run what can run from here, otherwise the picker just closed.
            PickerKey::Action(action) => {
                if help::runnable(&self.keymap, &self.scopes(), action) {
                    self.run(action);
                } else {
                    self.note = Some(format!("{}: works in another view", action.describe()));
                }
            }
            PickerKey::Agent(id) => self.open_agent_in_main(id),
            PickerKey::Task(id) => self.open_task(id, false),
            PickerKey::Project(id) => self.pick_project(id),
            PickerKey::Inbox(id) => {
                self.focus_inbox();
                self.buffers.inbox.select_item(&self.snap, id);
            }
            PickerKey::Command(name) => {
                if let Some(agent) = self.chat_target() {
                    self.start_chat(agent);
                    self.chat_view_mut().insert_command(&name);
                }
            }
        }
    }
}
