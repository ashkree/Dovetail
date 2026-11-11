//! `maps_draw` input validation (design 23i): every problem found, each saying how to fix it.

use std::collections::{HashMap, HashSet};

use dovetail_api::map::{
    Connection, Flow, FlowStep, Group, Lane, LaneId, Map, MapContent, MapId, MapKind, Node, NodeId,
    NodeKind,
};

use crate::input::DrawMapInput;

/// A map that passed validation but has no id or version yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidMap {
    /// The id the agent asked to update, if any; storage chooses one for a new map.
    pub requested_id: Option<MapId>,
    /// The map itself.
    pub content: MapContent,
}

impl ValidMap {
    /// Stamps the id and version storage gave it.
    pub fn into_map(self, id: MapId, version: u32) -> Map {
        Map {
            id,
            version,
            content: self.content,
        }
    }
}

/// Size limits on one map. They keep a map readable on one screen and its stored JSON small;
/// past them the agent is told to split the map. Text lengths count characters.
pub mod limits {
    /// Nodes on one map.
    pub const MAX_NODES: usize = 150;
    /// Connections on one map.
    pub const MAX_CONNECTIONS: usize = 300;
    /// Groups on one map.
    pub const MAX_GROUPS: usize = 50;
    /// Flows on one map.
    pub const MAX_FLOWS: usize = 20;
    /// Steps in one flow.
    pub const MAX_STEPS: usize = 60;
    /// Lanes on one swimlane map.
    pub const MAX_LANES: usize = 12;
    /// A map, node or lane id.
    pub const MAX_ID: usize = 64;
    /// The map's title.
    pub const MAX_TITLE: usize = 100;
    /// A node, lane, connection or arrow label, or a flow's name.
    pub const MAX_LABEL: usize = 60;
    /// A node's description.
    pub const MAX_DESCRIPTION: usize = 1000;
    /// A flow step's note.
    pub const MAX_NOTE: usize = 500;
    /// A node's pseudocode.
    pub const MAX_PSEUDOCODE: usize = 2000;
    /// A node's file link.
    pub const MAX_FILE: usize = 300;
    /// Problems listed by [`crate::report`]; the rest are counted.
    pub const MAX_REPORTED_ERRORS: usize = 40;
}

use limits::*;

