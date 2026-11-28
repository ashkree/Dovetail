//! Integration tests for the `visual` domain: maps, canvases (blocks, cards, links), slice
//! previews, and declared design variants.

mod common;

use common::*;

use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use dovetail_api::{
    Command, Event, ServerMsg, Snapshot, agent::AgentId, apply, conversation::EntryKind,
    task::NewTask,
};
use dovetail_api::{
    agent::AgentCommand,
    app::AppEvent,
    canvas::CanvasCommand,
    conversation::ConversationCommand,
    design::DesignCommand,
    map::{Map, MapCommand, MapEvent, MapId, MapKind},
    task::TaskCommand,
};
use dovetail_core::config::Config;
use dovetail_tools::project::archive_result::ARCHIVED;
use dovetail_tools::{Tool, map::DrawMap};
use dovetail_tools::{canvas::CanvasTool, design::DesignTool, map::MapTool};
use tempfile::TempDir;

#[test]
fn draw_map_stores_a_map_on_the_shelf_and_loads_it() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let (text, is_error) = draw(&h, checkout_map("Checkout"));
    assert!(!is_error, "{text}");
    assert!(
        text.starts_with(r#"{"id":"checkout","version":1}"#),
        "{text}"
    );
    h.wait_for("the shelf entry", |s| s.maps.len() == 1);
    let shelf = &h.snap.maps[0];
    assert_eq!(shelf.kind, MapKind::Structure);
    assert_eq!(shelf.drawn_by, Some(AgentId(COORDINATOR.into())));
    assert_eq!(shelf.task, None, "the coordinator has no task");
    let map = load_map(&mut h, "checkout", None).expect("loads");
    assert_eq!((map.version, map.content.nodes.len()), (1, 2));
    assert!(load_map(&mut h, "nope", None).is_err());
    // A second map with the same title gets its own id.
    let (text, _) = draw(&h, checkout_map("Checkout"));
    assert!(
        text.starts_with(r#"{"id":"checkout-2","version":1}"#),
        "{text}"
    );
    h.wait_for("both maps", |s| s.maps.len() == 2);

    // The shelf survives a restart.
    let paths = h.paths.clone();
    let _state = h.quit();
    let h = Harness::start_on(fixture("plain_turn"), paths);
    assert_eq!(h.snap.maps.len(), 2);
    h.quit();
}

#[test]
fn draw_map_refuses_an_invalid_map_with_every_problem() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let mut map = checkout_map("Checkout");
    map["nodes"][1]["kind"] = serde_json::json!("table");
    map["connections"][0]["to"] = serde_json::json!("ordrs");
    let (text, is_error) = draw(&h, map);
    assert!(is_error);
    assert!(
        text.starts_with("The map was not saved: 2 problems"),
        "{text}"
    );
    assert!(
        text.contains(r#"kind "table" is not a node kind"#),
        "{text}"
    );
    assert!(text.contains(r#"no node has id "ordrs""#), "{text}");
    // A misspelt field is a schema error, not silently dropped.
    let mut map = checkout_map("Checkout");
    map["nodez"] = serde_json::json!([]);
    let (text, is_error) = draw(&h, map);
    assert!(is_error && text.contains("nodez"), "{text}");
    h.drain(Duration::from_millis(100));
    assert!(h.snap.maps.is_empty());
    h.quit();
}

#[test]
fn draw_map_with_an_existing_id_adds_a_version() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    draw(&h, checkout_map("Checkout"));
    let mut redraw = checkout_map("Checkout, with Stripe");
    redraw["map"]["id"] = serde_json::json!("checkout");
    let (text, is_error) = draw(&h, redraw);
    assert!(!is_error, "{text}");
    assert!(
        text.starts_with(r#"{"id":"checkout","version":2}"#),
        "{text}"
    );
    h.wait_for("version 2", |s| {
        s.maps.first().is_some_and(|m| m.version == 2)
    });
    assert_eq!(h.snap.maps.len(), 1);
    assert_eq!(h.snap.maps[0].title, "Checkout, with Stripe");
    // The earlier version is kept.
    let first = load_map(&mut h, "checkout", Some(1)).expect("version 1");
    assert_eq!(first.content.title, "Checkout");
    assert_eq!(
        load_map(&mut h, "checkout", None).expect("latest").version,
        2
    );
    assert!(load_map(&mut h, "checkout", Some(3)).is_err());
    // A map keeps its type.
    let mut lanes = login_lanes("api");
    lanes["map"]["id"] = serde_json::json!("checkout");
    let (text, is_error) = draw(&h, lanes);
    assert!(is_error && text.contains("a map keeps its type"), "{text}");
    h.quit();
}

#[test]
fn draw_map_checks_lane_links_against_structure_maps() {
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let (text, is_error) = draw(&h, login_lanes("api"));
    assert!(is_error, "{text}");
    assert!(
        text.contains(r#"lanes[1] ("web"): "node" is "api", but no structure map"#),
        "{text}"
    );
    draw(&h, checkout_map("Checkout"));
    let (text, is_error) = draw(&h, login_lanes("api"));
    assert!(!is_error, "{text}");
    h.wait_for("both maps", |s| s.maps.len() == 2);
    // Most recently drawn first.
    assert_eq!(h.snap.maps[0].kind, MapKind::Swimlane);
    h.quit();
}

/// Blocks placed by the core, a card made incoming, placed, moved and linked, all still there
/// after a restart; the commands that make no sense are refused and change nothing.
#[test]
fn blocks_cards_and_links_are_core_state_that_survives_a_restart() {
    use dovetail_api::canvas::Layout;
    use dovetail_api::canvas::geometry::{BLOCK_GAP, CARD_W, HEADER, PAD};

    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let effort = create_effort(&mut h, "providers", "providers");
    let other = create_effort(&mut h, "other", "other");
    h.send(Command::Canvas(CanvasCommand::CreateBlock {
        effort,
        title: String::new(),
        layout: Layout::QuestionTree,
        near: None,
    }));
    h.send(Command::Canvas(CanvasCommand::CreateBlock {
        effort: other,
        title: "Elsewhere".into(),
        layout: Layout::FreeArea,
        near: None,
    }));
    h.send(Command::Canvas(CanvasCommand::CreateBlock {
        effort,
        title: "Which provider?".into(),
        layout: Layout::OptionsCompare,
        near: None,
    }));
    // Where the user looks: centred there when it is empty, else the nearest empty spot.
    h.send(Command::Canvas(CanvasCommand::CreateBlock {
        effort,
        title: "Notes".into(),
        layout: Layout::FreeArea,
        near: Some(dovetail_api::canvas::Point { x: 145, y: 1000 }),
    }));
    h.send(Command::Canvas(CanvasCommand::CreateBlock {
        effort,
        title: "Crowded".into(),
        layout: Layout::FreeArea,
        near: Some(dovetail_api::canvas::Point { x: 145, y: 80 }),
    }));
    h.wait_for("five blocks", |s| s.blocks.len() == 5);
    let tree = h.snap.blocks[0].clone();
    let elsewhere = h.snap.blocks[1].id;
    let compare = h.snap.blocks[2].clone();
    let (w, hh) = (
        2 * PAD + CARD_W,
        HEADER + dovetail_api::canvas::geometry::CARD_H + PAD,
    );
    assert_eq!(
        (h.snap.blocks[3].x, h.snap.blocks[3].y),
        (145 - w / 2, 1000 - hh / 2)
    );
    // Centred on the tree block it would cover: moved off it, the tree did not move.
    let crowded = h.snap.blocks[4].clone();
    assert_ne!((crowded.x, crowded.y), (145 - w / 2, 80 - hh / 2));
    assert!(
        crowded.x >= tree.x + w + BLOCK_GAP / 2
            || crowded.y >= tree.y + hh + BLOCK_GAP / 2
            || crowded.x + w + BLOCK_GAP / 2 <= tree.x
            || crowded.y + hh + BLOCK_GAP / 2 <= tree.y,
        "{crowded:?}"
    );
    assert_eq!(h.snap.blocks[0], tree);
    assert_eq!(
        tree.title, "Question tree",
        "an empty title is the layout's name"
    );
    assert_eq!((tree.x, tree.y), (0, 0));
    // The second block of an effort goes right of the first; another effort's canvas starts empty.
    assert_eq!(h.snap.blocks[1].x, 0);
    assert_eq!((compare.x, compare.y), (2 * PAD + CARD_W + BLOCK_GAP, 0));

    h.send(Command::Canvas(CanvasCommand::AddCard {
        card: idea(effort, "Ask Mollie about EU-only pricing"),
        link_from: None,
    }));
    h.wait_for("the idea", |s| {
        card_titled(s, "Ask Mollie about EU-only pricing").is_some()
    });
    let first = card_titled(&h.snap, "Ask Mollie about EU-only pricing")
        .expect("card")
        .clone();
    assert_eq!((first.block, first.added_by.as_str()), (None, "user"));

    // Refused: moving an incoming card, placing it in another effort's block, dropping a card
    // that does not exist, a link to itself.
    h.send(Command::Canvas(CanvasCommand::MoveCard {
        id: first.id,
        x: 1,
        y: 1,
    }));
    h.send(Command::Canvas(CanvasCommand::PlaceCard {
        id: first.id,
        block: Some(elsewhere),
    }));
    h.send(Command::Canvas(CanvasCommand::DropIncoming {
        id: dovetail_api::canvas::CardId(999),
    }));
    h.send(Command::Canvas(CanvasCommand::LinkCards {
        from: first.id,
        to: first.id,
        label: String::new(),
    }));
    // Placed with no block named: the effort's newest block, in its first cell.
    h.send(Command::Canvas(CanvasCommand::PlaceCard {
        id: first.id,
        block: None,
    }));
    h.wait_for("placed", |s| {
        s.cards.iter().any(|c| c.block == Some(crowded.id))
    });
    let placed = card_titled(&h.snap, "Ask Mollie about EU-only pricing").expect("card");
    assert_eq!(
        (placed.x, placed.y, placed.seen),
        (crowded.x + PAD, crowded.y + HEADER, true)
    );
    assert_eq!(h.failures.len(), 4, "{:?}", h.failures);
    assert!(h.failures[0].contains("incoming"), "{:?}", h.failures);
    assert!(h.failures[1].contains("another effort"), "{:?}", h.failures);
    h.send(Command::Canvas(CanvasCommand::PlaceCard {
        id: first.id,
        block: None,
    }));
    h.send(Command::Canvas(CanvasCommand::DropIncoming {
        id: first.id,
    }));
    h.send(Command::Canvas(CanvasCommand::MoveCard {
        id: first.id,
        x: 41,
        y: 62,
    }));
    h.wait_for("moved", |s| s.cards.iter().any(|c| (c.x, c.y) == (41, 62)));
    assert_eq!(h.failures.len(), 6, "{:?}", h.failures);

    // A second card linked from the first on creation; a card of another effort cannot be linked.
    h.send(Command::Canvas(CanvasCommand::AddCard {
        card: idea(effort, "Compare Stripe and Adyen"),
        link_from: Some(first.id),
    }));
    h.send(Command::Canvas(CanvasCommand::AddCard {
        card: idea(other, "Unrelated"),
        link_from: None,
    }));
    h.wait_for("three cards", |s| s.cards.len() == 3);
    let second = card_titled(&h.snap, "Compare Stripe and Adyen")
        .expect("card")
        .id;
    let third = card_titled(&h.snap, "Unrelated").expect("card").id;
    assert_eq!(h.snap.card_links.len(), 1);
    h.send(Command::Canvas(CanvasCommand::LinkCards {
        from: first.id,
        to: third,
        label: String::new(),
    }));
    h.send(Command::Canvas(CanvasCommand::LinkCards {
        from: first.id,
        to: second,
        label: "next".into(),
    }));
    h.wait_for("relabelled", |s| {
        s.card_links.first().is_some_and(|l| l.label == "next")
    });
    assert_eq!(h.snap.card_links.len(), 1, "linking again only relabels");
    assert!(
        h.failures
            .last()
            .expect("failure")
            .contains("different efforts")
    );

    // Everything is the same after a restart, and the ids go on from where they were.
    let before = (
        h.snap.blocks.clone(),
        h.snap.cards.clone(),
        h.snap.card_links.clone(),
    );
    let paths = h.paths.clone();
    let _state = h.quit();
    let mut h = Harness::start_on(fixture("plain_turn"), paths);
    h.wait_for("the first snapshot", |s| !s.efforts.is_empty());
    assert_eq!(
        (
            h.snap.blocks.clone(),
            h.snap.cards.clone(),
            h.snap.card_links.clone()
        ),
        before
    );
    h.send(Command::Canvas(CanvasCommand::DeleteCard { id: second }));
    h.wait_for("deleted", |s| s.cards.len() == 2);
    assert!(h.snap.card_links.is_empty(), "a card's links go with it");
    h.send(Command::Canvas(CanvasCommand::RenameBlock {
        id: tree.id,
        title: "Providers".into(),
    }));
    h.wait_for("renamed", |s| s.blocks[0].title == "Providers");
    h.quit();
}

/// A chat reply copied into an idea card; a reference checked against its target; a card whose
/// target is gone reads "missing" and does not stop the snapshot.
#[test]
fn a_reply_becomes_an_idea_and_a_reference_must_exist() {
    use dovetail_api::canvas::{CardKind, CardRef, NewCard, RefKind};

    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let effort = create_effort(&mut h, "providers", "providers");
    h.send(Command::Conversation(ConversationCommand::Prompt {
        agent: AgentId(COORDINATOR.into()),
        text: "hello".into(),
        attachments: vec![],
        effort: None,
    }));
    h.wait_for("a reply", |s| {
        s.conversations
            .get(&AgentId(COORDINATOR.into()))
            .is_some_and(|t| {
                t.entries
                    .iter()
                    .any(|e| e.kind == EntryKind::Assistant && !e.text.is_empty())
            })
    });
    let reply = h.snap.conversations[&AgentId(COORDINATOR.into())]
        .entries
        .iter()
        .find(|e| e.kind == EntryKind::Assistant)
        .cloned()
        .expect("reply");
    h.send(Command::Canvas(CanvasCommand::CardFromReply {
        agent: AgentId(COORDINATOR.into()),
        entry: reply.id,
        effort,
    }));
    h.wait_for("the idea", |s| {
        s.cards.iter().any(|c| c.kind == CardKind::Idea)
    });
    assert_eq!(h.snap.cards[0].text, reply.text, "the words are copied");
    assert_eq!(h.snap.cards[0].block, None);

    let reference = |kind: RefKind, id: &str| NewCard {
        effort,
        block: None,
        kind: kind.card_kind(),
        reference: Some(CardRef {
            kind,
            id: id.into(),
        }),
        text: String::new(),
    };
    // A target that does not exist is refused.
    h.send(Command::Canvas(CanvasCommand::AddCard {
        card: reference(RefKind::Task, "77"),
        link_from: None,
    }));
    h.send(Command::Canvas(CanvasCommand::AddCard {
        card: reference(RefKind::Map, "no-such-map"),
        link_from: None,
    }));
    h.send(Command::Task(TaskCommand::Create(NewTask {
        title: "Pick a provider".into(),
        p_level: None,
        bucket: None,
        notes: "Notes on the pick".into(),
        follow_up_of: None,
        pr: None,
        related: vec![],
        effort: Some(effort),
        criterion: None,
    })));
    h.wait_for("the task", |s| !s.tasks.is_empty());
    let id = h.snap.tasks[0].id;
    h.send(Command::Canvas(CanvasCommand::AddCard {
        card: reference(RefKind::Task, &id.0.to_string()),
        link_from: None,
    }));
    h.wait_for("the task card", |s| {
        card_titled(s, "Pick a provider").is_some()
    });
    let card = card_titled(&h.snap, "Pick a provider")
        .expect("card")
        .clone();
    assert!(!card.missing && card.text.is_empty());
    assert_eq!(h.failures.len(), 2, "{:?}", h.failures);
    assert!(
        h.failures[0].contains("there is no task 77"),
        "{:?}",
        h.failures
    );
    h.quit();
}

/// The coordinator's canvas tools add blocks, cards and links; no tool moves or deletes. A block
/// goes in empty space, a card under its parent, marked unseen; nothing already placed moves.
#[test]
fn the_canvas_tools_add_and_link_and_nothing_else() {
    use dovetail_api::canvas::geometry::{CARD_H, GAP};
    use dovetail_api::canvas::{BlockId, CardId, CardKind, Layout};
    use dovetail_tools::{
        Suite, TOOLS, canvas::CanvasAddBlock, canvas::CanvasAddCard, canvas::CanvasLinkCards,
    };

    let canvas_tools: Vec<&str> = TOOLS
        .iter()
        .filter(|t| t.suite == Suite::Canvas)
        .map(|t| t.name)
        .collect();
    assert_eq!(
        canvas_tools,
        ["canvas_add_block", "canvas_add_card", "canvas_link_cards"]
    );

    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    let effort = create_effort(&mut h, "providers", "providers");
    let socket = h.socket();
    let add_to = |kind: CardKind,
                  text: &str,
                  reference: Option<&str>,
                  block: Option<BlockId>,
                  under: Option<CardId>| {
        Tool::Canvas(CanvasTool::AddCard(CanvasAddCard {
            effort: Some(effort),
            block,
            under,
            kind,
            text: text.into(),
            reference: reference.map(Into::into),
        }))
    };
    let add = |kind: CardKind, text: &str, reference: Option<&str>| {
        add_to(kind, text, reference, None, None)
    };
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            add(CardKind::Question, "Which provider?", None),
        ),
    ));
    assert!(!is_error && text.contains("incoming"), "{text}");
    let (_, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, add(CardKind::Idea, "Use Stripe", None)),
    ));
    assert!(!is_error);
    h.wait_for("two cards", |s| s.cards.len() == 2);
    assert!(
        h.snap
            .cards
            .iter()
            .all(|c| c.block.is_none() && c.added_by == COORDINATOR && c.seen)
    );
    let (from, to) = (h.snap.cards[0].id, h.snap.cards[1].id);
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            Tool::Canvas(CanvasTool::LinkCards(CanvasLinkCards {
                from,
                to,
                label: "answers".into(),
            })),
        ),
    ));
    assert!(!is_error, "{text}");
    h.wait_for("the link", |s| s.card_links.len() == 1);

    // A block, a root question in it and a finding-like idea under the root.
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            Tool::Canvas(CanvasTool::AddBlock(CanvasAddBlock {
                effort: Some(effort),
                layout: Layout::QuestionTree,
                title: "Which provider?".into(),
            })),
        ),
    ));
    assert!(!is_error && text.contains("block"), "{text}");
    h.wait_for("the block", |s| s.blocks.len() == 1);
    let block = h.snap.blocks[0].clone();
    let (_, is_error) = result_text(socket_call(
        &socket,
        tool(
            COORDINATOR,
            add_to(CardKind::Question, "Fees?", None, Some(block.id), None),
        ),
    ));
    assert!(!is_error);
    h.wait_for("the root", |s| card_titled(s, "Fees?").is_some());
    let root = card_titled(&h.snap, "Fees?").expect("root").clone();
    assert_eq!((root.block, root.seen), (Some(block.id), false));
    for text in ["Stripe 1.5 %", "Adyen: no list price"] {
        let (_, is_error) = result_text(socket_call(
            &socket,
            tool(
                COORDINATOR,
                add_to(CardKind::Idea, text, None, Some(block.id), Some(root.id)),
            ),
        ));
        assert!(!is_error);
    }
    h.wait_for("the answers and their links", |s| {
        s.cards.len() == 5 && s.card_links.len() == 3
    });
    let a = card_titled(&h.snap, "Stripe 1.5 %").expect("a").clone();
    let b = card_titled(&h.snap, "Adyen: no list price")
        .expect("b")
        .clone();
    // One row below the root, side by side, linked from it; the root did not move.
    assert_eq!((a.x, a.y), (root.x, root.y + CARD_H + GAP));
    assert_eq!(b.y, a.y);
    assert!(b.x > a.x);
    assert!(
        h.snap
            .card_links
            .iter()
            .any(|l| l.from == root.id && l.to == b.id)
    );
    assert_eq!(card_titled(&h.snap, "Fees?").expect("root"), &root);

    // Refused: a reference that does not exist, text on a reference card, an idea without text,
    // `under` with no block.
    for (call, wanted) in [
        (
            add(CardKind::Finding, "", Some("41")),
            "there is no finding 41",
        ),
        (add(CardKind::Finding, "words", Some("41")), "no `text`"),
        (add(CardKind::Idea, " ", None), "`text`"),
        (
            add_to(CardKind::Idea, "x", None, None, Some(root.id)),
            "name the `block`",
        ),
    ] {
        let (text, is_error) = result_text(socket_call(&socket, tool(COORDINATOR, call)));
        assert!(is_error && text.contains(wanted), "{text}");
    }
    h.drain(Duration::from_millis(100));
    assert_eq!(h.snap.cards.len(), 5);
    h.quit();
}

