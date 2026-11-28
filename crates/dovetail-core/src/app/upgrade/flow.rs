//! `:upgrade` inside the core (ADR 0014): the background build, the "Upgrade ready" item in
//! Needs you, and the restart once every agent is idle.
//!
//! One build at a time; one item at a time. `y` on the item does not resolve it: it starts the
//! wait for idle, and the item then lists the agents it waits for. When none is left the core
//! quits the normal way (sessions end and resume as after any restart), tells the UI to quit
//! ([`AppEvent::UpgradeRestarting`]) and raises the flag the binary reads after the core stopped:
//! swap the staged build in, then `exec` it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use dovetail_api::{
    Event,
    activity::{
        feed::ActivityKind,
        inbox::{InboxEvent, InboxId, InboxKind, PermissionAnswer, UPGRADE_READY_PERMISSION},
    },
    agents::agent::AgentId,
    app::AppEvent,
};
use serde_json::json;

use super::{BuildError, BuildRequest, Ready, Source, Staging, UpgradeError, is_installed_binary};
use crate::engine::{COORDINATOR, Core, Input};
use crate::inbox::Waiter;
use crate::socket::HandlerError;
use crate::storage::now_ms;

/// The core's upgrade state.
#[derive(Debug, Default)]
pub(crate) struct UpgradeState {
    /// Cancels the running build (the app is quitting); `None` when none runs.
    pub(crate) building: Option<Arc<AtomicBool>>,
    /// The "Upgrade ready" item in Needs you.
    item: Option<InboxId>,
    /// `y` was pressed: restart as soon as every agent is idle.
    restarting: bool,
    /// The agents the item last said it waits for.
    waiting: Option<Vec<String>>,
    /// When `y` next looks for other projects' apps that stopped (they send this core no input).
    recheck: Option<tokio::time::Instant>,
    /// Raised when the app quits to restart into the new build; the binary reads it.
    pub(crate) restart: Arc<AtomicBool>,
}

impl UpgradeState {
    /// When to look again at the other apps `y` waits for.
    pub(crate) fn recheck_at(&self) -> Option<tokio::time::Instant> {
        self.recheck
    }
}

/// How often `y` looks again at the other projects' apps it waits for.
const APP_RECHECK: std::time::Duration = std::time::Duration::from_secs(2);

impl Core {
    /// Other Dovetail apps (other projects') whose socket answers: the swap would replace the
    /// binary under them, so `y` waits for them too.
    fn other_apps(&self) -> Vec<String> {
        super::running_blockers(
            &self
                .options
                .upgrade
                .as_ref()
                .map(|u| u.install.clone())
                .unwrap_or_default(),
            &self.paths.runtime,
            std::path::Path::new("/nonexistent"),
            std::process::id(),
        )
        .into_iter()
        .map(std::path::PathBuf::from)
        .filter(|p| p != &self.mcp.socket)
        .filter_map(|p| {
            p.file_stem()
                .map(|s| format!("app {}", s.to_string_lossy()))
        })
        .collect()
    }

    /// Why this run cannot upgrade, if it cannot: upgrades are off, or this is not the
    /// installed binary (a dev build in a slice must not replace the one the user works in).
    fn upgrade_refusal(&self) -> Option<String> {
        // The build runs in the open project's first repo, and a research project has none.
        if self.tracking.project.is_research() {
            return Some(
                "a research project has no repo to build from: :upgrade in a code project".into(),
            );
        }
        let Some(options) = &self.options.upgrade else {
            return Some("upgrade is not available in this run".into());
        };
        if is_installed_binary(&self.options.exe, &options.install) {
            None
        } else {
            Some(UpgradeError::NotInstalled(options.install.clone()).to_string())
        }
    }

    /// `:upgrade`: build in the background; the result arrives as [`Input::UpgradeBuilt`].
    pub(crate) fn start_upgrade(&mut self, local: bool) {
        if self.quitting.is_some() {
            return;
        }
        if let Some(why) = self.upgrade_refusal() {
            return self.fail(why);
        }
        if self.app.upgrade.building.is_some() {
            return self.fail("an upgrade is already building");
        }
        if self.app.upgrade.item.is_some() {
            return self.fail("an upgrade is ready: answer it in Needs you (y: restart, d: later)");
        }
        let Some(options) = self.options.upgrade.clone() else {
            return;
        };
        let request = BuildRequest {
            // `upgrade_refusal` refused a project with no repo.
            repo: self.tracking.project.first_repo_path().unwrap_or_default(),
            source: if local {
                Source::Local
            } else {
                Source::OriginMain
            },
            config_dir: self.paths.config.clone(),
            recipe: self
                .tracking
                .project
                .registered()
                .map(|p| self.paths.recipe(&p.id)),
        };
        let staging = Staging::new(&self.paths);
        let cancel = Arc::new(AtomicBool::new(false));
        self.app.upgrade.building = Some(cancel.clone());
        self.emit(Event::App(AppEvent::UpgradeBuilding(true)));
        let what = if local { "the checkout" } else { "origin/main" };
        self.activity(
            ActivityKind::Lifecycle,
            None,
            format!("building the upgrade from {what} in the background"),
        );
        let inputs = self.inputs.clone();
        tokio::task::spawn_blocking(move || {
            let result = super::build(&*options.exec, &staging, &request, &cancel, now_ms());
            let _ = inputs.send(Input::UpgradeBuilt(result));
        });
    }