/// One problem with a `maps_draw` input. `at` names where it is, e.g. `nodes[3] ("api")`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MapError {
    /// The map has no nodes at all.
    #[error("the map has no nodes; add at least one under \"nodes\"")]
    NoNodes,
    /// A list is over its limit.
    #[error(
        "\"{list}\" has {count} entries, more than the limit of {limit}; split it into smaller \
         maps, or fold detail into groups"
    )]
    TooMany {
        /// The list, e.g. `nodes` or `flows[0].steps`.
        list: String,
        /// How many it has.
        count: usize,
        /// The limit.
        limit: usize,
    },
    /// A required field is absent or blank.
    #[error("{at}: \"{field}\" is missing or empty; {hint}")]
    Missing {
        /// Where.
        at: String,
        /// The field.
        field: &'static str,
        /// What to put there.
        hint: &'static str,
    },
    /// A text field is over its limit.
    #[error("{at}: \"{field}\" is {len} characters, more than the limit of {limit}; shorten it")]
    TooLong {
        /// Where.
        at: String,
        /// The field.
        field: &'static str,
        /// Its length in characters.
        len: usize,
        /// The limit.
        limit: usize,
    },
    /// An id with characters other than letters, digits, `_` and `-`.
    #[error(
        "{at}: id \"{id}\" may only use letters, digits, \"_\" and \"-\"; for example \"{}\"",
        suggest_id(.id)
    )]
    BadId {
        /// Where.
        at: String,
        /// The id given.
        id: String,
    },
    /// `map.type` is neither `structure` nor `swimlane`.
    #[error(
        "map: type \"{value}\" is not a map type; use \"structure\" (what exists and how it \
         connects) or \"swimlane\" (what happens in order)"
    )]
    UnknownMapType {
        /// The type given.
        value: String,
    },
    /// A node kind outside the nine.
    #[error(
        "{at}: kind \"{value}\" is not a node kind; use one of {}, and \"concept\" for anything else",
        kind_list()
    )]
    UnknownNodeKind {
        /// Where.
        at: String,
        /// The kind given.
        value: String,
    },
    /// The same id or name twice.
    #[error("{at}: {what} \"{value}\" is already used by {first}; make each {what} unique")]
    Duplicate {
        /// Where the second one is.
        at: String,
        /// `node id`, `lane id`, `flow name` or `group parent`.
        what: &'static str,
        /// The repeated value.
        value: String,
        /// Where the first one is.
        first: String,
    },
    /// A reference to a node id that isn't on the map; for a group's `parent`, a missing parent.
    #[error(
        "{at}: no node has id \"{id}\"; add a node with that id under \"nodes\", or fix the id"
    )]
    UnknownNode {
        /// Where the reference is.
        at: String,
        /// The id referred to.
        id: String,
    },
    /// A step naming a lane that isn't on the map.
    #[error("{at}: no lane has id \"{id}\"; add it under \"lanes\", or use one of: {known}")]
    UnknownLane {
        /// Where the step is.
        at: String,
        /// The lane named.
        id: String,
        /// The lane ids there are.
        known: String,
    },
    /// A node listed as a child of two groups.
    #[error(
        "{at}: node \"{node}\" is already inside \"{first_parent}\"; a node sits in one group \
         only, so remove it from one of them"
    )]
    InTwoGroups {
        /// Where the second listing is.
        at: String,
        /// The node.
        node: String,
        /// The group it is in first.
        first_parent: String,
    },
    /// A node listed twice in the same group.
    #[error("{at}: node \"{node}\" is listed twice in group \"{parent}\"; list it once")]
    ListedTwice {
        /// Where the second listing is.
        at: String,
        /// The node.
        node: String,
        /// The group's parent node.
        parent: String,
    },
    /// A structure-map flow step with no connection into it from the previous step.
    #[error(
        "{at}: the step goes from \"{from}\" to \"{to}\", but no connection does; add one under \
         \"connections\" ({{\"from\": \"{from}\", \"to\": \"{to}\", \"label\": ...}}), or \
         change the step"
    )]
    MissingConnection {
        /// Where the step is.
        at: String,
        /// The previous step's node.
        from: String,
        /// This step's node.
        to: String,
    },
    /// Groups that contain each other.
    #[error(
        "groups form a cycle: {}; a node can't end up inside itself, so remove one of these links",
        cycle_text(.path)
    )]
    GroupCycle {
        /// The node ids around the cycle, each containing the next and the last the first.
        path: Vec<String>,
    },
    /// A group with no children.
    #[error("{at}: group \"{parent}\" has no children; list the nodes inside it, or remove it")]
    EmptyGroup {
        /// Where.
        at: String,
        /// Its parent node.
        parent: String,
    },
    /// A flow with no steps.
    #[error("{at}: the flow has no steps; add its steps in order, or remove it")]
    EmptyFlow {
        /// Where.
        at: String,
    },
    /// A field that belongs to the other map type.
    #[error("{at}: {what} only belong on {belongs} maps, and this is a {kind} map; {hint}")]
    WrongMapType {
        /// Where.
        at: String,
        /// What was given.
        what: &'static str,
        /// The type it belongs to.
        belongs: &'static str,
        /// This map's type.
        kind: &'static str,
        /// What to do.
        hint: &'static str,
    },
    /// A swimlane map without exactly one flow.
    #[error(
        "flows: a swimlane map has exactly one flow, its steps in order, and this one has \
         {count}; draw other flows as their own swimlane maps"
    )]
    SwimlaneFlowCount {
        /// How many flows it has.
        count: usize,
    },
    /// A swimlane map with no lanes.
    #[error("lanes: a swimlane map needs lanes, one per participant, top to bottom")]
    NoLanes,
    /// A swimlane step with no lane.
    #[error("{at}: the step has no lane; on a swimlane map every step names one of: {known}")]
    StepWithoutLane {
        /// Where.
        at: String,
        /// The lane ids there are.
        known: String,
    },
    /// A logic node with no pseudocode.
    #[error(
        "{at}: a logic node shows pseudocode, and this one has none; add a few lines of plain \
         pseudocode, or use another kind"
    )]
    LogicWithoutPseudocode {
        /// Where.
        at: String,
    },
    /// A lane linking a node that no structure map of the project has ([`check_lane_links`]).
    #[error(
        "{at}: \"node\" is \"{node}\", but no structure map in this project has a node with \
         that id; draw it on a structure map first, fix the id, or leave \"node\" out"
    )]
    UnknownLaneNode {
        /// Where the lane is.
        at: String,
        /// The node id it names.
        node: String,
    },
    /// A redraw that changes an existing map's type.
    #[error(
        "map: \"{id}\" is a {was} map, and a map keeps its type; to draw a {now} map, leave \
         \"id\" out so it becomes a new map"
    )]
    KindChanged {
        /// The map's id.
        id: String,
        /// Its stored type.
        was: &'static str,
        /// The type asked for.
        now: &'static str,
    },
}

fn kind_list() -> String {
    NodeKind::ALL
        .map(|k| format!("\"{}\"", k.as_str()))
        .join(", ")
}

fn cycle_text(path: &[String]) -> String {
    let mut text = path
        .iter()
        .map(|id| format!("\"{id}\" contains "))
        .collect::<String>();
    if let Some(first) = path.first() {
        text.push_str(&format!("\"{first}\""));
    }
    text
}