#[test]
fn a_preview_runs_in_the_worktree_serves_the_page_and_stops_with_the_slice() {
    let (mut h, _dir, _repo) = start_viewer_harness(Some(
        "preview = \"python3 -m http.server {port} --bind 127.0.0.1\"\n",
    ));
    assert_eq!(preview_of(&h.snap, "slice-web"), None);
    h.send(Command::Design(DesignCommand::StartPreview {
        agent: AgentId("slice-web".into()),
    }));
    h.wait_for("the preview to run", |s| {
        matches!(
            preview_of(s, "slice-web"),
            Some(dovetail_api::design::PreviewState::Running { .. })
        )
    });
    let Some(dovetail_api::design::PreviewState::Running { url }) =
        preview_of(&h.snap, "slice-web")
    else {
        unreachable!()
    };
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    let page = fetch(&url).expect("the preview answers");
    // The server lists the worktree's files: it runs there.
    assert!(page.contains("README"), "{page}");

    // Asking again while it runs changes nothing.
    h.send(Command::Design(DesignCommand::StartPreview {
        agent: AgentId("slice-web".into()),
    }));
    h.drain(Duration::from_millis(200));
    assert_eq!(
        preview_of(&h.snap, "slice-web"),
        Some(dovetail_api::design::PreviewState::Running { url: url.clone() })
    );

    // Archiving the slice stops it.
    let socket = h.socket();
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, archive_slice("web")),
    ));
    assert!(!is_error && text.starts_with(ARCHIVED), "{text}");
    assert!(gone(&url), "the preview still answers after archive");
    h.wait_for("the slice and its preview to go", |s| {
        preview_of(s, "slice-web").is_none() && !s.agents.iter().any(|a| a.name == "web")
    });
    h.quit();
}