    /// The build finished: "Upgrade ready", or an Error item with the build's tail.
    pub(crate) fn on_upgrade_built(&mut self, result: Result<Ready, BuildError>) {
        self.app.upgrade.building = None;
        self.emit(Event::App(AppEvent::UpgradeBuilding(false)));
        if self.quitting.is_some() {
            return;
        }
        let coordinator = AgentId(COORDINATOR.into());
        match result {
            Ok(ready) => self.raise_upgrade(&ready),
            Err(BuildError::Cancelled) => {}
            Err(e) => {
                tracing::warn!("upgrade build failed: {e}");
                self.raise(
                    &coordinator,
                    InboxKind::Error {
                        text: format!("upgrade: {e}\n\nNothing was replaced."),
                    },
                    Waiter::None,
                );
            }
        }
    }

    /// `dovetail upgrade` built into the staging folder and hands it over the socket.
    pub(crate) fn take_staged_upgrade(&mut self) -> Result<(), HandlerError> {
        if self.quitting.is_some() {
            return Err(HandlerError::ShuttingDown);
        }
        if let Some(why) = self.upgrade_refusal() {
            return Err(HandlerError::Refused(why));
        }
        if self.app.upgrade.building.is_some() {
            return Err(HandlerError::Refused(
                "the app is building an upgrade itself".into(),
            ));
        }
        if self.app.upgrade.item.is_some() {
            return Ok(());
        }
        let ready = Staging::new(&self.paths)
            .ready()
            .ok_or_else(|| HandlerError::Refused("no staged build was found".into()))?;
        self.raise_upgrade(&ready);
        Ok(())
    }

    /// On open: a build staged earlier and never installed (the user chose `d` and the app
    /// crashed, or the quit could not swap it) is offered again.
    pub(crate) fn resume_upgrade(&mut self) {
        if self.upgrade_refusal().is_some() {
            return;
        }
        if let Some(ready) = Staging::new(&self.paths).ready() {
            self.raise_upgrade(&ready);
        }
    }

    fn raise_upgrade(&mut self, ready: &Ready) {
        let coordinator = AgentId(COORDINATOR.into());
        let id = self.raise(
            &coordinator,
            InboxKind::Permission {
                tool: UPGRADE_READY_PERMISSION.into(),
                input: json!({
                    "source": ready.source,
                    "differing": ready.differing,
                    "waiting": [],
                    "restarting": false,
                }),
                reason: None,
                suggestions: Vec::new(),
            },
            Waiter::UpgradeReady,
        );
        self.app.upgrade.item = Some(id);
        self.app.upgrade.restarting = false;
        self.app.upgrade.waiting = None;
        self.activity(
            ActivityKind::Lifecycle,
            None,
            format!("upgrade ready ({}): see Needs you", ready.source),
        );
    }

    /// `y` or `d` on "Upgrade ready" (the item is still in the inbox).
    pub(crate) fn answer_upgrade(&mut self, id: InboxId, answer: PermissionAnswer) {
        match answer {
            PermissionAnswer::Deny { .. } => {
                self.resolve(id, Some("kept for the next quit".into()));
                self.upgrade_item_gone();
                self.activity(
                    ActivityKind::Lifecycle,
                    None,
                    "upgrade kept: it is installed when Dovetail quits".into(),
                );
            }
            PermissionAnswer::Allow | PermissionAnswer::AllowAlways => {
                self.app.upgrade.restarting = true;
                // The first check tells the item what it waits for.
                self.app.upgrade.waiting = None;
                self.upgrade_check();
            }
        }
    }

    /// The item left Needs you without a restart (`d`, or dismissed): the build stays staged.
    pub(crate) fn upgrade_item_gone(&mut self) {
        self.app.upgrade.item = None;
        self.app.upgrade.restarting = false;
        self.app.upgrade.waiting = None;
    }

    /// The agents a restart would cut off: a session mid-turn or setting up, one held by a
    /// permission request, one native `claude` has.
    fn agents_not_idle(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .sessions
            .live
            .iter()
            .filter(|(_, live)| live.setting_up || live.turn.busy())
            .map(|(id, _)| self.name_of(id))
            .chain(
                self.agents
                    .rows
                    .iter()
                    .filter(|(_, row)| row.takeover.is_some())
                    .map(|(id, _)| self.name_of(id)),
            )
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// After `y`: keeps the item's list of waited-for agents current, and restarts when the
    /// list is empty. Cheap enough to run after every input.
    pub(crate) fn upgrade_check(&mut self) {
        if !self.app.upgrade.restarting || self.quitting.is_some() {
            return;
        }
        let Some(id) = self.app.upgrade.item else {
            return;
        };
        let mut waiting = self.agents_not_idle();
        let apps = self.other_apps();
        self.app.upgrade.recheck =
            (!apps.is_empty()).then(|| tokio::time::Instant::now() + APP_RECHECK);
        waiting.extend(apps);
        if self.app.upgrade.waiting.as_ref() != Some(&waiting) {
            self.app.upgrade.waiting = Some(waiting.clone());
            if let Some(entry) = self.activity.inbox.get_mut(&id)
                && let InboxKind::Permission { input, .. } = &mut entry.item.kind
            {
                input["waiting"] = json!(waiting);
                input["restarting"] = json!(true);
                let item = entry.item.clone();
                self.emit(Event::Inbox(InboxEvent::Upserted(item)));
            }
        }
        if waiting.is_empty() {
            self.resolve(id, Some("restarting".into()));
            self.upgrade_item_gone();
            self.app.upgrade.restart.store(true, Ordering::SeqCst);
            self.activity(
                ActivityKind::Lifecycle,
                None,
                "every agent is idle: restarting into the upgrade".into(),
            );
            self.emit(Event::App(AppEvent::UpgradeRestarting));
            self.begin_quit();
        }
    }

    /// The app is quitting: stop the build, so cargo is killed and reaped with the rest.
    pub(crate) fn cancel_upgrade_build(&mut self) {
        if let Some(cancel) = &self.app.upgrade.building {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}
