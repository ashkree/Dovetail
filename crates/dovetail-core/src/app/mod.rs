//! App domain: system clipboard, external editor and browser launchers, and self-upgrade.

pub(crate) mod clipboard;
pub mod editor;
pub mod external;
pub mod upgrade;

use std::path::PathBuf;

use dovetail_api::Event;
use dovetail_api::agents::agent::AgentId;
use dovetail_api::app::{AppCommand, AppEvent};

use crate::engine::{Core, Input};

/// State owned by the app domain: external editor handoff and self-upgrade flow.
#[derive(Default)]
pub(crate) struct AppState {
    /// An editor runs in the UI's terminal ([`dovetail_api::app::AppCommand::OpenEditor`]).
    pub(crate) editor_open: bool,
    /// An editor the core accepted and is waiting for the UI to let go of the terminal for.
    pub(crate) editor_pending: Option<PathBuf>,
    /// `:upgrade`: the background build, the "Upgrade ready" item, the wait for idle (ADR 0014).
    pub(crate) upgrade: upgrade::flow::UpgradeState,
}

impl Core {
    pub(crate) async fn handle_app_command(&mut self, command: AppCommand) {
        match command {
            AppCommand::Paste { agent } => self.paste_clipboard(agent),
            AppCommand::PasteText { agent, text } => self.paste_text(agent, text),
            AppCommand::Copy { text } => self.copy_to_clipboard(text),
            AppCommand::OpenEditor { path } => self.open_editor(path),
            AppCommand::TerminalReleased => self.run_editor(),
            AppCommand::StartUpgrade { local } => self.start_upgrade(local),
            AppCommand::OpenExternal { url } => self.open_external(url),
            AppCommand::Resync => self.send_snapshot(false).await,
            AppCommand::TakeOver { agent } => self.take_over(agent).await,
            // Quit is handled by the loop.
            AppCommand::Quit { .. } => {}
        }
    }

    /// Reads the clipboard on a blocking thread (the tools may take seconds) and answers with
    /// [`AppEvent::Pasted`] when done. Images are saved under `attachments/`.
    fn paste_clipboard(&mut self, agent: AgentId) {
        let dir = self.paths.attachments();
        let inputs = self.inputs.clone();
        tokio::task::spawn_blocking(move || {
            let paste = clipboard::paste(&dir);
            let _ = inputs.send(Input::Pasted { agent, paste });
        });
    }

    /// Pasted text: a path to an existing image becomes an attachment (the file is read and
    /// copied on a blocking thread), anything else comes back as the text.
    fn paste_text(&mut self, agent: AgentId, text: String) {
        let dir = self.paths.attachments();
        let inputs = self.inputs.clone();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        tokio::task::spawn_blocking(move || {
            let paste = crate::agents::attachments::paste_text(text, &dir, home.as_deref());
            let _ = inputs.send(Input::Pasted { agent, paste });
        });
    }

    /// Copies text to the clipboard on a blocking thread; only a failure is reported.
    fn copy_to_clipboard(&mut self, text: String) {
        let inputs = self.inputs.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = clipboard::copy(&text) {
                let _ = inputs.send(Input::CopyFailed(e));
            }
        });
    }

    /// Opens a web page in the browser (ADR 0018). The opener is waited for on a blocking
    /// thread, so it is reaped; only a failure is reported.
    fn open_external(&mut self, url: String) {
        let opener = external::resolve(self.options.opener.as_deref());
        let mut child = match external::spawn(&opener, &url) {
            Ok(child) => child,
            Err(e) => {
                self.fail(e);
                return;
            }
        };
        let inputs = self.inputs.clone();
        let shown = opener.to_string_lossy().into_owned();
        tokio::task::spawn_blocking(move || match child.wait() {
            Ok(status) if status.success() => {}
            Ok(status) => {
                let _ = inputs.send(Input::OpenerFailed(format!(
                    "{shown} could not open {url} ({status})"
                )));
            }
            Err(e) => {
                let _ = inputs.send(Input::OpenerFailed(format!("{shown}: {e}")));
            }
        });
    }

    /// Accepts [`AppCommand::OpenEditor`]: checks the path, then asks the UI to let go of the
    /// terminal ([`AppEvent::TerminalLend`]). A missing path or a terminal already lent out is
    /// answered with [`AppEvent::CommandFailed`] and the UI keeps the terminal, so nothing
    /// flickers.
    fn open_editor(&mut self, path: PathBuf) {
        if self.app.editor_open
            || self.app.editor_pending.is_some()
            || self.sessions.takeover.is_some()
        {
            self.fail("the terminal is already lent out");
            return;
        }
        if !path.exists() {
            self.fail(editor::EditorError::Missing(path.display().to_string()));
            return;
        }
        self.app.editor_pending = Some(path);
        self.emit(Event::App(AppEvent::TerminalLend));
    }

    /// Runs the accepted editor in the terminal the UI has let go of, and answers
    /// [`AppEvent::TerminalReturned`] when it exits.
    fn run_editor(&mut self) {
        let Some(path) = self.app.editor_pending.take() else {
            return;
        };
        self.app.editor_open = true;
        let editor = editor::resolve(self.options.editor.as_deref());
        let inputs = self.inputs.clone();
        tokio::task::spawn_blocking(move || {
            let error = editor::run(&editor, &path).err().map(|e| {
                tracing::info!(%e, "editor");
                e.to_string()
            });
            let _ = inputs.send(Input::EditorExited(error));
        });
    }
}