#[test]
fn restarting_a_preview_starts_it_again_and_quitting_stops_it() {
    let (mut h, _dir, _repo) = start_viewer_harness(Some(
        "preview = \"python3 -m http.server {port} --bind 127.0.0.1\"\n",
    ));
    let agent = AgentId("slice-web".into());
    h.send(Command::Design(DesignCommand::StartPreview {
        agent: agent.clone(),
    }));
    h.wait_for("running", |s| {
        matches!(
            preview_of(s, "slice-web"),
            Some(dovetail_api::design::PreviewState::Running { .. })
        )
    });
    let Some(dovetail_api::design::PreviewState::Running { url: first }) =
        preview_of(&h.snap, "slice-web")
    else {
        unreachable!()
    };
    h.send(Command::Design(DesignCommand::RestartPreview {
        agent: agent.clone(),
    }));
    h.wait_for("running again on a new port", |s| {
        matches!(
            preview_of(s, "slice-web"),
            Some(dovetail_api::design::PreviewState::Running { url }) if url != first
        )
    });
    assert!(gone(&first), "the first run was stopped");
    let Some(dovetail_api::design::PreviewState::Running { url }) =
        preview_of(&h.snap, "slice-web")
    else {
        unreachable!()
    };
    assert!(fetch(&url).is_some());
    h.quit();
    assert!(gone(&url), "quitting stops the preview");
}

