//! Queries on a project's `state.db`: maps and their versions (M15, design 23h).

use std::collections::HashSet;

use dovetail_api::agents::agent::AgentId;
use dovetail_api::tracking::task::TaskId;
use dovetail_api::visual::map::{Map, MapContent, MapId, MapKind, MapSummary, NodeId};
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::projects::slug;
use crate::storage::DbError;

/// Longest id Dovetail makes from a title; agents' own ids follow the validation limit.
const MAX_MADE_ID: usize = 48;

/// A validated map to store as a new map or a new version.
#[derive(Debug, Clone)]
pub struct NewMapVersion {
    /// The id the agent gave: an existing map to redraw, or the id for a new one. `None`: make
    /// one from the title.
    pub id: Option<MapId>,
    /// The validated content.
    pub content: MapContent,
    /// The drawing agent's task; recorded on a new map only.
    pub task: Option<TaskId>,
    /// The drawing agent.
    pub drawn_by: AgentId,
}

/// What storing a map did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drawn {
    /// Stored: the map's shelf entry now (version 1 for a new map).
    Saved(MapSummary),
    /// Nothing stored: the id names a map of the other type.
    KindChanged {
        /// The map.
        id: MapId,
        /// Its stored type.
        was: MapKind,
    },
}

const SUMMARY_COLUMNS: &str =
    "id, title, type, task_id, drawn_by, current_version, created_ms, updated_ms";

fn summary(row: &Row<'_>) -> rusqlite::Result<MapSummary> {
    let kind: String = row.get(2)?;
    let kind = MapKind::parse(&kind).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            2,
            Type::Text,
            format!("map type {kind:?}").into(),
        )
    })?;
    Ok(MapSummary {
        id: MapId(row.get(0)?),
        title: row.get(1)?,
        kind,
        task: row.get::<_, Option<i64>>(3)?.map(TaskId),
        drawn_by: row.get::<_, Option<String>>(4)?.map(AgentId),
        version: row.get(5)?,
        created_ms: row.get(6)?,
        updated_ms: row.get(7)?,
    })
}

/// Every map's shelf entry, most recently drawn first.
pub(crate) fn load_maps(conn: &Connection) -> Result<Vec<MapSummary>, DbError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SUMMARY_COLUMNS} FROM maps ORDER BY updated_ms DESC, id"
    ))?;
    let rows = stmt.query_map([], summary)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn load_summary(conn: &Connection, id: &MapId) -> Result<Option<MapSummary>, DbError> {
    Ok(conn
        .query_row(
            &format!("SELECT {SUMMARY_COLUMNS} FROM maps WHERE id = ?1"),
            [&id.0],
            summary,
        )
        .optional()?)
}

/// One version of a map (`None`: the latest), or `None` when there is no such map or version.
pub(crate) fn load_map(
    conn: &Connection,
    id: &MapId,
    version: Option<u32>,
) -> Result<Option<Map>, DbError> {
    let row: Option<(u32, String)> = conn
        .query_row(
            "SELECT v.version, v.model FROM map_versions v JOIN maps m ON m.id = v.map_id \
             WHERE v.map_id = ?1 AND v.version = COALESCE(?2, m.current_version)",
            params![id.0, version],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(version, model)| {
        let content: MapContent = serde_json::from_str(&model)
            .map_err(|e| DbError::Corrupt(format!("map {} version {version}: {e}", id.0)))?;
        Ok(Map {
            id: id.clone(),
            version,
            content,
        })
    })
    .transpose()
}

/// Ids of every node on the latest version of each structure map: what a lane may link.
pub(crate) fn structure_nodes(conn: &Connection) -> Result<HashSet<NodeId>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT json_extract(n.value, '$.id') \
         FROM maps m JOIN map_versions v ON v.map_id = m.id AND v.version = m.current_version, \
         json_each(v.model, '$.nodes') n WHERE m.type = ?1",
    )?;
    let rows = stmt.query_map([MapKind::Structure.as_str()], |r| r.get::<_, String>(0))?;
    rows.map(|r| Ok(NodeId(r?))).collect()
}

/// Stores a new map, or a new version of the map the id names.
pub(crate) fn save_map(
    conn: &mut Connection,
    new: NewMapVersion,
    now: i64,
) -> Result<Drawn, DbError> {
    let tx = conn.transaction()?;
    let existing = match &new.id {
        Some(id) => load_summary(&tx, id)?,
        None => None,
    };
    let model = serde_json::to_string(&new.content)
        .map_err(|e| DbError::Corrupt(format!("map model: {e}")))?;
    let kind = new.content.kind.as_str();
    let (id, version) = match existing {
        Some(old) if old.kind != new.content.kind => {
            return Ok(Drawn::KindChanged {
                id: old.id,
                was: old.kind,
            });
        }
        Some(old) => {
            let version = old.version + 1;
            tx.execute(
                "UPDATE maps SET title = ?2, current_version = ?3, drawn_by = ?4, updated_ms = ?5 \
                 WHERE id = ?1",
                params![old.id.0, new.content.title, version, new.drawn_by.0, now],
            )?;
            (old.id, version)
        }
        None => {
            let id = match new.id {
                Some(id) => id,
                None => fresh_id(&tx, &new.content.title)?,
            };
            tx.execute(
                "INSERT INTO maps (id, task_id, title, type, current_version, drawn_by, \
                 created_ms, updated_ms) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?6)",
                params![
                    id.0,
                    new.task.map(|t| t.0),
                    new.content.title,
                    kind,
                    new.drawn_by.0,
                    now
                ],
            )?;
            (id, 1)
        }
    };
    tx.execute(
        "INSERT INTO map_versions (map_id, version, model, drawn_by, created_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id.0, version, model, new.drawn_by.0, now],
    )?;
    let saved = load_summary(&tx, &id)?
        .ok_or_else(|| DbError::NotFound(format!("map {} just written", id.0)))?;
    tx.commit()?;
    Ok(Drawn::Saved(saved))
}

/// An unused id made from `title`: its slug, numbered (`-2`, ...) when taken.
fn fresh_id(conn: &Connection, title: &str) -> Result<MapId, DbError> {
    let base = if title.chars().any(|c| c.is_ascii_alphanumeric()) {
        let mut s = slug(title);
        s.truncate(MAX_MADE_ID);
        s.trim_end_matches('-').to_owned()
    } else {
        "map".to_owned()
    };
    let taken = |id: &str| -> Result<bool, DbError> {
        Ok(conn
            .query_row("SELECT 1 FROM maps WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some())
    };
    let mut candidate = base.clone();
    let mut n = 1;
    while taken(&candidate)? {
        n += 1;
        candidate = format!("{base}-{n}");
    }
    Ok(MapId(candidate))
}
