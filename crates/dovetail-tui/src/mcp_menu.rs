//! The MCP server menu (task #44; `docs/spikes/mcp-headless.md`): `/mcp` in the chat bar and
//! `:mcp`. It is the one [`Picker`], with the session's servers as rows and the selected
//! server's details and tools as the preview. Claude Code gives no change event, so the menu
//! asks for the servers when it opens and again after every action (and on `Tab`).
//!
//! Keys, besides the picker's own (also named in `:keys` and in the menu's own hint
//! line): `Enter` runs the server's first action, `Ctrl-r` reconnects,
//! `Ctrl-t` disables or enables, `Ctrl-l` logs in, `Ctrl-o` logs out, `Tab` refreshes. Only the
//! actions that apply to the selected server run ([`McpServer::actions`]).
//!
//! Logging in: Claude Code returns a URL ([`McpOutcome::AuthUrl`]); the menu has it opened in the
//! browser. A claude.ai connector finishes in the browser by itself. For any other server the
//! browser ends at `redirect_uri`, which nothing listens on, so the menu asks for that address
//! to be pasted back and hands it over ([`McpRequest::OAuthCallback`]).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dovetail_api::{
    agent::AgentId,
    mcp::{McpAction, McpOutcome, McpRequest, McpServer, McpStatus},
};
use ratatui::style::Color;
use ratatui::text::{Line, Span};

use crate::fmt;
use crate::input::{InputOutcome, TextInput};
use crate::picker::{Picker, PickerItem, PickerKey, PickerOutcome};
use crate::theme::Theme;

/// Where a non-claude.ai login ends up in the browser. Nothing listens there: the address the
/// browser shows is what the user pastes back.
pub const REDIRECT_URI: &str = "http://localhost:19284/callback";

/// What the app does after a key or an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Ask the session.
    Ask(McpRequest),
    /// Open this page in the browser.
    Open(String),
    /// Close the menu.
    Close,
}

/// A line under the details: what the last action did.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Note {
    text: String,
    error: bool,
}

/// A log-in waiting for the pasted address.
struct Pasting {
    server: String,
    input: TextInput,
}

/// The open menu.
pub struct McpMenu {
    session: AgentId,
    agent_name: String,
    /// `None` until the first answer.
    servers: Option<Vec<McpServer>>,
    picker: Picker,
    note: Option<Note>,
    pasting: Option<Pasting>,
}

impl McpMenu {
    /// A menu for `session`; the caller sends [`McpRequest::Status`].
    pub fn new(session: AgentId, agent_name: String) -> Self {
        Self {
            session,
            agent_name,
            servers: None,
            picker: Picker::new("MCP servers", Vec::new()).preview_from_top(),
            note: None,
            pasting: None,
        }
    }

    /// The session it shows.
    pub fn session(&self) -> &AgentId {
        &self.session
    }

    /// The picker, for the keys and the box.
    pub fn picker(&self) -> &Picker {
        &self.picker
    }

    /// The server highlighted now.
    fn selected(&self) -> Option<&McpServer> {
        let PickerKey::McpServer(name) = &self.picker.selected()?.key else {
            return None;
        };
        self.servers.as_ref()?.iter().find(|s| &s.name == name)
    }

