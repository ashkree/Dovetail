//! Tests for the `canvas` feature.

use crate::common::{Case, canvas_card, id, roundtrip, run};
use dovetail_api::canvas::{
    Block, BlockId, CanvasCommand, CanvasEvent, Card, CardId, CardKind, CardLink, CardRef, Layout,
    NewCard, Point, RefKind,
};
use dovetail_api::conversation::EntryId;
use dovetail_api::effort::EffortId;
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

fn card(id: i64, kind: CardKind, block: Option<i64>, title: &str) -> Card {
    Card {
        id: CardId(id),
        effort: EffortId(2),
        block: block.map(BlockId),
        kind,
        x: 0,
        y: 0,
        reference: None,
        title: title.into(),
        text: String::new(),
        missing: false,
        added_by: "user".into(),
        created_ms: 1,
        seen: true,
    }
}

/// Blocks, cards and links: a card arrives in incoming, is placed and moved, linked, relabelled
/// and unlinked; removing a card drops its links.
pub fn cases() -> Vec<Case> {
    let mut placed = card(1, CardKind::Question, Some(1), "Which provider?");
    placed.text = "Which provider?".into();
    let mut incoming = card(2, CardKind::Finding, None, "Stripe fees");
    incoming.reference = Some(CardRef {
        kind: RefKind::Finding,
        id: "41".into(),
    });
    let mut on_canvas = incoming.clone();
    on_canvas.block = Some(BlockId(1));
    on_canvas.x = 320;
    on_canvas.y = 180;
    let mut moved = on_canvas.clone();
    moved.x = 340;
    let link = |label: &str| CardLink {
        from: CardId(1),
        to: CardId(2),
        label: label.into(),
    };
    let events = vec![
        Event::Canvas(CanvasEvent::BlockUpserted(Block {
            id: BlockId(1),
            effort: EffortId(2),
            title: "guest-pay".into(),
            layout: Layout::QuestionTree,
            x: 0,
            y: 0,
        })),
        Event::Canvas(CanvasEvent::CardUpserted(placed)),
        Event::Canvas(CanvasEvent::CardUpserted(incoming)),
        Event::Canvas(CanvasEvent::CardUpserted(on_canvas)),
        Event::Canvas(CanvasEvent::CardUpserted(moved)),
        Event::Canvas(CanvasEvent::CardLinked(link(""))),
        Event::Canvas(CanvasEvent::CardLinked(link("answers"))),
        Event::Canvas(CanvasEvent::CardLinked(CardLink {
            from: CardId(1),
            to: CardId(3),
            label: String::new(),
        })),
        Event::Canvas(CanvasEvent::CardUnlinked {
            from: CardId(1),
            to: CardId(3),
        }),
        Event::Canvas(CanvasEvent::CardLoaded {
            id: CardId(2),
            body: "Stripe charges 1.5 %.".into(),
        }),
        Event::Canvas(CanvasEvent::CardRemoved(CardId(1))),
    ];
    vec![Case::new(
        "canvas",
        Snapshot::default(),
        events
            .into_iter()
            .enumerate()
            .map(|(i, e)| (i as u64 + 1, e))
            .collect(),
    )]
}

#[test]
fn canvas_commands_and_events_roundtrip() {
    let events = vec![
        Event::Canvas(CanvasEvent::BlockUpserted(Block {
            id: BlockId(4),
            effort: EffortId(7),
            title: "payment-providers".into(),
            layout: Layout::FreeArea,
            x: 400,
            y: 0,
        })),
        Event::Canvas(CanvasEvent::CardUpserted(canvas_card())),
        Event::Canvas(CanvasEvent::CardRemoved(CardId(90))),
        Event::Canvas(CanvasEvent::CardLinked(CardLink {
            from: CardId(1),
            to: CardId(2),
            label: String::new(),
        })),
        Event::Canvas(CanvasEvent::CardUnlinked {
            from: CardId(1),
            to: CardId(2),
        }),
        Event::Canvas(CanvasEvent::CardLoaded {
            id: CardId(90),
            body: "Stripe charges 1.5 %.".into(),
        }),
    ];
    for (n, event) in events.into_iter().enumerate() {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }

    let commands = vec![
        Command::Canvas(CanvasCommand::CreateBlock {
            effort: EffortId(7),
            title: String::new(),
            layout: Layout::OptionsCompare,
            near: Some(Point { x: 300, y: 120 }),
        }),
        Command::Canvas(CanvasCommand::RenameBlock {
            id: BlockId(4),
            title: "options".into(),
        }),
        Command::Canvas(CanvasCommand::AddCard {
            card: NewCard {
                effort: EffortId(7),
                block: Some(BlockId(4)),
                kind: CardKind::Idea,
                reference: None,
                text: "Ask Mollie".into(),
            },
            link_from: Some(CardId(90)),
        }),
        Command::Canvas(CanvasCommand::PlaceCard {
            id: CardId(92),
            block: None,
        }),
        Command::Canvas(CanvasCommand::SeeCard { id: CardId(92) }),
        Command::Canvas(CanvasCommand::MoveCard {
            id: CardId(92),
            x: 41,
            y: 61,
        }),
        Command::Canvas(CanvasCommand::DeleteCard { id: CardId(92) }),
        Command::Canvas(CanvasCommand::DropIncoming { id: CardId(92) }),
        Command::Canvas(CanvasCommand::LinkCards {
            from: CardId(1),
            to: CardId(2),
            label: "answers".into(),
        }),
        Command::Canvas(CanvasCommand::UnlinkCards {
            from: CardId(1),
            to: CardId(2),
        }),
        Command::Canvas(CanvasCommand::CardFromReply {
            agent: id("coordinator"),
            entry: EntryId(5),
            effort: EffortId(7),
        }),
        Command::Canvas(CanvasCommand::LoadCard { id: CardId(90) }),
    ];
    for command in &commands {
        roundtrip(command);
    }
}

#[test]
fn card_loaded_only_advances_seq() {
    let mut state = Snapshot::default();
    let mut expected = state.clone();
    expected.seq = 1;
    run(
        &mut state,
        vec![Event::Canvas(CanvasEvent::CardLoaded {
            id: CardId(90),
            body: "Stripe charges 1.5 %.".into(),
        })],
    );
    assert_eq!(state, expected);
}
