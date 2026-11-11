use dovetail_api::map::{
    Connection, Flow, FlowStep, Group, Lane, LaneId, MapContent, MapKind, Node, NodeId, NodeKind,
};

use crate::render::extract_placements;
use crate::{MapRenderer, RenderError, RenderOptions};

fn node(id: &str, kind: NodeKind, label: &str) -> Node {
    Node {
        id: NodeId(id.into()),
        kind,
        label: label.into(),
        description: String::new(),
        file: None,
        pseudocode: None,
    }
}

fn conn(from: &str, to: &str, label: &str) -> Connection {
    Connection {
        from: NodeId(from.into()),
        to: NodeId(to.into()),
        label: label.into(),
    }
}

fn id(s: &str) -> NodeId {
    NodeId(s.into())
}

/// Core (a group) holds db and api; the user calls api; api reads db. `end` is a valid map id but
/// a reserved word in Mermaid.
fn sample() -> MapContent {
    MapContent {
        title: "Sample".into(),
        kind: MapKind::Structure,
        nodes: vec![
            node("core", NodeKind::Part, "Core"),
            node("db", NodeKind::Data, "Tasks \"db\""),
            node("api", NodeKind::Endpoint, "POST /tasks"),
            node("user", NodeKind::Actor, "User"),
            node("end", NodeKind::Logic, "if ok\nsave"),
        ],
        groups: vec![Group {
            parent: id("core"),
            children: vec![id("db"), id("api")],
        }],
        connections: vec![
            conn("user", "api", "calls"),
            conn("api", "db", ""),
            conn("end", "core", "x"),
        ],
        flows: vec![],
        lanes: vec![],
    }
}

