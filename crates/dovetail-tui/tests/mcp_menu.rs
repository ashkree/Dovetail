//! Task #44: the MCP server menu. `/mcp` in the chat bar and `:mcp` open it on the session in
//! view; it asks for the servers, shows them with their tools and runs actions on the selected
//! one. The mock core answers like Claude Code does (`docs/spikes/mcp-headless.md`).

mod support;

use dovetail_api::{Command, McpRequest};
use dovetail_mock::{MockConfig, MockCore};
use dovetail_tui::{App, Mode};
use support::*;

/// An app connected to a mock core, the first messages delivered.
fn connected() -> (App, MockCore) {
    let core = MockCore::new(MockConfig::default());
    let mut app = App::new(jiff::tz::TimeZone::UTC);
    for msg in core.connect() {
        app.on_server(msg);
    }
    let mut pair = (app, core);
    pump(&mut pair);
    pair
}

/// Carries commands to the core and its answers back, until nothing is left.
fn pump(pair: &mut (App, MockCore)) -> Vec<Command> {
    let (app, core) = pair;
    let mut sent = Vec::new();
    loop {
        let commands = app.take_commands();
        if commands.is_empty() {
            break;
        }
        for command in commands {
            sent.push(command.clone());
            core.handle(command);
        }
        while core.peek_delay() == 0 {
            app.on_server(core.pop());
        }
    }
    sent
}

fn mcp_requests(sent: &[Command]) -> Vec<McpRequest> {
    sent.iter()
        .filter_map(|c| match c {
            Command::Mcp { request, .. } => Some(request.clone()),
            _ => None,
        })
        .collect()
}

/// `/mcp` typed into the chat bar: the `/` opens the skill picker, Esc closes it, the rest goes
/// into the bar.
fn slash_mcp(app: &mut App) {
    type_keys(app, "/");
    app.on_key(esc(), 0);
    type_keys(app, "mcp");
}

/// `:name` then Enter, from Normal mode.
fn colon(app: &mut App, line: &str) {
    type_keys(app, line);
    app.on_key(enter(), 50);
}

#[test]
fn slash_mcp_opens_the_menu_and_asks_for_the_servers() {
    let mut pair = connected();
    press(&mut pair.0, "i");
    slash_mcp(&mut pair.0);
    pair.0.on_key(enter(), 100);
    let sent = pump(&mut pair);
    assert_eq!(mcp_requests(&sent), vec![McpRequest::Status]);
    let screen = render(&mut pair.0, 130, 32);
    assert!(screen.contains("MCP servers"), "{screen}");
    // The first server's details and tools are in the preview.
    assert!(screen.contains("dovetail"), "{screen}");
    assert!(screen.contains("report_status"), "{screen}");
    // Nothing went to Claude Code as a prompt.
    assert!(!sent.iter().any(|c| matches!(c, Command::Prompt { .. })));
}

#[test]
fn esc_closes_the_menu_and_the_sent_command_leaves_no_bar_open() {
    let mut pair = connected();
    press(&mut pair.0, "i");
    slash_mcp(&mut pair.0);
    pair.0.on_key(enter(), 100);
    pump(&mut pair);
    pair.0.on_key(esc(), 101);
    // Enter sent the line, so the bar is closed: back to Normal, as after any sent message.
    assert_eq!(pair.0.mode(), Mode::Normal);
    assert!(!render(&mut pair.0, 130, 32).contains("MCP servers"));
}

#[test]
fn colon_mcp_opens_it_from_normal_mode_and_esc_goes_back_to_normal() {
    let mut pair = connected();
    colon(&mut pair.0, ":mcp");
    let sent = pump(&mut pair);
    assert_eq!(mcp_requests(&sent), vec![McpRequest::Status]);
    assert!(render(&mut pair.0, 130, 32).contains("MCP servers"));
    pair.0.on_key(esc(), 200);
    assert_eq!(pair.0.mode(), Mode::Normal);
}

