//! The `mcp` feature: the MCP menu, a session's servers and the actions on them.

mod command;
mod event;
pub use command::McpCommand;
pub use event::McpEvent;

use serde::{Deserialize, Serialize};

/// Where one server stands, as `mcp_status` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpStatus {
    /// Running and answering.
    Connected,
    /// Could not connect; the server's `error` says why.
    Failed,
    /// Waiting for the user to log in.
    NeedsAuth,
    /// Switched off.
    Disabled,
    /// Still connecting.
    Pending,
    /// A status this version does not know, as Claude Code wrote it.
    Other(String),
}

impl McpStatus {
    /// Reads Claude Code's word for it (`needs-auth`).
    pub fn parse(word: &str) -> Self {
        match word {
            "connected" => Self::Connected,
            "failed" => Self::Failed,
            "needs-auth" => Self::NeedsAuth,
            "disabled" => Self::Disabled,
            "pending" => Self::Pending,
            other => Self::Other(other.to_owned()),
        }
    }

    /// The words the menu shows.
    pub fn label(&self) -> &str {
        match self {
            Self::Connected => "connected",
            Self::Failed => "failed",
            Self::NeedsAuth => "needs auth",
            Self::Disabled => "disabled",
            Self::Pending => "pending",
            Self::Other(word) => word,
        }
    }
}

/// One tool a server offers. Claude Code gives names and two hints, no descriptions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpTool {
    /// The tool's name, without the `mcp__server__` prefix.
    pub name: String,
    /// The server says it changes nothing.
    #[serde(default)]
    pub read_only: bool,
    /// The server says it can destroy things.
    #[serde(default)]
    pub destructive: bool,
}

/// One server of a session, from `mcp_status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServer {
    /// Its name in the config (`claude.ai Google Calendar`).
    pub name: String,
    /// Where it stands.
    pub status: McpStatus,
    /// Why it failed, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What the server calls itself once connected: `name version`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_info: Option<String>,
    /// `stdio`, `http`, `sse`, `claudeai-proxy`; empty when not told.
    #[serde(default)]
    pub transport: String,
    /// How it is reached: the command line of a stdio server, the URL of another. Environment
    /// values are never part of it (Claude Code leaves them out).
    #[serde(default)]
    pub target: String,
    /// Where its config comes from: `dynamic` (a launch flag), `claudeai`, `user`, `project`.
    #[serde(default)]
    pub scope: String,
    /// Its tools; none while it is not connected.
    #[serde(default)]
    pub tools: Vec<McpTool>,
}

/// What the menu can do to a server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAction {
    /// Connect again.
    Reconnect,
    /// Switch it off for this session.
    Disable,
    /// Switch it on again.
    Enable,
    /// Start the OAuth login.
    LogIn,
    /// Forget the stored login.
    LogOut,
}

impl McpAction {
    /// The words for the menu.
    pub fn label(self) -> &'static str {
        match self {
            Self::Reconnect => "reconnect",
            Self::Disable => "disable",
            Self::Enable => "enable",
            Self::LogIn => "log in",
            Self::LogOut => "log out",
        }
    }
}

impl McpServer {
    /// Whether the server can have a login at all: a stdio server cannot.
    pub fn has_auth(&self) -> bool {
        self.transport != "stdio" && !self.transport.is_empty()
    }

    /// Whether the connection goes through claude.ai, which finishes a login in the browser by
    /// itself (no URL to paste back).
    pub fn is_claude_ai(&self) -> bool {
        self.transport == "claudeai-proxy"
    }

    /// Whether it failed for want of a login: a failure whose text speaks of authorisation (the
    /// spike's `Dynamic Client Registration rejected (HTTP 401)`). Only a server that can have a
    /// login counts; a stdio server's `401` is something else.
    pub fn failed_for_auth(&self) -> bool {
        if self.status != McpStatus::Failed || !self.has_auth() {
            return false;
        }
        let error = self.error.as_deref().unwrap_or("").to_ascii_lowercase();
        ["401", "unauthorized", "oauth", "auth"]
            .iter()
            .any(|word| error.contains(word))
    }

    /// The actions that apply to it now, the one `Enter` runs first.
    pub fn actions(&self) -> Vec<McpAction> {
        use McpAction::*;
        let auth = self.has_auth();
        match self.status {
            McpStatus::Disabled => vec![Enable],
            McpStatus::NeedsAuth => vec![LogIn, Reconnect, Disable],
            McpStatus::Failed if self.failed_for_auth() => vec![LogIn, Reconnect, Disable],
            McpStatus::Failed if auth => vec![Reconnect, LogIn, Disable],
            McpStatus::Failed => vec![Reconnect, Disable],
            McpStatus::Connected if auth => vec![Reconnect, Disable, LogOut],
            McpStatus::Connected => vec![Reconnect, Disable],
            McpStatus::Pending | McpStatus::Other(_) => vec![Disable],
        }
    }
}

/// One control request of the menu, which Claude Code answers with an [`McpOutcome`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpRequest {
    /// `mcp_status`: every server with its tools.
    Status,
    /// `mcp_reconnect`.
    Reconnect {
        /// The server.
        server: String,
    },
    /// `mcp_toggle`.
    Toggle {
        /// The server.
        server: String,
        /// `true` switches it on.
        enabled: bool,
    },
    /// `mcp_authenticate` with `redirectUriOnly`: asks for the URL to log in at.
    Authenticate {
        /// The server.
        server: String,
        /// Where the login ends up in the browser.
        redirect_uri: String,
    },
    /// `mcp_oauth_callback_url`: hands over the URL the browser ended up at.
    OAuthCallback {
        /// The server.
        server: String,
        /// The URL pasted back.
        url: String,
    },
    /// `mcp_clear_auth`.
    ClearAuth {
        /// The server.
        server: String,
    },
}