fn suggest_id(id: &str) -> String {
    let mut out = String::new();
    for c in id.trim().chars() {
        if is_id_char(c) {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "node-1".to_owned()
    } else {
        out.chars().take(MAX_ID).collect()
    }
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// The text a `maps_draw` call returns when validation fails: every problem, one per line, up to
/// [`limits::MAX_REPORTED_ERRORS`].
pub fn report(errors: &[MapError]) -> String {
    let mut text = format!(
        "The map was not saved: {} problem{} to fix. Fix them and call maps_draw again with the \
         whole map.",
        errors.len(),
        if errors.len() == 1 { "" } else { "s" }
    );
    for error in errors.iter().take(MAX_REPORTED_ERRORS) {
        text.push_str(&format!("\n- {error}"));
    }
    if errors.len() > MAX_REPORTED_ERRORS {
        text.push_str(&format!(
            "\n- …and {} more",
            errors.len() - MAX_REPORTED_ERRORS
        ));
    }
    text
}

/// Checks a `maps_draw` input and builds the map, or returns every problem found.
pub fn validate(input: &DrawMapInput) -> Result<ValidMap, Vec<MapError>> {
    let mut v = Validator::default();

    // The header.
    let requested_id = input.map.id.as_deref().map(|id| {
        v.id("map", id);
        MapId(id.to_owned())
    });
    let title = v.text(
        "map",
        "title",
        &input.map.title,
        MAX_TITLE,
        "give the map a short title",
    );
    let kind = match input.map.kind.trim() {
        "" => {
            v.push(MapError::Missing {
                at: "map".into(),
                field: "type",
                hint: "use \"structure\" or \"swimlane\"",
            });
            None
        }
        value => {
            let kind = MapKind::parse(value);
            if kind.is_none() {
                v.push(MapError::UnknownMapType {
                    value: value.to_owned(),
                });
            }
            kind
        }
    };

    // Sizes.
    if input.nodes.is_empty() {
        v.push(MapError::NoNodes);
    }
    v.count("nodes", input.nodes.len(), MAX_NODES);
    v.count("connections", input.connections.len(), MAX_CONNECTIONS);
    v.count("groups", input.groups.len(), MAX_GROUPS);
    v.count("flows", input.flows.len(), MAX_FLOWS);
    v.count("lanes", input.lanes.len(), MAX_LANES);

    // Nodes.
    let mut nodes = Vec::with_capacity(input.nodes.len());
    let mut node_at: HashMap<&str, String> = HashMap::new();
    for (i, n) in input.nodes.iter().enumerate() {
        let at = loc("nodes", i, &n.id);
        if v.id(&at, &n.id) {
            if let Some(first) = node_at.get(n.id.as_str()) {
                v.push(MapError::Duplicate {
                    at: at.clone(),
                    what: "node id",
                    value: n.id.clone(),
                    first: first.clone(),
                });
            } else {
                node_at.insert(&n.id, at.clone());
            }
        }
        let node_kind = match n.kind.trim() {
            "" => {
                v.push(MapError::Missing {
                    at: at.clone(),
                    field: "kind",
                    hint: "use one of the nine node kinds, or \"concept\"",
                });
                None
            }
            value => {
                let kind = NodeKind::parse(value);
                if kind.is_none() {
                    v.push(MapError::UnknownNodeKind {
                        at: at.clone(),
                        value: value.to_owned(),
                    });
                }
                kind
            }
        };
        let label = v.text(
            &at,
            "label",
            &n.label,
            MAX_LABEL,
            "give the node a short name",
        );
        let description = v.text(
            &at,
            "description",
            &n.description,
            MAX_DESCRIPTION,
            "say in plain English what this node is",
        );
        let file = n.file.as_deref().map(|f| {
            v.text(
                &at,
                "file",
                f,
                MAX_FILE,
                "give the file's path, or leave \"file\" out",
            )
        });
        let pseudocode = n
            .pseudocode
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .map(|p| v.text(&at, "pseudocode", p, MAX_PSEUDOCODE, ""));
        if node_kind == Some(NodeKind::Logic) && pseudocode.is_none() {
            v.push(MapError::LogicWithoutPseudocode { at: at.clone() });
        }
        if let Some(kind) = node_kind {
            nodes.push(Node {
                id: NodeId(n.id.clone()),
                kind,
                label,
                description,
                file,
                pseudocode,
            });
        }
    }
    let known_nodes: HashSet<&str> = node_at.keys().copied().collect();

    // Groups.
    let mut groups = Vec::with_capacity(input.groups.len());
    if kind == Some(MapKind::Swimlane) && !input.groups.is_empty() {
        v.push(MapError::WrongMapType {
            at: "groups".into(),
            what: "groups",
            belongs: MapKind::Structure.as_str(),
            kind: MapKind::Swimlane.as_str(),
            hint: "remove them; swimlanes have no nesting",
        });
    } else {
        let mut parent_at: HashMap<&str, String> = HashMap::new();
        let mut parent_of: HashMap<&str, &str> = HashMap::new();
        for (i, g) in input.groups.iter().enumerate() {
            let at = loc("groups", i, &g.parent);
            v.node_ref(&known_nodes, &at, "parent", &g.parent);
            if !g.parent.is_empty() {
                if let Some(first) = parent_at.get(g.parent.as_str()) {
                    v.push(MapError::Duplicate {
                        at: at.clone(),
                        what: "group parent",
                        value: g.parent.clone(),
                        first: first.clone(),
                    });
                } else {
                    parent_at.insert(&g.parent, at.clone());
                }
            }
            if g.children.is_empty() {
                v.push(MapError::EmptyGroup {
                    at: at.clone(),
                    parent: g.parent.clone(),
                });
            }
            let mut listed: HashSet<&str> = HashSet::new();
            for (j, child) in g.children.iter().enumerate() {
                let child_at = format!("{at}.children[{j}]");
                v.node_ref(&known_nodes, &child_at, "child", child);
                if !listed.insert(child) {
                    v.push(MapError::ListedTwice {
                        at: child_at,
                        node: child.clone(),
                        parent: g.parent.clone(),
                    });
                } else if let Some(first_parent) = parent_of.get(child.as_str()) {
                    v.push(MapError::InTwoGroups {
                        at: child_at,
                        node: child.clone(),
                        first_parent: (*first_parent).to_owned(),
                    });
                } else {
                    parent_of.insert(child, &g.parent);
                }
            }
            groups.push(Group {
                parent: NodeId(g.parent.clone()),
                children: g.children.iter().map(|c| NodeId(c.clone())).collect(),
            });
        }
        for path in cycles(&parent_of) {
            v.push(MapError::GroupCycle { path });
        }
    }

    // Connections.
    let mut connections = Vec::with_capacity(input.connections.len());
    if kind == Some(MapKind::Swimlane) && !input.connections.is_empty() {
        v.push(MapError::WrongMapType {
            at: "connections".into(),
            what: "connections",
            belongs: MapKind::Structure.as_str(),
            kind: MapKind::Swimlane.as_str(),
            hint: "remove them; a swimlane's arrows follow its steps in order, so put their \
                   labels in each step's \"arrow\"",
        });
    } else {
        for (i, c) in input.connections.iter().enumerate() {
            let at = format!("connections[{i}]");
            v.node_ref(&known_nodes, &at, "from", &c.from);
            v.node_ref(&known_nodes, &at, "to", &c.to);
            let label = v.text(
                &at,
                "label",
                &c.label,
                MAX_LABEL,
                "say what the arrow means",
            );
            connections.push(Connection {
                from: NodeId(c.from.clone()),
                to: NodeId(c.to.clone()),
                label,
            });
        }
    }

    // Lanes.
    let mut lanes = Vec::with_capacity(input.lanes.len());
    let mut lane_at: HashMap<&str, String> = HashMap::new();
    if kind == Some(MapKind::Structure) && !input.lanes.is_empty() {
        v.push(MapError::WrongMapType {
            at: "lanes".into(),
            what: "lanes",
            belongs: MapKind::Swimlane.as_str(),
            kind: MapKind::Structure.as_str(),
            hint: "remove them, or make this a swimlane map",
        });
    } else {
        if kind == Some(MapKind::Swimlane) && input.lanes.is_empty() {
            v.push(MapError::NoLanes);
        }
        for (i, l) in input.lanes.iter().enumerate() {
            let at = loc("lanes", i, &l.id);
            if v.id(&at, &l.id) {
                if let Some(first) = lane_at.get(l.id.as_str()) {
                    v.push(MapError::Duplicate {
                        at: at.clone(),
                        what: "lane id",
                        value: l.id.clone(),
                        first: first.clone(),
                    });
                } else {
                    lane_at.insert(&l.id, at.clone());
                }
            }
            let label = v.text(&at, "label", &l.label, MAX_LABEL, "name the participant");
            // A node of a structure map: only its shape can be checked here.
            if let Some(node) = &l.node {
                v.id_field(&at, "node", node);
            }
            lanes.push(Lane {
                id: LaneId(l.id.clone()),
                label,
                node: l.node.as_ref().map(|n| NodeId(n.clone())),
            });
        }
    }
    let known_lanes = input
        .lanes
        .iter()
        .filter(|l| !l.id.is_empty())
        .map(|l| format!("\"{}\"", l.id))
        .collect::<Vec<_>>()
        .join(", ");

    // Flows.
    if kind == Some(MapKind::Swimlane) && input.flows.len() != 1 {
        v.push(MapError::SwimlaneFlowCount {
            count: input.flows.len(),
        });
    }
    let mut flows = Vec::with_capacity(input.flows.len());
    let mut flow_at: HashMap<&str, String> = HashMap::new();
    let mut lane_on_structure_reported = false;
    let mut arrow_on_structure_reported = false;
    for (i, f) in input.flows.iter().enumerate() {
        let at = loc("flows", i, &f.name);
        let name = v.text(&at, "name", &f.name, MAX_LABEL, "name the flow");
        if !name.is_empty() {
            if let Some(first) = flow_at.get(f.name.trim()) {
                v.push(MapError::Duplicate {
                    at: at.clone(),
                    what: "flow name",
                    value: name.clone(),
                    first: first.clone(),
                });
            } else {
                flow_at.insert(f.name.trim(), at.clone());
            }
        }
        if f.steps.is_empty() {
            v.push(MapError::EmptyFlow { at: at.clone() });
        }
        v.count(&format!("flows[{i}].steps"), f.steps.len(), MAX_STEPS);
        let mut steps = Vec::with_capacity(f.steps.len());
        for (j, s) in f.steps.iter().enumerate() {
            let step_at = format!("flows[{i}].steps[{j}]");
            v.node_ref(&known_nodes, &step_at, "node", &s.node);
            let note = v.text(
                &step_at,
                "note",
                &s.note,
                MAX_NOTE,
                "say what happens at this step",
            );
            let mut arrow = s
                .arrow
                .as_deref()
                .filter(|a| !a.trim().is_empty())
                .map(|a| v.text(&step_at, "arrow", a, MAX_LABEL, ""));
            // On structure maps the arrow into a step is the connection from the previous one.
            let mut connection = None;
            if kind == Some(MapKind::Structure) {
                if arrow.take().is_some() && !arrow_on_structure_reported {
                    arrow_on_structure_reported = true;
                    v.push(MapError::WrongMapType {
                        at: step_at.clone(),
                        what: "step arrows",
                        belongs: MapKind::Swimlane.as_str(),
                        kind: MapKind::Structure.as_str(),
                        hint: "remove \"arrow\" from every step; on a structure map a step's \
                               arrow is the connection from the previous step's node",
                    });
                }
                let prev = j.checked_sub(1).map(|p| f.steps[p].node.as_str());
                if let Some(prev) = prev
                    && known_nodes.contains(prev)
                    && known_nodes.contains(s.node.as_str())
                {
                    connection = input
                        .connections
                        .iter()
                        .position(|c| c.from == prev && c.to == s.node);
                    if connection.is_none() {
                        v.push(MapError::MissingConnection {
                            at: step_at.clone(),
                            from: prev.to_owned(),
                            to: s.node.clone(),
                        });
                    }
                }
            }
            match (kind, s.lane.as_deref()) {
                (Some(MapKind::Structure), Some(_)) if !lane_on_structure_reported => {
                    lane_on_structure_reported = true;
                    v.push(MapError::WrongMapType {
                        at: step_at.clone(),
                        what: "step lanes",
                        belongs: MapKind::Swimlane.as_str(),
                        kind: MapKind::Structure.as_str(),
                        hint: "remove \"lane\" from every step, or make this a swimlane map",
                    });
                }
                (Some(MapKind::Swimlane), None) => {
                    v.push(MapError::StepWithoutLane {
                        at: step_at.clone(),
                        known: known_lanes.clone(),
                    });
                }
                (Some(MapKind::Swimlane), Some(lane)) if !lane_at.contains_key(lane) => {
                    v.push(MapError::UnknownLane {
                        at: step_at.clone(),
                        id: lane.to_owned(),
                        known: known_lanes.clone(),
                    });
                }
                _ => {}
            }
            steps.push(FlowStep {
                node: NodeId(s.node.clone()),
                connection,
                arrow,
                note,
                lane: s.lane.as_ref().map(|l| LaneId(l.clone())),
            });
        }
        flows.push(Flow { name, steps });
    }

    match kind {
        Some(kind) if v.errors.is_empty() => Ok(ValidMap {
            requested_id,
            content: MapContent {
                title,
                kind,
                nodes,
                groups,
                connections,
                flows,
                lanes,
            },
        }),
        // Any `None` above pushed an error, so this list is never empty.
        _ => Err(v.errors),
    }
}

/// The cross-map check validation leaves out: every lane's `node` must be a node of one of the
/// project's structure maps, `structure_nodes` (their latest versions). Empty when every link
/// resolves, or the map has none.
pub fn check_lane_links(content: &MapContent, structure_nodes: &HashSet<NodeId>) -> Vec<MapError> {
    content
        .lanes
        .iter()
        .enumerate()
        .filter_map(|(i, lane)| {
            let node = lane.node.as_ref()?;
            (!structure_nodes.contains(node)).then(|| MapError::UnknownLaneNode {
                at: loc("lanes", i, &lane.id.0),
                node: node.0.clone(),
            })
        })
        .collect()
}

/// Where an item is: `nodes[3] ("api")`, or `nodes[3]` when it has no id yet.
fn loc(list: &str, i: usize, id: &str) -> String {
    if id.trim().is_empty() {
        format!("{list}[{i}]")
    } else {
        format!("{list}[{i}] (\"{id}\")")
    }
}

/// Every cycle in a child → parent map, each once, starting at its smallest id.
fn cycles(parent_of: &HashMap<&str, &str>) -> Vec<Vec<String>> {
    let mut found: Vec<Vec<String>> = Vec::new();
    let mut starts: Vec<&str> = parent_of.keys().copied().collect();
    starts.sort_unstable();
    for start in starts {
        // Walk up from `start`; each node has one parent, so the walk either ends or loops.
        let mut path = vec![start];
        let mut current = parent_of.get(start).copied();
        while let Some(node) = current {
            if node == start {
                // `path` runs child → parent; the error reads parent → child.
                let mut cycle: Vec<String> = path.iter().rev().map(|s| (*s).to_owned()).collect();
                let min = cycle
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, s)| *s)
                    .map_or(0, |(i, _)| i);
                cycle.rotate_left(min);
                if !found.contains(&cycle) {
                    found.push(cycle);
                }
                break;
            }
            if path.contains(&node) {
                // A loop that doesn't pass through `start`; found from one of its own nodes.
                break;
            }
            path.push(node);
            current = parent_of.get(node).copied();
        }
    }
    found
}