#[test]
fn typing_filters_the_servers() {
    let mut pair = connected();
    colon(&mut pair.0, ":mcp");
    pump(&mut pair);
    type_keys(&mut pair.0, "calen");
    let screen = render(&mut pair.0, 130, 32);
    assert!(screen.contains("claude.ai Google Calendar"), "{screen}");
    assert!(!screen.contains("scratch"), "{screen}");
}

#[test]
fn an_action_asks_the_session_and_the_list_is_asked_for_again() {
    let mut pair = connected();
    colon(&mut pair.0, ":mcp");
    pump(&mut pair);
    // Select the disabled stdio server and enable it.
    type_keys(&mut pair.0, "scratch");
    pair.0.on_key(ctrl('t'), 300);
    let sent = pump(&mut pair);
    assert_eq!(
        mcp_requests(&sent),
        vec![
            McpRequest::Toggle {
                server: "scratch".into(),
                enabled: true
            },
            McpRequest::Status
        ]
    );
    let screen = render(&mut pair.0, 130, 32);
    assert!(screen.contains("connected"), "{screen}");
}

#[test]
fn a_refused_action_shows_its_error_in_the_menu() {
    let mut pair = connected();
    colon(&mut pair.0, ":mcp");
    pump(&mut pair);
    type_keys(&mut pair.0, "broken");
    pair.0.on_key(ctrl('r'), 300);
    pump(&mut pair);
    let screen = render(&mut pair.0, 130, 32);
    assert!(screen.contains("reconnect broken: ENOENT"), "{screen}");
}

#[test]
fn a_login_opens_the_page_the_session_returned() {
    let mut pair = connected();
    colon(&mut pair.0, ":mcp");
    pump(&mut pair);
    type_keys(&mut pair.0, "calendar");
    pair.0.on_key(ctrl('l'), 300);
    let sent = pump(&mut pair);
    assert!(
        sent.contains(&Command::OpenExternal {
            url: "https://login.example/start".into()
        }),
        "{sent:?}"
    );
    // claude.ai finishes in the browser: nothing to paste, a refresh follows.
    assert!(!render(&mut pair.0, 130, 32).contains("paste the address"));
}

#[test]
fn slash_mcp_with_text_after_it_is_refused_and_nothing_is_sent() {
    let mut pair = connected();
    press(&mut pair.0, "i");
    type_keys(&mut pair.0, "/");
    pair.0.on_key(esc(), 0);
    type_keys(&mut pair.0, "mcp now");
    pair.0.on_key(enter(), 100);
    let sent = pump(&mut pair);
    assert!(sent.is_empty(), "{sent:?}");
    assert!(!render(&mut pair.0, 130, 32).contains("MCP servers"));
    assert_eq!(
        pair.0.note(),
        Some("/mcp takes nothing after it; nothing was sent")
    );
}

#[test]
fn a_pasted_login_address_goes_to_the_session_as_the_callback_request() {
    let mut pair = connected();
    colon(&mut pair.0, ":mcp");
    pump(&mut pair);
    type_keys(&mut pair.0, "wiki");
    pair.0.on_key(ctrl('l'), 300);
    let sent = pump(&mut pair);
    assert!(
        sent.contains(&Command::OpenExternal {
            url: "https://login.example/start".into()
        }),
        "{sent:?}"
    );
    assert!(render(&mut pair.0, 130, 32).contains("paste the address"));
    // A paste arrives as typed characters.
    pair.0
        .on_paste("http://localhost:19284/callback?code=abc", 400);
    pair.0.on_key(enter(), 500);
    let sent = pump(&mut pair);
    assert_eq!(
        mcp_requests(&sent),
        vec![
            McpRequest::OAuthCallback {
                server: "wiki".into(),
                url: "http://localhost:19284/callback?code=abc".into()
            },
            McpRequest::Status
        ]
    );
    let screen = render(&mut pair.0, 130, 32);
    assert!(screen.contains("log in wiki: done"), "{screen}");
}