#[test]
fn a_slice_of_a_repo_with_no_preview_line_says_so_and_a_failing_command_shows_its_output() {
    let (mut h, _dir, _repo) = start_viewer_harness(None);
    h.send(Command::Design(DesignCommand::StartPreview {
        agent: AgentId("slice-web".into()),
    }));
    h.wait_for("no command", |s| {
        preview_of(s, "slice-web") == Some(dovetail_api::design::PreviewState::NoCommand)
    });
    h.quit();

    let (mut h, _dir, _repo) =
        start_viewer_harness(Some("preview = \"echo boom on port {port}; exit 3\"\n"));
    h.send(Command::Design(DesignCommand::StartPreview {
        agent: AgentId("slice-web".into()),
    }));
    h.wait_for("failed", |s| {
        matches!(
            preview_of(s, "slice-web"),
            Some(dovetail_api::design::PreviewState::Failed { .. })
        )
    });
    let Some(dovetail_api::design::PreviewState::Failed { output }) =
        preview_of(&h.snap, "slice-web")
    else {
        unreachable!()
    };
    assert!(output.contains("exited before it listened"), "{output}");
    assert!(output.contains("boom on port"), "{output}");
    h.quit();
}

#[test]
fn a_repo_override_replaces_the_projects_preview_line() {
    // The project line would fail; the repo's own (named after the repo) serves.
    let (mut h, _dir, _repo) = start_viewer_harness(Some(
        "preview = \"exit 1\"\n[repos.p]\npreview = \"python3 -m http.server {port} --bind 127.0.0.1\"\n",
    ));
    h.send(Command::Design(DesignCommand::StartPreview {
        agent: AgentId("slice-web".into()),
    }));
    h.wait_for("a preview state", |s| preview_of(s, "slice-web").is_some());
    h.wait_for("running", |s| {
        matches!(
            preview_of(s, "slice-web"),
            Some(
                dovetail_api::design::PreviewState::Running { .. }
                    | dovetail_api::design::PreviewState::Failed { .. }
            )
        ) && !matches!(
            preview_of(s, "slice-web"),
            Some(dovetail_api::design::PreviewState::Starting)
        )
    });
    assert!(
        matches!(
            preview_of(&h.snap, "slice-web"),
            Some(dovetail_api::design::PreviewState::Running { .. })
        ),
        "{:?}",
        preview_of(&h.snap, "slice-web")
    );
    h.quit();
}

