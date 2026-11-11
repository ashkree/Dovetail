//! Projects: the project switcher (`<leader>pp`, `:project`), opening another project (B24), and
//! `:project new <name> --research` (research-only projects, decided 2026-11-11).

use dovetail_api::{AgentStatus, Command, ProjectId, ProjectKind, UNREGISTERED_WARNING};

use super::overlay::{Overlay, Purpose};
use super::{App, Confirm, Then};
use crate::picker::Picker;
use crate::switcher;

/// What `:project new` says when it is not given a name and `--research`.
const NEW_USAGE: &str = "usage: :project new <name> --research (a code project: open dovetail in \
                         its folder and :project add)";

impl App {
    /// The open project is a research project: no repo, no worktrees, no slices.
    pub(super) fn in_research_project(&self) -> bool {
        self.snap
            .project
            .as_ref()
            .is_some_and(|p| p.kind == ProjectKind::Research)
    }

    /// `<leader>pp`: every registered project, research ones marked.
    pub(super) fn open_project_picker(&mut self) {
        let items = switcher::project_items(&self.snap, &self.theme);
        if items.is_empty() {
            self.note = Some(
                "no projects yet: :project add keeps this folder, :project new <name> --research \
                 makes a research project"
                    .into(),
            );
            return;
        }
        self.open_overlay(Overlay::Picker(
            Picker::new("Projects", items),
            Purpose::Open,
        ));
    }

    /// A project was picked: ask, then the core quits and the app starts again on it.
    pub(super) fn pick_project(&mut self, id: ProjectId) {
        let Some(project) = self.snap.projects.iter().find(|p| p.id == id) else {
            return;
        };
        if self.snap.project.as_ref().is_some_and(|p| p.id == id) {
            self.note = Some(format!("{} is already open", project.name));
            return;
        }
        let unregistered = self
            .snap
            .warnings
            .iter()
            .any(|w| w.id.0 == UNREGISTERED_WARNING);
        let running = self
            .snap
            .agents
            .iter()
            .any(|a| a.kind.is_session() && a.status == AgentStatus::Working);
        let why = if unregistered {
            "This folder is not a project yet, so its chat is lost. "
        } else if running {
            "A turn is running. "
        } else {
            ""
        };
        self.confirm = Some(Confirm {
            prompt: format!(
                "{why}Open {}? This project's sessions stop and resume when you come back.",
                project.name
            ),
            then: Then::Commands(vec![Command::OpenProject { id }]),
            otherwise: None,
        });
    }

    /// `:project new <name> --research`. Only research projects are made this way: a code
    /// project is a folder of the user's, added from inside it.
    pub(super) fn new_project(&mut self, arg: &str) {
        let mut research = false;
        let mut words = Vec::new();
        for word in arg.split_whitespace() {
            match word {
                "--research" => research = true,
                _ => words.push(word),
            }
        }
        let name = words.join(" ");
        if !research || name.is_empty() {
            self.note = Some(NEW_USAGE.into());
            return;
        }
        self.note = Some(format!(
            "creating research project {name}: <leader>pp opens it"
        ));
        self.outbox.push(Command::NewResearchProject { name });
    }
}