#[derive(Default)]
struct Validator {
    errors: Vec<MapError>,
}

impl Validator {
    fn push(&mut self, error: MapError) {
        self.errors.push(error);
    }

    fn count(&mut self, list: &str, count: usize, limit: usize) {
        if count > limit {
            self.push(MapError::TooMany {
                list: list.to_owned(),
                count,
                limit,
            });
        }
    }

    /// Checks a required text field; returns it trimmed.
    fn text(
        &mut self,
        at: &str,
        field: &'static str,
        value: &str,
        limit: usize,
        hint: &'static str,
    ) -> String {
        let value = value.trim();
        let len = value.chars().count();
        if len == 0 {
            self.push(MapError::Missing {
                at: at.to_owned(),
                field,
                hint,
            });
        } else if len > limit {
            self.push(MapError::TooLong {
                at: at.to_owned(),
                field,
                len,
                limit,
            });
        }
        value.to_owned()
    }

    /// Checks an `id` field; true when it is well formed.
    fn id(&mut self, at: &str, id: &str) -> bool {
        self.id_field(at, "id", id)
    }

    /// Checks a field holding an id; true when it is well formed.
    fn id_field(&mut self, at: &str, field: &'static str, id: &str) -> bool {
        let len = id.chars().count();
        if id.is_empty() {
            self.push(MapError::Missing {
                at: at.to_owned(),
                field,
                hint: "give it a short id such as \"api\" or \"user-db\"",
            });
            false
        } else if len > MAX_ID {
            self.push(MapError::TooLong {
                at: at.to_owned(),
                field,
                len,
                limit: MAX_ID,
            });
            false
        } else if !id.chars().all(is_id_char) {
            self.push(MapError::BadId {
                at: at.to_owned(),
                id: id.to_owned(),
            });
            false
        } else {
            true
        }
    }