#[test]
fn a_slice_declares_variants_the_user_picks_and_keeps_and_the_slice_is_prompted() {
    use dovetail_api::design::VariantLevel::{Component, Global};
    let mut h = Harness::start(fixture("plain_turn"), true, Config::default());
    h.send(Command::Agent(AgentCommand::NewSlice {
        repo: None,
        task: None,
        name: "later".into(),
        brief: "go".into(),
    }));
    h.wait_for("the slice to have run", |s| {
        s.conversations
            .get(&AgentId("slice-later".into()))
            .is_some_and(|t| t.entries.iter().any(|e| e.text == "alpha"))
    });
    let socket = h.socket();
    let agent = AgentId("slice-later".into());

    // Only a slice may declare; the coordinator is refused.
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(COORDINATOR, declare(vec![slot("theme", Global, &["a"])])),
    ));
    assert!(is_error && text.contains("for slices"), "{text}");
    // The whole list is checked: every problem comes back.
    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            "slice-later",
            declare(vec![
                slot("theme", Global, &["a", "a"]),
                slot("theme", Global, &[]),
            ]),
        ),
    ));
    assert!(
        is_error
            && text.contains("twice")
            && text.contains("needs 1 to 8 options")
            && text.contains("appears twice"),
        "{text}"
    );

    let (text, is_error) = result_text(socket_call(
        &socket,
        tool(
            "slice-later",
            declare(vec![
                slot("theme", Global, &["light", "dark"]),
                slot("header", Component, &["plain", "bold"]),
            ]),
        ),
    ));
    assert!(!is_error, "{text}");
    h.wait_for("the declared set", |s| s.variants.len() == 1);
    let set = h.snap.variants[0].clone();
    assert_eq!(set.selection["theme"], "light");
    assert_eq!(set.selection["header"], "plain");

    h.send(Command::Design(DesignCommand::SelectVariant {
        agent: agent.clone(),
        slot: "theme".into(),
        option: "dark".into(),
    }));
    h.wait_for("the pick", |s| {
        s.variants
            .first()
            .is_some_and(|v| v.selection["theme"] == "dark")
    });
    h.send(Command::Design(DesignCommand::SelectVariant {
        agent: agent.clone(),
        slot: "theme".into(),
        option: "neon".into(),
    }));
    h.drain(Duration::from_millis(300));
    assert!(
        h.failures.iter().any(|f| f.contains("not an option")),
        "{:?}",
        h.failures
    );

    // Declaring again keeps a pick that is still valid and drops one that is not.
    let (_, is_error) = result_text(socket_call(
        &socket,
        tool(
            "slice-later",
            declare(vec![
                slot("theme", Global, &["light", "dark", "sepia"]),
                slot("header", Component, &["compact"]),
            ]),
        ),
    ));
    assert!(!is_error);
    h.wait_for("the redeclared set", |s| {
        s.variants
            .first()
            .is_some_and(|v| v.slots.len() == 2 && v.slots[1].options == ["compact"])
    });
    assert_eq!(h.snap.variants[0].selection["theme"], "dark");
    assert_eq!(h.snap.variants[0].selection["header"], "compact");

    // Keep: the picks become the kept options and the slice is prompted.
    h.send(Command::Design(DesignCommand::KeepVariants {
        agent: agent.clone(),
    }));
    h.wait_for("the kept options", |s| {
        s.variants
            .first()
            .is_some_and(|v| v.slots[0].kept.as_deref() == Some("dark"))
    });
    h.wait_for("the prompt in the slice's conversation", |s| {
        s.conversations
            .get(&agent)
            .is_some_and(|t| t.entries.iter().any(|e| e.kind == EntryKind::User && e.text.contains("Keep this combination of variants: theme (global): dark; header (component): compact")))
    });

    // An empty declaration clears the list.
    let (_, is_error) = result_text(socket_call(&socket, tool("slice-later", declare(vec![]))));
    assert!(!is_error);
    h.wait_for("the cleared list", |s| s.variants.is_empty());
    h.quit();
}