impl McpRequest {
    /// The server it is about; `None` for [`McpRequest::Status`].
    pub fn server(&self) -> Option<&str> {
        match self {
            Self::Status => None,
            Self::Reconnect { server }
            | Self::Toggle { server, .. }
            | Self::Authenticate { server, .. }
            | Self::OAuthCallback { server, .. }
            | Self::ClearAuth { server } => Some(server),
        }
    }

    /// The request that carries out `action` on `server`. A log-in asks for the URL first, with
    /// `redirect_uri` as the place the browser ends up.
    pub fn for_action(action: McpAction, server: &str, redirect_uri: &str) -> Self {
        let server = server.to_owned();
        match action {
            McpAction::Reconnect => Self::Reconnect { server },
            McpAction::Disable => Self::Toggle {
                server,
                enabled: false,
            },
            McpAction::Enable => Self::Toggle {
                server,
                enabled: true,
            },
            McpAction::LogIn => Self::Authenticate {
                server,
                redirect_uri: redirect_uri.to_owned(),
            },
            McpAction::LogOut => Self::ClearAuth { server },
        }
    }
}

/// What Claude Code answered to an [`McpRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpOutcome {
    /// The servers, in answer to [`McpRequest::Status`].
    Status(Vec<McpServer>),
    /// It worked, with nothing to report.
    Done,
    /// The login starts at `url`; open it.
    AuthUrl {
        /// Where to log in.
        url: String,
        /// Claude Code listens for the end of the login itself, so nothing is pasted back.
        callback_expected: bool,
    },
    /// Refused; Claude Code's reason.
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(transport: &str, status: McpStatus) -> McpServer {
        McpServer {
            name: "s".into(),
            status,
            error: None,
            server_info: None,
            transport: transport.into(),
            target: String::new(),
            scope: String::new(),
            tools: Vec::new(),
        }
    }

    #[test]
    fn statuses_read_claude_codes_words() {
        assert_eq!(McpStatus::parse("needs-auth"), McpStatus::NeedsAuth);
        assert_eq!(McpStatus::parse("connected"), McpStatus::Connected);
        assert_eq!(McpStatus::parse("odd"), McpStatus::Other("odd".into()));
        assert_eq!(McpStatus::parse("odd").label(), "odd");
    }

    #[test]
    fn a_stdio_server_never_offers_a_login() {
        for status in [
            McpStatus::Connected,
            McpStatus::Failed,
            McpStatus::NeedsAuth,
        ] {
            let actions = server("stdio", status.clone()).actions();
            if status != McpStatus::NeedsAuth {
                assert!(!actions.contains(&McpAction::LogIn), "{status:?}");
            }
            assert!(!actions.contains(&McpAction::LogOut), "{status:?}");
        }
    }

    #[test]
    fn actions_follow_the_status() {
        use McpAction::*;
        let acts = |t, s| server(t, s).actions();
        assert_eq!(acts("stdio", McpStatus::Connected), [Reconnect, Disable]);
        assert_eq!(acts("stdio", McpStatus::Failed), [Reconnect, Disable]);
        assert_eq!(acts("stdio", McpStatus::Disabled), [Enable]);
        assert_eq!(
            acts("http", McpStatus::NeedsAuth),
            [LogIn, Reconnect, Disable]
        );
        assert_eq!(acts("http", McpStatus::Failed), [Reconnect, LogIn, Disable]);
        let mut auth_failed = server("http", McpStatus::Failed);
        auth_failed.error = Some("Dynamic Client Registration rejected (HTTP 401): ".into());
        assert_eq!(auth_failed.actions(), [LogIn, Reconnect, Disable]);
        // The same words from a stdio server are not a login to start.
        let mut stdio = server("stdio", McpStatus::Failed);
        stdio.error = Some("auth helper not found".into());
        assert_eq!(stdio.actions(), [Reconnect, Disable]);
        assert_eq!(
            acts("http", McpStatus::Connected),
            [Reconnect, Disable, LogOut]
        );
        assert_eq!(acts("claudeai-proxy", McpStatus::Disabled), [Enable]);
        assert_eq!(acts("http", McpStatus::Pending), [Disable]);
    }

    #[test]
    fn an_action_maps_to_its_request() {
        let uri = "http://localhost:1/cb";
        assert_eq!(
            McpRequest::for_action(McpAction::Disable, "a", uri),
            McpRequest::Toggle {
                server: "a".into(),
                enabled: false
            }
        );
        assert_eq!(
            McpRequest::for_action(McpAction::Enable, "a", uri),
            McpRequest::Toggle {
                server: "a".into(),
                enabled: true
            }
        );
        assert_eq!(
            McpRequest::for_action(McpAction::LogIn, "a", uri),
            McpRequest::Authenticate {
                server: "a".into(),
                redirect_uri: uri.into()
            }
        );
        assert_eq!(McpRequest::Status.server(), None);
        assert_eq!(
            McpRequest::for_action(McpAction::LogOut, "a", uri).server(),
            Some("a")
        );
    }
}
