//! Tests for the `map` feature.

use crate::common::{Case, map_summary, roundtrip, run, sample_map};
use dovetail_api::map::{Map, MapCommand, MapContent, MapEvent, MapId, MapKind, MapSummary};
use dovetail_api::{Command, Event, ServerMsg, Snapshot};

fn shelf_entry(id: &str, version: u32) -> MapSummary {
    MapSummary {
        id: MapId(id.into()),
        title: id.into(),
        kind: MapKind::Structure,
        task: None,
        drawn_by: None,
        version,
        created_ms: 0,
        updated_ms: i64::from(version),
    }
}

pub fn cases() -> Vec<Case> {
    vec![
        Case::new(
            "map",
            Snapshot::default(),
            vec![(1, Event::Map(MapEvent::Upserted(map_summary())))],
        ),
        Case::no_op(
            "map_loaded",
            Snapshot::default(),
            vec![(1, Event::Map(MapEvent::Loaded(sample_map())))],
        ),
    ]
}

#[test]
fn map_commands_and_events_roundtrip() {
    for (n, event) in [
        Event::Map(MapEvent::Upserted(map_summary())),
        Event::Map(MapEvent::Loaded(sample_map())),
    ]
    .into_iter()
    .enumerate()
    {
        roundtrip(&ServerMsg::Event {
            seq: n as u64,
            event,
        });
    }

    roundtrip(&Command::Map(MapCommand::Load {
        id: MapId("checkout".into()),
        version: Some(2),
    }));
}

#[test]
fn a_drawn_map_goes_to_the_front_of_the_shelf_and_a_redraw_replaces_it() {
    let mut s = Snapshot::default();
    run(
        &mut s,
        vec![
            Event::Map(MapEvent::Upserted(shelf_entry("a", 1))),
            Event::Map(MapEvent::Upserted(shelf_entry("b", 1))),
            Event::Map(MapEvent::Upserted(shelf_entry("a", 2))),
        ],
    );
    let shelf: Vec<(&str, u32)> = s
        .maps
        .iter()
        .map(|m| (m.id.0.as_str(), m.version))
        .collect();
    assert_eq!(shelf, [("a", 2), ("b", 1)]);
    // Loaded content passes through without touching the snapshot.
    let before = s.maps.clone();
    let loaded = Map {
        id: MapId("a".into()),
        version: 2,
        content: MapContent {
            title: "a".into(),
            kind: MapKind::Structure,
            nodes: vec![],
            groups: vec![],
            connections: vec![],
            flows: vec![],
            lanes: vec![],
        },
    };
    run(&mut s, vec![Event::Map(MapEvent::Loaded(loaded))]);
    assert_eq!(s.maps, before);
}

#[test]
fn map_loaded_only_advances_seq() {
    let mut state = Snapshot::default();
    let mut expected = state.clone();
    expected.seq = 1;
    run(&mut state, vec![Event::Map(MapEvent::Loaded(sample_map()))]);
    assert_eq!(state, expected);
}