#[test]
fn declared_variants_and_picks_survive_a_restart() {
    use dovetail_api::design::VariantLevel::Global;
    let (mut h, dir, repo) = start_viewer_harness(None);
    let paths = h.paths.clone();
    let socket = h.socket();
    let (_, is_error) = result_text(socket_call(
        &socket,
        tool(
            "slice-web",
            declare(vec![slot("theme", Global, &["light", "dark"])]),
        ),
    ));
    assert!(!is_error);
    h.send(Command::Design(DesignCommand::SelectVariant {
        agent: AgentId("slice-web".into()),
        slot: "theme".into(),
        option: "dark".into(),
    }));
    h.wait_for("the pick", |s| {
        s.variants
            .first()
            .is_some_and(|v| v.selection["theme"] == "dark")
    });
    h.quit();
    let h = Harness::start_with(fixture("plain_turn"), paths, Some(repo), Config::default());
    assert_eq!(h.snap.variants.len(), 1, "{:?}", h.snap.variants);
    assert_eq!(h.snap.variants[0].selection["theme"], "dark");
    drop(dir);
    h.quit();
}

fn draw(h: &Harness, map: serde_json::Value) -> (String, bool) {
    result_text(socket_call(
        &h.socket(),
        tool(COORDINATOR, Tool::Map(MapTool::Draw(DrawMap(map)))),
    ))
}