    /// Checks a reference to a node.
    fn node_ref(&mut self, known: &HashSet<&str>, at: &str, field: &'static str, id: &str) {
        if id.is_empty() {
            self.push(MapError::Missing {
                at: at.to_owned(),
                field,
                hint: "give the id of a node under \"nodes\"",
            });
        } else if !known.contains(id) {
            let at = if field == "child" {
                at.to_owned()
            } else {
                format!("{at}.{field}")
            };
            self.push(MapError::UnknownNode {
                at,
                id: id.to_owned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn run(input: Value) -> Result<ValidMap, Vec<MapError>> {
        // The fixtures are hand-written to match the input shape.
        validate(&serde_json::from_value(input).expect("input shape"))
    }

    fn errors(input: Value) -> Vec<MapError> {
        match run(input) {
            Ok(map) => panic!("expected errors, got {map:?}"),
            Err(errors) => errors,
        }
    }

    /// The single error `input` gives.
    fn only_error(input: Value) -> MapError {
        let errors = errors(input);
        assert_eq!(errors.len(), 1, "{errors:#?}");
        errors.into_iter().next().expect("checked above")
    }

    fn structure() -> Value {
        json!({
            "map": { "title": "Checkout", "type": "structure" },
            "nodes": [
                { "id": "shop", "kind": "Part", "label": "Shop service", "description": "Sells things." },
                { "id": "api", "kind": "endpoint", "label": "POST /checkout", "description": "Takes the order." },
                { "id": "handler", "kind": "file", "label": "checkout.rs", "description": "The handler.",
                  "file": "src/checkout.rs" },
                { "id": "rules", "kind": "logic", "label": "Price rules", "description": "Discounts.",
                  "pseudocode": "if member: price *= 0.9" },
                { "id": "orders", "kind": "data", "label": "orders", "description": "Orders table." },
                { "id": "stripe", "kind": "external", "label": "Stripe", "description": "Payments." }
            ],
            "groups": [
                { "parent": "shop", "children": ["api", "orders"] },
                { "parent": "api", "children": ["handler", "rules"] }
            ],
            "connections": [
                { "from": "handler", "to": "orders", "label": "writes" },
                { "from": "handler", "to": "stripe", "label": "charges" },
                { "from": "api", "to": "handler", "label": "calls" }
            ],
            "flows": [
                { "name": "Place an order", "steps": [
                    { "node": "api", "note": "The page posts the cart." },
                    { "node": "handler", "note": "The handler checks it." },
                    { "node": "orders", "note": "The order is saved." }
                ] }
            ]
        })
    }

    fn swimlane() -> Value {
        json!({
            "map": { "id": "login-flow", "title": "Logging in", "type": "swimlane" },
            "nodes": [
                { "id": "user", "kind": "actor", "label": "User", "description": "Someone signing in." },
                { "id": "page", "kind": "endpoint", "label": "/login", "description": "The form." },
                { "id": "db", "kind": "data", "label": "users", "description": "Accounts." },
                { "id": "ok", "kind": "outcome", "label": "Signed in", "description": "Session set." }
            ],
            "lanes": [
                { "id": "you", "label": "User" },
                { "id": "web", "label": "Web", "node": "web-app" },
                { "id": "store", "label": "Database" }
            ],
            "flows": [
                { "name": "Log in", "steps": [
                    { "node": "user", "lane": "you", "note": "Types a password." },
                    { "node": "page", "lane": "web", "arrow": "submits", "note": "Checks the form." },
                    { "node": "db", "lane": "store", "arrow": "looks up", "note": "Finds the account." },
                    { "node": "ok", "lane": "you", "arrow": "returns", "note": "Back to the user." }
                ] }
            ]
        })
    }

    fn wrong_type_fields(errors: &[MapError], belongs_to: &str) -> Vec<&'static str> {
        errors
            .iter()
            .filter_map(|e| match e {
                MapError::WrongMapType { what, belongs, .. } if *belongs == belongs_to => {
                    Some(*what)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_valid_structure_map_builds() {
        let map = run(structure()).expect("valid");
        assert_eq!(map.requested_id, None);
        let c = &map.content;
        assert_eq!(c.kind, MapKind::Structure);
        assert_eq!(c.nodes.len(), 6);
        assert_eq!(c.nodes[0].kind, NodeKind::Part);
        assert_eq!(c.nodes[2].file.as_deref(), Some("src/checkout.rs"));
        assert_eq!(c.groups[1].parent, NodeId("api".into()));
        assert_eq!(c.connections[1].label, "charges");
        // Each step after the first follows the connection from the step before.
        let followed: Vec<_> = c.flows[0].steps.iter().map(|s| s.connection).collect();
        assert_eq!(followed, [None, Some(2), Some(0)]);
        assert!(c.flows[0].steps.iter().all(|s| s.arrow.is_none()));
        assert!(c.lanes.is_empty());
    }

    #[test]
    fn structure_step_without_a_connection_into_it() {
        let mut input = structure();
        // api → handler exists; handler → rules doesn't.
        input["flows"][0]["steps"][2]["node"] = json!("rules");
        assert_eq!(
            only_error(input),
            MapError::MissingConnection {
                at: "flows[0].steps[2]".into(),
                from: "handler".into(),
                to: "rules".into(),
            }
        );
    }

    #[test]
    fn step_arrows_on_a_structure_map() {
        let mut input = structure();
        input["flows"][0]["steps"][1]["arrow"] = json!("calls");
        input["flows"][0]["steps"][2]["arrow"] = json!("writes");
        let errors = errors(input);
        // Reported once, not per step.
        assert_eq!(wrong_type_fields(&errors, "swimlane"), ["step arrows"]);
        assert_eq!(errors.len(), 1, "{errors:#?}");
    }

    #[test]
    fn lane_node_links_are_kept_and_shape_checked() {
        let map = run(swimlane()).expect("valid");
        assert_eq!(map.content.lanes[1].node, Some(NodeId("web-app".into())));
        let mut input = swimlane();
        input["lanes"][1]["node"] = json!("web app");
        assert_eq!(
            only_error(input),
            MapError::BadId {
                at: "lanes[1] (\"web\")".into(),
                id: "web app".into()
            }
        );
    }

    #[test]
    fn lane_links_are_checked_against_structure_nodes() {
        let map = run(swimlane()).expect("valid").content;
        let known: HashSet<NodeId> = [NodeId("web-app".into())].into();
        assert!(check_lane_links(&map, &known).is_empty());
        assert_eq!(
            check_lane_links(&map, &HashSet::new()),
            [MapError::UnknownLaneNode {
                at: "lanes[1] (\"web\")".into(),
                node: "web-app".into(),
            }]
        );
        // A structure map has no lanes, so nothing to check.
        let structure = run(structure()).expect("valid").content;
        assert!(check_lane_links(&structure, &HashSet::new()).is_empty());
    }

    #[test]
    fn a_child_listed_twice_in_one_group() {
        let mut input = structure();
        input["groups"][1]["children"] = json!(["handler", "rules", "handler"]);
        assert_eq!(
            only_error(input),
            MapError::ListedTwice {
                at: "groups[1] (\"api\").children[2]".into(),
                node: "handler".into(),
                parent: "api".into(),
            }
        );
    }

    #[test]
    fn a_valid_swimlane_map_builds() {
        let map = run(swimlane()).expect("valid");
        assert_eq!(map.requested_id, Some(MapId("login-flow".into())));
        let c = &map.content;
        assert_eq!(c.kind, MapKind::Swimlane);
        assert_eq!(c.lanes.len(), 3);
        assert_eq!(c.flows[0].steps[3].lane, Some(LaneId("you".into())));
    }

    #[test]
    fn a_stored_map_round_trips_through_json() {
        let map = run(structure())
            .expect("valid")
            .into_map(MapId("m1".into()), 2);
        let json = serde_json::to_string(&map).expect("serialises");
        assert_eq!(serde_json::from_str::<Map>(&json).expect("parses"), map);
    }

    #[test]
    fn no_nodes() {
        let mut input = structure();
        input["nodes"] = json!([]);
        input["groups"] = json!([]);
        input["connections"] = json!([]);
        input["flows"] = json!([]);
        assert_eq!(only_error(input), MapError::NoNodes);
    }

    #[test]
    fn too_many_nodes() {
        let mut input = structure();
        let nodes: Vec<Value> = (0..=MAX_NODES)
            .map(|i| json!({ "id": format!("n{i}"), "kind": "concept", "label": "x", "description": "x" }))
            .collect();
        input["nodes"] = json!(nodes);
        input["groups"] = json!([]);
        input["connections"] = json!([]);
        input["flows"] = json!([]);
        assert_eq!(
            only_error(input),
            MapError::TooMany {
                list: "nodes".into(),
                count: MAX_NODES + 1,
                limit: MAX_NODES
            }
        );
    }

    #[test]
    fn missing_field() {
        let mut input = structure();
        input["map"]["title"] = json!("  ");
        assert!(matches!(
            only_error(input),
            MapError::Missing { ref at, field: "title", .. } if at == "map"
        ));
    }

    #[test]
    fn text_too_long() {
        let mut input = structure();
        input["nodes"][1]["label"] = json!("x".repeat(MAX_LABEL + 1));
        assert_eq!(
            only_error(input),
            MapError::TooLong {
                at: "nodes[1] (\"api\")".into(),
                field: "label",
                len: MAX_LABEL + 1,
                limit: MAX_LABEL
            }
        );
    }

    #[test]
    fn bad_id() {
        let mut input = swimlane();
        input["map"]["id"] = json!("Login Flow!");
        let error = only_error(input);
        assert_eq!(
            error,
            MapError::BadId {
                at: "map".into(),
                id: "Login Flow!".into()
            }
        );
        assert!(error.to_string().contains("\"login-flow\""), "{error}");
    }

    #[test]
    fn unknown_map_type() {
        let mut input = structure();
        input["map"]["type"] = json!("sequence");
        assert_eq!(
            only_error(input),
            MapError::UnknownMapType {
                value: "sequence".into()
            }
        );
    }

    #[test]
    fn unknown_node_kind() {
        let mut input = structure();
        input["nodes"][5]["kind"] = json!("service");
        let error = only_error(input);
        assert_eq!(
            error,
            MapError::UnknownNodeKind {
                at: "nodes[5] (\"stripe\")".into(),
                value: "service".into()
            }
        );
        assert!(error.to_string().contains("\"part\", \"file\""), "{error}");
    }

    #[test]
    fn duplicate_node_id() {
        let mut input = structure();
        let copy = input["nodes"][1].clone();
        input["nodes"]
            .as_array_mut()
            .expect("nodes is an array")
            .push(copy);
        assert_eq!(
            only_error(input),
            MapError::Duplicate {
                at: "nodes[6] (\"api\")".into(),
                what: "node id",
                value: "api".into(),
                first: "nodes[1] (\"api\")".into(),
            }
        );
    }

    #[test]
    fn connection_to_a_missing_node() {
        let mut input = structure();
        input["connections"][1]["to"] = json!("strip");
        assert_eq!(
            only_error(input),
            MapError::UnknownNode {
                at: "connections[1].to".into(),
                id: "strip".into()
            }
        );
    }

    #[test]
    fn group_with_a_missing_parent() {
        let mut input = structure();
        input["groups"][0]["parent"] = json!("store");
        assert_eq!(
            only_error(input),
            MapError::UnknownNode {
                at: "groups[0] (\"store\").parent".into(),
                id: "store".into()
            }
        );
    }

    #[test]
    fn node_in_two_groups() {
        let mut input = structure();
        input["groups"][1]["children"] = json!(["handler", "rules", "orders"]);
        assert_eq!(
            only_error(input),
            MapError::InTwoGroups {
                at: "groups[1] (\"api\").children[2]".into(),
                node: "orders".into(),
                first_parent: "shop".into(),
            }
        );
    }

    #[test]
    fn group_cycle() {
        let mut input = structure();
        // shop contains api, api contains handler, handler contains shop.
        input["groups"] = json!([
            { "parent": "shop", "children": ["api"] },
            { "parent": "api", "children": ["handler"] },
            { "parent": "handler", "children": ["shop"] }
        ]);
        let error = only_error(input);
        assert_eq!(
            error,
            MapError::GroupCycle {
                path: vec!["api".into(), "handler".into(), "shop".into()]
            }
        );
        assert!(
            error
                .to_string()
                .contains("\"api\" contains \"handler\" contains \"shop\" contains \"api\""),
            "{error}"
        );
    }

    #[test]
    fn a_node_inside_itself_is_a_cycle() {
        let mut input = structure();
        input["groups"] = json!([{ "parent": "shop", "children": ["shop"] }]);
        assert_eq!(
            only_error(input),
            MapError::GroupCycle {
                path: vec!["shop".into()]
            }
        );
    }

    #[test]
    fn empty_group() {
        let mut input = structure();
        input["groups"][1]["children"] = json!([]);
        assert_eq!(
            only_error(input),
            MapError::EmptyGroup {
                at: "groups[1] (\"api\")".into(),
                parent: "api".into()
            }
        );
    }

    #[test]
    fn empty_flow() {
        let mut input = structure();
        input["flows"][0]["steps"] = json!([]);
        assert_eq!(
            only_error(input),
            MapError::EmptyFlow {
                at: "flows[0] (\"Place an order\")".into()
            }
        );
    }

    #[test]
    fn flow_step_on_an_unknown_node() {
        let mut input = structure();
        input["flows"][0]["steps"][1]["node"] = json!("ghost");
        assert_eq!(
            only_error(input),
            MapError::UnknownNode {
                at: "flows[0].steps[1].node".into(),
                id: "ghost".into()
            }
        );
    }

    #[test]
    fn step_in_an_unknown_lane() {
        let mut input = swimlane();
        input["flows"][0]["steps"][2]["lane"] = json!("db");
        assert_eq!(
            only_error(input),
            MapError::UnknownLane {
                at: "flows[0].steps[2]".into(),
                id: "db".into(),
                known: "\"you\", \"web\", \"store\"".into(),
            }
        );
    }

    #[test]
    fn swimlane_step_without_a_lane() {
        let mut input = swimlane();
        input["flows"][0]["steps"][0]
            .as_object_mut()
            .expect("the step is an object")
            .remove("lane");
        assert!(matches!(
            only_error(input),
            MapError::StepWithoutLane { ref at, .. } if at == "flows[0].steps[0]"
        ));
    }

    #[test]
    fn swimlane_with_two_flows() {
        let mut input = swimlane();
        let flow = input["flows"][0].clone();
        let mut second = flow.clone();
        second["name"] = json!("Log in again");
        input["flows"] = json!([flow, second]);
        assert_eq!(only_error(input), MapError::SwimlaneFlowCount { count: 2 });
    }

    #[test]
    fn swimlane_without_lanes() {
        let mut input = swimlane();
        input["lanes"] = json!([]);
        let errors = errors(input);
        assert_eq!(errors[0], MapError::NoLanes);
        // Each of the four steps then names a lane that isn't there.
        assert_eq!(errors.len(), 5, "{errors:#?}");
    }

    #[test]
    fn structure_only_fields_on_a_swimlane() {
        let mut input = swimlane();
        input["groups"] = json!([{ "parent": "page", "children": ["db"] }]);
        input["connections"] = json!([{ "from": "page", "to": "db", "label": "reads" }]);
        let errors = errors(input);
        assert_eq!(
            wrong_type_fields(&errors, "structure"),
            ["groups", "connections"]
        );
        assert_eq!(errors.len(), 2, "{errors:#?}");
    }

    #[test]
    fn swimlane_only_fields_on_a_structure_map() {
        let mut input = structure();
        input["lanes"] = json!([{ "id": "web", "label": "Web" }]);
        input["flows"][0]["steps"][0]["lane"] = json!("web");
        input["flows"][0]["steps"][1]["lane"] = json!("web");
        let errors = errors(input);
        // Step lanes are reported once, not per step.
        assert_eq!(
            wrong_type_fields(&errors, "swimlane"),
            ["lanes", "step lanes"]
        );
        assert_eq!(errors.len(), 2, "{errors:#?}");
    }

    #[test]
    fn logic_without_pseudocode() {
        let mut input = structure();
        input["nodes"][3]["pseudocode"] = json!(" ");
        assert_eq!(
            only_error(input),
            MapError::LogicWithoutPseudocode {
                at: "nodes[3] (\"rules\")".into()
            }
        );
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let mut input = structure();
        input["map"]["type"] = json!("");
        input["nodes"][0]["kind"] = json!("module");
        input["connections"][1]["from"] = json!("nope");
        input["flows"][0]["steps"][0]["note"] = json!("");
        assert_eq!(errors(input).len(), 4);
    }

    #[test]
    fn misspelt_fields_are_refused_not_dropped() {
        let mut input = structure();
        input["nodes"][3]["pseudo_code"] = json!("x");
        assert!(serde_json::from_value::<DrawMapInput>(input).is_err());
    }

    #[test]
    fn the_report_lists_problems_up_to_the_cap() {
        let errors = vec![MapError::NoLanes; MAX_REPORTED_ERRORS + 3];
        let text = report(&errors);
        let head = format!("The map was not saved: {} problems", errors.len());
        assert!(text.starts_with(&head), "{text}");
        assert_eq!(text.lines().count(), MAX_REPORTED_ERRORS + 2);
        assert!(text.ends_with("…and 3 more"), "{text}");
    }
}