    /// Applies a key.
    pub fn key(&mut self, key: KeyEvent) -> Vec<Step> {
        if self.pasting.is_some() {
            return self.paste_key(key);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let wanted = match key.code {
            KeyCode::Char('r') if ctrl => Some(Pick::Action(McpAction::Reconnect)),
            KeyCode::Char('t') if ctrl => Some(Pick::Toggle),
            KeyCode::Char('l') if ctrl => Some(Pick::Action(McpAction::LogIn)),
            KeyCode::Char('o') if ctrl => Some(Pick::Action(McpAction::LogOut)),
            KeyCode::Tab => return vec![Step::Ask(McpRequest::Status)],
            _ => None,
        };
        if let Some(pick) = wanted {
            return self.run(pick);
        }
        match self.picker.key(key) {
            PickerOutcome::Pending => Vec::new(),
            PickerOutcome::Closed => vec![Step::Close],
            PickerOutcome::Chosen(_) => self.run(Pick::First),
        }
    }

    fn paste_key(&mut self, key: KeyEvent) -> Vec<Step> {
        let Some(pasting) = &mut self.pasting else {
            return Vec::new();
        };
        match pasting.input.handle(key) {
            InputOutcome::Cancel => {
                self.pasting = None;
                self.say("log in cancelled", false);
                Vec::new()
            }
            InputOutcome::Submit => {
                let url = pasting.input.text().trim().to_owned();
                if url.is_empty() {
                    return Vec::new();
                }
                let server = pasting.server.clone();
                self.pasting = None;
                vec![Step::Ask(McpRequest::OAuthCallback { server, url })]
            }
            InputOutcome::Edited | InputOutcome::Ignored => Vec::new(),
        }
    }

    /// Carries out `pick` on the selected server, if it applies.
    fn run(&mut self, pick: Pick) -> Vec<Step> {
        let Some(server) = self.selected() else {
            return Vec::new();
        };
        let actions = server.actions();
        let action = match pick {
            Pick::First => actions.first().copied(),
            Pick::Toggle => actions
                .iter()
                .copied()
                .find(|a| matches!(a, McpAction::Disable | McpAction::Enable)),
            Pick::Action(a) => actions.contains(&a).then_some(a),
        };
        let name = server.name.clone();
        let Some(action) = action else {
            let what = match pick {
                Pick::Action(a) => a.label(),
                Pick::Toggle => "disable or enable",
                Pick::First => "act on",
            };
            self.say(format!("cannot {what} {name} now"), true);
            return Vec::new();
        };
        self.say(format!("{} {name}…", action.label()), false);
        vec![Step::Ask(McpRequest::for_action(
            action,
            &name,
            REDIRECT_URI,
        ))]
    }

    /// Takes in the session's answer to `request`.
    pub fn reply(&mut self, request: &McpRequest, outcome: McpOutcome, theme: &Theme) -> Vec<Step> {
        match (request, outcome) {
            (McpRequest::Status, McpOutcome::Status(servers)) => {
                self.set_servers(servers, theme);
                Vec::new()
            }
            (McpRequest::Status, McpOutcome::Failed(why)) => {
                self.servers.get_or_insert_with(Vec::new);
                self.say(format!("could not list the servers: {why}"), true);
                Vec::new()
            }
            (
                McpRequest::Authenticate { server, .. },
                McpOutcome::AuthUrl {
                    url,
                    callback_expected,
                },
            ) => {
                let claude_ai = self
                    .servers
                    .iter()
                    .flatten()
                    .find(|s| &s.name == server)
                    .is_some_and(McpServer::is_claude_ai);
                // Claude Code listens for the end of the login itself, or claude.ai finishes it:
                // nothing to paste back.
                if claude_ai || callback_expected {
                    self.say(
                        format!("log in to {server} in the browser, then Tab to refresh"),
                        false,
                    );
                } else {
                    self.say(
                        format!(
                            "log in to {server} in the browser, then paste the address it ends at"
                        ),
                        false,
                    );
                    self.pasting = Some(Pasting {
                        server: server.clone(),
                        input: TextInput::line(""),
                    });
                }
                vec![Step::Open(url), Step::Ask(McpRequest::Status)]
            }
            (request, McpOutcome::Failed(why)) => {
                let what = request.server().map_or(String::new(), |s| format!(" {s}"));
                self.say(format!("{}{what}: {why}", verb(request)), true);
                // The answer may have changed something (a failed reconnect updates the error).
                vec![Step::Ask(McpRequest::Status)]
            }
            (request, _) => {
                let what = request.server().map_or(String::new(), |s| format!(" {s}"));
                self.say(format!("{}{what}: done", verb(request)), false);
                vec![Step::Ask(McpRequest::Status)]
            }
        }
    }

    fn set_servers(&mut self, servers: Vec<McpServer>, theme: &Theme) {
        self.picker.set_items(server_items(&servers, theme));
        self.servers = Some(servers);
    }

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.note = Some(Note {
            text: text.into(),
            error,
        });
    }