fn checkout_map(title: &str) -> serde_json::Value {
    serde_json::json!({
        "map": { "title": title, "type": "structure" },
        "nodes": [
            { "id": "api", "kind": "endpoint", "label": "POST /checkout", "description": "Takes the order." },
            { "id": "orders", "kind": "data", "label": "orders", "description": "Orders table." }
        ],
        "connections": [ { "from": "api", "to": "orders", "label": "writes" } ]
    })
}

fn login_lanes(lane_node: &str) -> serde_json::Value {
    serde_json::json!({
        "map": { "title": "Logging in", "type": "swimlane" },
        "nodes": [
            { "id": "user", "kind": "actor", "label": "User", "description": "Signs in." },
            { "id": "check", "kind": "endpoint", "label": "/login", "description": "Checks it." }
        ],
        "lanes": [
            { "id": "you", "label": "User" },
            { "id": "web", "label": "Web", "node": lane_node }
        ],
        "flows": [ { "name": "Log in", "steps": [
            { "node": "user", "lane": "you", "note": "Types a password." },
            { "node": "check", "lane": "web", "arrow": "submits", "note": "Checks it." }
        ] } ]
    })
}

/// Sends `LoadMap` and waits for its answer, folding events in on the way.
fn load_map(h: &mut Harness, id: &str, version: Option<u32>) -> Result<Map, String> {
    h.send(Command::Map(MapCommand::Load {
        id: MapId(id.into()),
        version,
    }));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .expect("timed out waiting for the map");
        match h.link.messages.recv_timeout(left).expect("a message") {
            ServerMsg::Event { seq, event } => {
                let answer = match &event {
                    Event::Map(MapEvent::Loaded(map)) => Some(Ok(map.clone())),
                    Event::App(AppEvent::CommandFailed { message }) => Some(Err(message.clone())),
                    _ => None,
                };
                apply(&mut h.snap, seq, event);
                if let Some(answer) = answer {
                    return answer;
                }
            }
            ServerMsg::Snapshot(s) => h.snap = s,
            ServerMsg::Hello { .. } => {}
        }
    }
}