#[test]
fn mermaid_without_selection() {
    insta::assert_snapshot!(MapRenderer::mermaid(&sample(), None), @r#"
    ---
    config:
      layout: elk
      theme: dark
      look: classic
    ---
    flowchart TB
      subgraph n0["Core"]
        n1[("Tasks #quot;db#quot;")]
        n2("POST /tasks")
      end
      n3(["☺ User"])
      n4["ƒ if ok<br/>save"]
      n3 -->|"calls"| n2
      n2 --> n1
      n4 -->|"x"| n0
      classDef data fill:#e8a93a,stroke:#9b6a12,stroke-width:2px,color:#2a1b00
      class n1 data
      classDef endpoint fill:#4cc3d9,stroke:#1d7f91,stroke-width:2px,color:#0b2a30
      class n2 endpoint
      classDef actor fill:#f2a1b6,stroke:#b5546f,stroke-width:2px,color:#3a0f1c
      class n3 actor
      classDef logic fill:#8f9be8,stroke:#4c58a8,stroke-width:1px,color:#101433,font-family:monospace
      class n4 logic
    "#);
}

#[test]
fn mermaid_with_selected_node_has_one_combined_class() {
    insta::assert_snapshot!(MapRenderer::mermaid(&sample(), Some(&id("api"))), @r#"
    ---
    config:
      layout: elk
      theme: dark
      look: classic
    ---
    flowchart TB
      subgraph n0["Core"]
        n1[("Tasks #quot;db#quot;")]
        n2("POST /tasks")
      end
      n3(["☺ User"])
      n4["ƒ if ok<br/>save"]
      n3 -->|"calls"| n2
      n2 --> n1
      n4 -->|"x"| n0
      classDef data fill:#e8a93a,stroke:#9b6a12,stroke-width:2px,color:#2a1b00
      class n1 data
      classDef actor fill:#f2a1b6,stroke:#b5546f,stroke-width:2px,color:#3a0f1c
      class n3 actor
      classDef logic fill:#8f9be8,stroke:#4c58a8,stroke-width:1px,color:#101433,font-family:monospace
      class n4 logic
      classDef endpointSel fill:#4cc3d9,color:#0b2a30,stroke:#ffd84a,stroke-width:6px
      class n2 endpointSel
    "#);
}

#[test]
fn selected_node_is_in_exactly_one_class_line() {
    let source = MapRenderer::mermaid(&sample(), Some(&id("api")));
    // n2 is "api": merman's PNG only reads a node's first class, so it must appear once.
    let lines = source
        .lines()
        .filter(|l| l.trim_start().starts_with("class "))
        .collect::<Vec<_>>();
    let mentions: Vec<_> = lines
        .iter()
        .filter(|l| l.split([' ', ',']).any(|w| w == "n2"))
        .collect();
    assert_eq!(mentions.len(), 1, "{lines:?}");
    assert!(mentions[0].ends_with("endpointSel"));
}

#[test]
fn every_kind_has_a_style_and_a_selected_style() {
    let nodes: Vec<Node> = NodeKind::ALL
        .iter()
        .map(|k| node(&format!("a-{}", k.as_str()), *k, k.as_str()))
        .collect();
    for (i, kind) in NodeKind::ALL.iter().enumerate() {
        let content = MapContent {
            nodes: nodes.clone(),
            groups: vec![],
            connections: vec![],
            ..sample()
        };
        let source = MapRenderer::mermaid(&content, Some(&nodes[i].id));
        assert!(
            source.contains(&format!("classDef {}Sel ", kind.as_str())),
            "{kind:?}"
        );
        assert!(source.contains("stroke:#ffd84a"), "{kind:?}");
    }
}

#[test]
fn hostile_labels_are_escaped() {
    let mut content = sample();
    content.nodes[3].label = "a\"b <i>&#1</i>".into();
    let source = MapRenderer::mermaid(&content, None);
    assert!(
        source.contains("☺ a#quot;b #lt;i#gt;#amp;#35;1#lt;/i#gt;"),
        "{source}"
    );
}

/// Layout JSON as merman writes it, trimmed to what the extractor reads. Bounds start at (10, 20),
/// so the viewBox starts at (2, 12); `scale` 2 doubles everything.
const LAYOUT: &str = r#"{
    "semantic": { "nodes": [ { "id": "wrong", "x": 1, "y": 1, "width": 1, "height": 1 } ] },
    "layout": { "FlowchartV2": {
        "bounds": { "min_x": 10.0, "min_y": 20.0, "max_x": 300.0, "max_y": 200.0 },
        "nodes": [
            { "id": "n0", "x": 100.0, "y": 80.0, "width": 160.0, "height": 120.0, "is_cluster": true },
            { "id": "n1", "x": 100.0, "y": 80.0, "width": 40.0, "height": 20.0, "is_cluster": false }
        ],
        "edges": []
    } }
}"#;

fn two_nodes() -> MapContent {
    MapContent {
        nodes: vec![
            node("core", NodeKind::Part, "Core"),
            node("db", NodeKind::Data, "db"),
        ],
        groups: vec![Group {
            parent: id("core"),
            children: vec![id("db")],
        }],
        connections: vec![],
        ..sample()
    }
}

#[test]
fn positions_come_from_layout_json_in_png_pixels() {
    let ids = ["n0".to_string(), "n1".to_string()];
    let placed = extract_placements(LAYOUT.as_bytes(), &two_nodes(), &ids, 2.0).unwrap();
    assert_eq!(placed.len(), 2);

    // Top-left of n1 in user units is (80, 70); minus the viewBox origin (2, 12), times 2.
    assert_eq!(placed[1].id, id("db"));
    assert_eq!((placed[1].rect.x, placed[1].rect.y), (156.0, 116.0));
    assert_eq!((placed[1].rect.width, placed[1].rect.height), (80.0, 40.0));
    assert!(!placed[1].is_group);

    assert_eq!(placed[0].id, id("core"));
    assert!(placed[0].is_group);
    assert_eq!((placed[0].rect.x, placed[0].rect.y), (36.0, 16.0));
}

#[test]
fn layout_json_in_the_wrong_shape_is_an_error() {
    let ids = ["n0".to_string()];
    let err = extract_placements(br#"{"layout":{}}"#, &two_nodes(), &ids, 1.0).unwrap_err();
    assert!(matches!(err, RenderError::Layout(_)), "{err}");
}

fn yellow_pixels(png_bytes: &[u8]) -> usize {
    let mut dec = png::Decoder::new(std::io::Cursor::new(png_bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba);
    buf[..info.buffer_size()]
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| {
            p[0].abs_diff(0xff) < 30 && p[1].abs_diff(0xd8) < 30 && p[2].abs_diff(0x4a) < 30
        })
        .count()
}

#[test]
fn renders_png_with_placements_and_a_visible_selection() {
    let renderer = MapRenderer::new();
    let plain = renderer
        .render(&sample(), &RenderOptions::default())
        .unwrap();
    assert_eq!(plain.nodes.len(), 5);
    assert_eq!(yellow_pixels(&plain.png), 0);

    let selected = renderer
        .render(
            &sample(),
            &RenderOptions {
                selected: Some(id("api")),
                fit: None,
            },
        )
        .unwrap();
    assert!(
        yellow_pixels(&selected.png) > 200,
        "selection ring missing from the PNG"
    );
    // Selecting changes style only; layout must not move.
    assert_eq!(plain.nodes, selected.nodes);

    for p in &plain.nodes {
        let r = p.rect;
        assert!(r.x >= 0.0 && r.y >= 0.0, "{p:?}");
        assert!(r.x + r.width <= f64::from(plain.width) + 1.0, "{p:?}");
        assert!(r.y + r.height <= f64::from(plain.height) + 1.0, "{p:?}");
    }

    // The group's box encloses its children, and a hit inside a child finds the child.
    let rect = |name: &str| plain.nodes.iter().find(|p| p.id == id(name)).unwrap().rect;
    let (core, api) = (rect("core"), rect("api"));
    assert!(core.x <= api.x && core.y <= api.y);
    assert!(core.x + core.width >= api.x + api.width && core.y + core.height >= api.y + api.height);
    let (cx, cy) = (api.x + api.width / 2.0, api.y + api.height / 2.0);
    assert_eq!(plain.hit_test(cx, cy), Some(&id("api")));
    assert_eq!(
        plain.hit_test(core.x + 2.0, core.y + 2.0),
        Some(&id("core"))
    );
    assert_eq!(plain.hit_test(-5.0, -5.0), None);
}

#[test]
fn fit_box_limits_the_png_size() {
    let out = MapRenderer::new()
        .render(
            &sample(),
            &RenderOptions {
                selected: None,
                fit: Some((300, 300)),
            },
        )
        .unwrap();
    assert!(
        out.width <= 300 && out.height <= 300,
        "{}x{}",
        out.width,
        out.height
    );
}

#[test]
fn a_swimlane_without_a_flow_and_an_empty_map_are_refused() {
    let renderer = MapRenderer::new();
    let swim = MapContent {
        kind: MapKind::Swimlane,
        ..sample()
    };
    assert!(matches!(
        renderer.render(&swim, &RenderOptions::default()),
        Err(RenderError::Layout(_))
    ));
    let empty = MapContent {
        nodes: vec![],
        groups: vec![],
        connections: vec![],
        ..sample()
    };
    assert!(matches!(
        renderer.render(&empty, &RenderOptions::default()),
        Err(RenderError::Empty)
    ));
}

/// The user's request goes to the api and back: two lanes, three steps.
fn swimlane() -> MapContent {
    let lane = |l: &str| Lane {
        id: LaneId(l.into()),
        label: l.into(),
        node: None,
    };
    let step = |n: &str, l: &str| FlowStep {
        node: id(n),
        connection: None,
        arrow: Some("then".into()),
        note: String::new(),
        lane: Some(LaneId(l.into())),
    };
    MapContent {
        title: "Request".into(),
        kind: MapKind::Swimlane,
        nodes: vec![
            node("user", NodeKind::Actor, "User <&>"),
            node("api", NodeKind::Endpoint, "POST /tasks"),
        ],
        groups: vec![],
        connections: vec![],
        flows: vec![Flow {
            name: "request".into(),
            steps: vec![
                step("user", "you"),
                step("api", "server"),
                step("user", "you"),
            ],
        }],
        lanes: vec![lane("you"), lane("server")],
    }
}

#[test]
fn a_swimlane_renders_with_one_box_per_step_inside_the_picture() {
    let map = MapRenderer::new()
        .render(
            &swimlane(),
            &RenderOptions {
                selected: None,
                fit: Some((800, 600)),
            },
        )
        .expect("a valid swimlane renders");
    assert!(map.png.starts_with(b"\x89PNG"));
    assert!(map.width <= 800 && map.height <= 600);
    let steps: Vec<Option<usize>> = map.nodes.iter().map(|n| n.step).collect();
    assert_eq!(steps, vec![Some(0), Some(1), Some(2)]);
    for n in &map.nodes {
        assert!(n.rect.x >= 0.0 && n.rect.x + n.rect.width <= f64::from(map.width));
        assert!(n.rect.y >= 0.0 && n.rect.y + n.rect.height <= f64::from(map.height));
    }
    // The api's box is in the second lane, below the user's.
    assert!(map.nodes[1].rect.y > map.nodes[0].rect.y);
    let (cx, cy) = (
        map.nodes[1].rect.x + map.nodes[1].rect.width / 2.0,
        map.nodes[1].rect.y + map.nodes[1].rect.height / 2.0,
    );
    assert_eq!(map.hit_test(cx, cy), Some(&id("api")));
}