    /// The preview pane's lines for the selected server.
    pub fn preview(&self, width: usize, theme: &Theme) -> Vec<Line<'static>> {
        let mut out = Vec::new();
        if let Some(p) = &self.pasting {
            out.push(Line::from(Span::styled(
                format!("Log in to {}", p.server),
                theme.accent(),
            )));
            for line in fmt::wrap(
                "Finish in the browser, then paste the address it ends at (it will not load) and press Enter. Esc cancels.",
                width,
            ) {
                out.push(Line::from(Span::styled(line, theme.body())));
            }
            out.push(Line::default());
            out.push(Line::from(vec![
                Span::styled("› ", theme.accent()),
                Span::styled(p.input.text().to_owned(), theme.body()),
                Span::styled("▏", theme.accent()),
            ]));
            return out;
        }
        match (&self.servers, self.selected()) {
            (None, _) => out.push(Line::from(Span::styled(
                format!("asking {} for its servers…", self.agent_name),
                theme.dim(),
            ))),
            (Some(servers), None) if servers.is_empty() => out.push(Line::from(Span::styled(
                format!("{} has no MCP servers", self.agent_name),
                theme.dim(),
            ))),
            (Some(_), None) => {}
            (Some(_), Some(server)) => out.extend(server_lines(server, width, theme)),
        }
        if let Some(note) = &self.note {
            out.push(Line::default());
            let style = if note.error {
                theme.fg(theme.error)
            } else {
                theme.dim()
            };
            for line in fmt::wrap(&note.text, width) {
                out.push(Line::from(Span::styled(line, style)));
            }
        }
        out
    }
}

/// What a key asks for.
#[derive(Debug, Clone, Copy)]
enum Pick {
    /// The server's first action (`Enter`).
    First,
    /// Disable or enable, whichever applies.
    Toggle,
    /// This action.
    Action(McpAction),
}

/// The past-tense-free name of a request, for the note.
fn verb(request: &McpRequest) -> &'static str {
    match request {
        McpRequest::Status => "list",
        McpRequest::Reconnect { .. } => "reconnect",
        McpRequest::Toggle { enabled: true, .. } => "enable",
        McpRequest::Toggle { enabled: false, .. } => "disable",
        McpRequest::Authenticate { .. } => "log in",
        McpRequest::OAuthCallback { .. } => "log in",
        McpRequest::ClearAuth { .. } => "log out",
    }
}

fn status_look(status: &McpStatus, theme: &Theme) -> (&'static str, Color) {
    match status {
        McpStatus::Connected => ("✓", theme.ok),
        McpStatus::Failed => ("✗", theme.error),
        McpStatus::NeedsAuth => ("⚠", theme.needs_you),
        McpStatus::Disabled => ("○", theme.dim),
        McpStatus::Pending => ("◐", theme.working),
        McpStatus::Other(_) => ("?", theme.dim),
    }
}

/// The rows: every server in the order Claude Code lists them.
pub fn server_items(servers: &[McpServer], theme: &Theme) -> Vec<PickerItem> {
    servers
        .iter()
        .map(|s| {
            let (glyph, color) = status_look(&s.status, theme);
            PickerItem {
                key: PickerKey::McpServer(s.name.clone()),
                label: s.name.clone(),
                haystack: format!("{} {} {}", s.name, s.status.label(), s.transport),
                detail: s.status.label().to_owned(),
                glyph,
                color,
                depth: 0,
                section: "",
            }
        })
        .collect()
}

fn server_lines(server: &McpServer, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let (glyph, color) = status_look(&server.status, theme);
    let mut out = vec![Line::from(vec![
        Span::styled(glyph, theme.fg(color)),
        Span::raw(" "),
        Span::styled(server.name.clone(), theme.accent()),
        Span::styled(format!("  {}", server.status.label()), theme.fg(color)),
    ])];
    let facts: Vec<&str> = [server.transport.as_str(), server.scope.as_str()]
        .into_iter()
        .filter(|f| !f.is_empty())
        .collect();
    if !facts.is_empty() {
        out.push(Line::from(Span::styled(facts.join(" · "), theme.dim())));
    }
    if !server.target.is_empty() {
        for line in fmt::wrap(&server.target, width) {
            out.push(Line::from(Span::styled(line, theme.body())));
        }
    }
    if let Some(info) = &server.server_info {
        out.push(Line::from(Span::styled(
            format!("server: {info}"),
            theme.dim(),
        )));
    }
    if let Some(error) = &server.error {
        for line in fmt::wrap(error, width) {
            out.push(Line::from(Span::styled(line, theme.fg(theme.error))));
        }
    }
    out.push(Line::default());
    let actions = server.actions();
    let mut keys: Vec<String> = actions
        .first()
        .map(|a| format!("⏎ {}", a.label()))
        .into_iter()
        .collect();
    keys.extend(
        actions
            .into_iter()
            .map(|a| format!("{} {}", action_key(a), a.label())),
    );
    keys.push("tab refresh".into());
    for line in fmt::wrap(&keys.join(" · "), width) {
        out.push(Line::from(Span::styled(line, theme.dim())));
    }
    if !server.tools.is_empty() {
        out.push(Line::default());
        out.push(Line::from(Span::styled(
            format!("Tools ({})", server.tools.len()),
            theme.dim(),
        )));
        for tool in &server.tools {
            let mut spans = vec![Span::styled(format!("  {}", tool.name), theme.body())];
            if tool.read_only {
                spans.push(Span::styled("  read-only", theme.fg(theme.ok)));
            }
            if tool.destructive {
                spans.push(Span::styled("  destructive", theme.fg(theme.error)));
            }
            out.push(Line::from(spans));
        }
    }
    out
}