fn idea(effort: dovetail_api::effort::EffortId, text: &str) -> dovetail_api::canvas::NewCard {
    dovetail_api::canvas::NewCard {
        effort,
        block: None,
        kind: dovetail_api::canvas::CardKind::Idea,
        reference: None,
        text: text.into(),
    }
}

fn card_titled<'a>(s: &'a Snapshot, title: &str) -> Option<&'a dovetail_api::canvas::Card> {
    s.cards.iter().find(|c| c.title == title)
}

/// The preview state of slice `name` in the snapshot.
fn preview_of(s: &Snapshot, agent: &str) -> Option<dovetail_api::design::PreviewState> {
    s.previews
        .iter()
        .find(|p| p.agent.0 == agent)
        .map(|p| p.state.clone())
}

/// `GET /` of a preview URL, as plain HTTP on a socket: the body, or `None` if nothing answers.
fn fetch(url: &str) -> Option<String> {
    let addr = url.strip_prefix("http://")?.trim_end_matches('/');
    let mut stream = std::net::TcpStream::connect(addr).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    stream
        .write_all(format!("GET / HTTP/1.0\r\nHost: {addr}\r\n\r\n").as_bytes())
        .ok()?;
    let mut body = String::new();
    std::io::Read::read_to_string(&mut stream, &mut body).ok()?;
    Some(body)
}

/// Waits (up to 5 s) for nothing to answer at `url`.
fn gone(url: &str) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if fetch(url).is_none() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn start_viewer_harness(recipe: Option<&str>) -> (Harness, TempDir, PathBuf) {
    let (dir, repo, paths) = repo_with_slices(&["web"]);
    if let Some(text) = recipe {
        write_recipe(&paths, text);
    }
    let mut h = Harness::start_with(
        fixture("plain_turn"),
        paths.clone(),
        Some(repo.clone()),
        Config::default(),
    );
    h.wait_for("the slice", |s| s.agents.iter().any(|a| a.name == "web"));
    (h, dir, repo)
}

fn declare(slots: Vec<dovetail_api::design::VariantSlot>) -> Tool {
    Tool::Design(DesignTool::DeclareVariants(
        dovetail_tools::design::DeclareVariants { slots },
    ))
}

fn slot(
    name: &str,
    level: dovetail_api::design::VariantLevel,
    options: &[&str],
) -> dovetail_api::design::VariantSlot {
    dovetail_api::design::VariantSlot {
        name: name.into(),
        level,
        options: options.iter().map(|o| (*o).into()).collect(),
        kept: None,
    }
}
