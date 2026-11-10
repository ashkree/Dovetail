//! The `draw_map` tool's JSON schema, as `tools/list` shows it (design 23i).
//!
//! Written by hand to carry the descriptions an agent reads, and kept in step with
//! [`crate::DrawMapInput`] by a test that compares every object's fields with the input types'.
//! Kinds, types and limits come from the same constants validation uses.

use serde_json::{Value, json};

use crate::limits::*;
use dovetail_api::map::{MapKind, NodeKind};

/// The JSON schema of [`crate::DrawMapInput`].
pub fn input_schema() -> Value {
    let node_kinds: Vec<&str> = NodeKind::ALL.iter().map(|k| k.as_str()).collect();
    let map_kinds: Vec<&str> = MapKind::ALL.iter().map(|k| k.as_str()).collect();
    let id = |what: &str| {
        json!({
            "type": "string",
            "maxLength": MAX_ID,
            "description": format!("{what}: letters, digits, \"_\" and \"-\" only."),
        })
    };
    json!({
        "type": "object",
        "properties": {
            "map": {
                "type": "object",
                "description": "What the map is.",
                "properties": {
                    "id": {
                        "type": "string",
                        "maxLength": MAX_ID,
                        "description": "Omit for a new map; Dovetail gives it an id. Give an \
                            existing map's id to redraw it as a new version (it keeps its type).",
                    },
                    "title": {"type": "string", "maxLength": MAX_TITLE, "description": "Short title for the map shelf."},
                    "type": {
                        "type": "string",
                        "enum": map_kinds,
                        "description": "`structure`: what exists and how it connects (nodes, \
                            groups, connections, guided flows). `swimlane`: what happens in order \
                            (lanes and exactly one flow whose steps name their lane).",
                    },
                },
                "required": ["title", "type"],
            },
            "nodes": {
                "type": "array",
                "maxItems": MAX_NODES,
                "description": "Every node on the map; groups, connections and steps refer to them by id.",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": id("Unique within the map"),
                        "kind": {
                            "type": "string",
                            "enum": node_kinds,
                            "description": "part: a service, package or module (drills in); \
                                file: a source file; endpoint: a route, screen or CLI command; \
                                data: a table, cache, queue or config; external: another system; \
                                actor: a user or role; outcome: a result; logic: a few lines of \
                                pseudocode; concept: anything else.",
                        },
                        "label": {"type": "string", "maxLength": MAX_LABEL, "description": "Short name drawn in the box."},
                        "description": {"type": "string", "maxLength": MAX_DESCRIPTION, "description": "Plain-English description of what this is."},
                        "file": {"type": "string", "maxLength": MAX_FILE, "description": "Path of the file it stands for, if any."},
                        "pseudocode": {"type": "string", "maxLength": MAX_PSEUDOCODE, "description": "A few lines of plain pseudocode; required for `logic` nodes."},
                    },
                    "required": ["id", "kind", "label", "description"],
                },
            },
            "groups": {
                "type": "array",
                "maxItems": MAX_GROUPS,
                "description": "Structure maps only: which nodes sit inside which. A node sits in one group at most.",
                "items": {
                    "type": "object",
                    "properties": {
                        "parent": {"type": "string", "description": "Id of the node you drill into."},
                        "children": {"type": "array", "items": {"type": "string"}, "description": "Ids of the nodes inside it."},
                    },
                    "required": ["parent", "children"],
                },
            },
            "connections": {
                "type": "array",
                "maxItems": MAX_CONNECTIONS,
                "description": "Structure maps only: labelled arrows between nodes.",
                "items": {
                    "type": "object",
                    "properties": {
                        "from": {"type": "string", "description": "Source node id."},
                        "to": {"type": "string", "description": "Target node id."},
                        "label": {"type": "string", "maxLength": MAX_LABEL, "description": "What it means: \"calls\", \"reads from\"."},
                    },
                    "required": ["from", "to", "label"],
                },
            },
            "flows": {
                "type": "array",
                "maxItems": MAX_FLOWS,
                "description": "Guided walks through the map, one step at a time. A swimlane map has exactly one.",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "maxLength": MAX_LABEL, "description": "Unique within the map."},
                        "steps": {
                            "type": "array",
                            "maxItems": MAX_STEPS,
                            "description": "In order. On a structure map, a connection must lead from each step's node to the next.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "node": {"type": "string", "description": "The node this step is at."},
                                    "arrow": {"type": "string", "maxLength": MAX_LABEL, "description": "Swimlane maps only: label of the arrow into this step (\"submits\")."},
                                    "note": {"type": "string", "maxLength": MAX_NOTE, "description": "What happens at this step."},
                                    "lane": {"type": "string", "description": "Swimlane maps only: the lane this step sits in."},
                                },
                                "required": ["node", "note"],
                            },
                        },
                    },
                    "required": ["name", "steps"],
                },
            },
            "lanes": {
                "type": "array",
                "maxItems": MAX_LANES,
                "description": "Swimlane maps only: the participants, top to bottom.",
                "items": {
                    "type": "object",
                    "properties": {
                        "id": id("Unique within the map; steps name it"),
                        "label": {"type": "string", "maxLength": MAX_LABEL, "description": "Shown at the left of the band."},
                        "node": {"type": "string", "maxLength": MAX_ID, "description": "Id of this participant's node on one of the project's structure maps, to jump between the two views. That node must exist."},
                    },
                    "required": ["id", "label"],
                },
            },
        },
        "required": ["map", "nodes"],
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::input::*;

    /// An input with every optional field set and every list non-empty, so its JSON shows every
    /// field the input types have.
    fn full_input() -> DrawMapInput {
        DrawMapInput {
            map: MapHeaderInput {
                id: Some("m".into()),
                title: "t".into(),
                kind: "structure".into(),
            },
            nodes: vec![NodeInput {
                id: "n".into(),
                kind: "logic".into(),
                label: "l".into(),
                description: "d".into(),
                file: Some("f".into()),
                pseudocode: Some("p".into()),
            }],
            groups: vec![GroupInput {
                parent: "n".into(),
                children: vec!["n".into()],
            }],
            connections: vec![ConnectionInput {
                from: "n".into(),
                to: "n".into(),
                label: "l".into(),
            }],
            flows: vec![FlowInput {
                name: "f".into(),
                steps: vec![StepInput {
                    node: "n".into(),
                    arrow: Some("a".into()),
                    note: "n".into(),
                    lane: Some("l".into()),
                }],
            }],
            lanes: vec![LaneInput {
                id: "l".into(),
                label: "l".into(),
                node: Some("n".into()),
            }],
        }
    }

    /// Walks the schema and the serialised input side by side, collecting the paths where an
    /// object's fields differ.
    fn compare(schema: &Value, value: &Value, path: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(fields) => {
                let props = schema["properties"]
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                let in_schema: BTreeSet<&String> = props.keys().collect();
                let in_input: BTreeSet<&String> = fields.keys().collect();
                if in_schema != in_input {
                    out.push(format!("{path}: schema {in_schema:?}, input {in_input:?}"));
                }
                for (key, v) in fields {
                    compare(
                        &props.get(key).cloned().unwrap_or(Value::Null),
                        v,
                        &format!("{path}.{key}"),
                        out,
                    );
                }
                for required in schema["required"].as_array().into_iter().flatten() {
                    let name = required.as_str().unwrap_or_default();
                    if !fields.contains_key(name) {
                        out.push(format!("{path}: required `{name}` is not an input field"));
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    compare(&schema["items"], item, &format!("{path}[]"), out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn the_schema_matches_the_input_types() {
        let input = serde_json::to_value(full_input()).expect("serialises");
        let mut problems = Vec::new();
        compare(&input_schema(), &input, "", &mut problems);
        assert!(problems.is_empty(), "{problems:#?}");
    }

    #[test]
    fn enums_come_from_the_model() {
        let schema = input_schema();
        let kinds = schema.pointer("/properties/nodes/items/properties/kind/enum");
        assert_eq!(
            kinds.and_then(Value::as_array).map(Vec::len),
            Some(NodeKind::ALL.len())
        );
        let types = schema.pointer("/properties/map/properties/type/enum");
        assert_eq!(types, Some(&json!(["structure", "swimlane"])));
    }
}