/// The key that runs `action`, as the hint line shows it.
fn action_key(action: McpAction) -> &'static str {
    match action {
        McpAction::Reconnect => "^r",
        McpAction::Disable | McpAction::Enable => "^t",
        McpAction::LogIn => "^l",
        McpAction::LogOut => "^o",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dovetail_api::mcp::McpTool;

    fn server(name: &str, transport: &str, status: McpStatus) -> McpServer {
        McpServer {
            name: name.into(),
            status,
            error: None,
            server_info: None,
            transport: transport.into(),
            target: String::new(),
            scope: String::new(),
            tools: Vec::new(),
        }
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn menu(servers: Vec<McpServer>) -> McpMenu {
        let mut m = McpMenu::new(AgentId("coordinator".into()), "coordinator".into());
        m.reply(
            &McpRequest::Status,
            McpOutcome::Status(servers),
            &Theme::default(),
        );
        m
    }

    fn ask(server: &str, enabled: bool) -> Step {
        Step::Ask(McpRequest::Toggle {
            server: server.into(),
            enabled,
        })
    }

    #[test]
    fn rows_show_each_server_with_its_status() {
        let items = server_items(
            &[
                server("good", "stdio", McpStatus::Connected),
                server("cal", "claudeai-proxy", McpStatus::NeedsAuth),
                server("off", "stdio", McpStatus::Disabled),
            ],
            &Theme::default(),
        );
        let details: Vec<&str> = items.iter().map(|i| i.detail.as_str()).collect();
        assert_eq!(details, ["connected", "needs auth", "disabled"]);
        assert_eq!(items[1].key, PickerKey::McpServer("cal".into()));
        assert!(items[1].haystack.contains("needs auth"));
    }

    #[test]
    fn keys_send_only_the_actions_that_apply() {
        let mut m = menu(vec![server("good", "stdio", McpStatus::Connected)]);
        assert_eq!(
            m.key(ctrl('r')),
            vec![Step::Ask(McpRequest::Reconnect {
                server: "good".into()
            })]
        );
        assert_eq!(m.key(ctrl('t')), vec![ask("good", false)]);
        // A stdio server has no login to start or clear.
        assert_eq!(m.key(ctrl('l')), Vec::new());
        assert_eq!(m.key(ctrl('o')), Vec::new());
        assert!(
            m.preview(40, &Theme::default())
                .iter()
                .any(|l| l.to_string().contains("cannot log out good now"))
        );
    }

    #[test]
    fn a_disabled_server_offers_only_enable() {
        let mut m = menu(vec![server("off", "stdio", McpStatus::Disabled)]);
        assert_eq!(m.key(ctrl('r')), Vec::new());
        assert_eq!(m.key(ctrl('t')), vec![ask("off", true)]);
        assert_eq!(m.key(key(KeyCode::Enter)), vec![ask("off", true)]);
    }

    #[test]
    fn enter_runs_the_first_action_and_a_login_is_for_a_server_that_needs_one() {
        let mut m = menu(vec![server("cal", "claudeai-proxy", McpStatus::NeedsAuth)]);
        let login = Step::Ask(McpRequest::Authenticate {
            server: "cal".into(),
            redirect_uri: REDIRECT_URI.into(),
        });
        assert_eq!(m.key(key(KeyCode::Enter)), vec![login.clone()]);
        assert_eq!(m.key(ctrl('l')), vec![login]);
    }

    #[test]
    fn an_error_reply_shows_in_the_menu_and_the_servers_are_asked_again() {
        let mut m = menu(vec![server("broken", "stdio", McpStatus::Failed)]);
        let steps = m.reply(
            &McpRequest::Reconnect {
                server: "broken".into(),
            },
            McpOutcome::Failed("ENOENT: no such file".into()),
            &Theme::default(),
        );
        assert_eq!(steps, vec![Step::Ask(McpRequest::Status)]);
        let text: Vec<String> = m
            .preview(60, &Theme::default())
            .iter()
            .map(|l| l.to_string())
            .collect();
        assert!(text.iter().any(|l| l.contains("reconnect broken: ENOENT")));
    }

    #[test]
    fn a_login_claude_code_listens_for_asks_for_no_paste() {
        let mut m = menu(vec![server("api", "http", McpStatus::NeedsAuth)]);
        let steps = m.reply(
            &McpRequest::Authenticate {
                server: "api".into(),
                redirect_uri: REDIRECT_URI.into(),
            },
            McpOutcome::AuthUrl {
                url: "https://auth.example/x".into(),
                callback_expected: true,
            },
            &Theme::default(),
        );
        assert_eq!(steps[0], Step::Open("https://auth.example/x".into()));
        assert!(m.pasting.is_none());
        assert!(
            m.preview(80, &Theme::default())
                .iter()
                .any(|l| l.to_string().contains("then Tab to refresh"))
        );
    }

    #[test]
    fn the_hint_line_names_what_enter_runs() {
        let m = menu(vec![server("api", "http", McpStatus::NeedsAuth)]);
        let text: Vec<String> = m
            .preview(80, &Theme::default())
            .iter()
            .map(|l| l.to_string())
            .collect();
        assert!(
            text.iter().any(|l| l.starts_with("⏎ log in · ^l log in")),
            "{text:?}"
        );
    }

    #[test]
    fn a_claude_ai_login_opens_the_browser_and_waits_for_a_refresh() {
        let mut m = menu(vec![server("cal", "claudeai-proxy", McpStatus::NeedsAuth)]);
        let steps = m.reply(
            &McpRequest::Authenticate {
                server: "cal".into(),
                redirect_uri: REDIRECT_URI.into(),
            },
            McpOutcome::AuthUrl {
                url: "https://claude.ai/start".into(),
                callback_expected: false,
            },
            &Theme::default(),
        );
        assert_eq!(
            steps,
            vec![
                Step::Open("https://claude.ai/start".into()),
                Step::Ask(McpRequest::Status)
            ]
        );
        assert!(m.pasting.is_none(), "claude.ai finishes by itself");
        assert_eq!(
            m.key(key(KeyCode::Tab)),
            vec![Step::Ask(McpRequest::Status)]
        );
    }

    #[test]
    fn another_login_takes_the_pasted_address_and_hands_it_over() {
        let mut m = menu(vec![server("api", "http", McpStatus::NeedsAuth)]);
        m.reply(
            &McpRequest::Authenticate {
                server: "api".into(),
                redirect_uri: REDIRECT_URI.into(),
            },
            McpOutcome::AuthUrl {
                url: "https://auth.example/x".into(),
                callback_expected: false,
            },
            &Theme::default(),
        );
        for c in "http://localhost:19284/callback?code=1".chars() {
            assert_eq!(m.key(key(KeyCode::Char(c))), Vec::new());
        }
        assert_eq!(
            m.key(key(KeyCode::Enter)),
            vec![Step::Ask(McpRequest::OAuthCallback {
                server: "api".into(),
                url: "http://localhost:19284/callback?code=1".into()
            })]
        );
        assert!(m.pasting.is_none());
    }

    #[test]
    fn escape_cancels_a_login_before_it_closes_the_menu() {
        let mut m = menu(vec![server("api", "http", McpStatus::NeedsAuth)]);
        m.pasting = Some(Pasting {
            server: "api".into(),
            input: TextInput::line(""),
        });
        assert_eq!(m.key(key(KeyCode::Esc)), Vec::new());
        assert!(m.pasting.is_none());
        assert_eq!(m.key(key(KeyCode::Esc)), vec![Step::Close]);
    }

    #[test]
    fn the_preview_lists_tools_with_their_markers() {
        let mut s = server("docs", "claudeai-proxy", McpStatus::Connected);
        s.tools = vec![
            McpTool {
                name: "read".into(),
                read_only: true,
                destructive: false,
            },
            McpTool {
                name: "delete".into(),
                read_only: false,
                destructive: true,
            },
        ];
        let m = menu(vec![s]);
        let text: Vec<String> = m
            .preview(60, &Theme::default())
            .iter()
            .map(|l| l.to_string())
            .collect();
        assert!(text.contains(&"  read  read-only".to_string()), "{text:?}");
        assert!(text.contains(&"  delete  destructive".to_string()));
        assert!(text.iter().any(|l| l.starts_with("Tools (2)")));
    }
}
