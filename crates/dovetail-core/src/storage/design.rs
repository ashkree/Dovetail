//! The variants slices declared and the user's picks (ADR 0026), in `state.db`.

use std::collections::BTreeMap;

use dovetail_api::agents::agent::AgentId;
use dovetail_api::visual::design::{VariantSet, VariantSlot};
use rusqlite::{Connection, OptionalExtension, params};

use super::{Db, DbError, Pending};

impl Db {
    /// Every declared variant set of the open project.
    pub fn load_variants(&self) -> Pending<Vec<VariantSet>> {
        self.run(|c, _| load_all(c.state()?))
    }

    /// Replaces a slice's declared list. A pick that is still a valid option of a slot of the
    /// same name stays. An empty list removes the row. Resolves to the stored set, or `None`
    /// when the list is empty.
    pub fn declare_variants(
        &self,
        agent: AgentId,
        slots: Vec<VariantSlot>,
    ) -> Pending<Option<VariantSet>> {
        self.run(move |c, _| declare(c.state()?, agent, slots))
    }

    /// Records the user's pick of `option` in `slot`; refused for an unknown slice, slot or
    /// option.
    pub fn select_variant(
        &self,
        agent: AgentId,
        slot: String,
        option: String,
    ) -> Pending<VariantSet> {
        self.run(move |c, _| select(c.state()?, agent, &slot, &option))
    }

    /// Marks every slot's current pick as its kept option and returns the set as it was
    /// before (to build the prompt from) and after.
    pub fn keep_variants(&self, agent: AgentId) -> Pending<(VariantSet, VariantSet)> {
        self.run(move |c, _| keep(c.state()?, agent))
    }

    /// Forgets a slice's variants (the slice is gone).
    pub fn forget_variants(&self, agent: AgentId) -> Pending<()> {
        self.run(move |c, _| {
            c.state()?.execute(
                "DELETE FROM design_variants WHERE agent_id = ?1",
                params![agent.0],
            )?;
            Ok(())
        })
    }
}

fn parse(agent: String, slots: &str, selection: &str) -> Result<VariantSet, DbError> {
    let corrupt = |what: &str, e: serde_json::Error| DbError::Corrupt(format!("{what}: {e}"));
    Ok(VariantSet {
        agent: AgentId(agent),
        slots: serde_json::from_str(slots).map_err(|e| corrupt("variant slots", e))?,
        selection: serde_json::from_str(selection).map_err(|e| corrupt("variant selection", e))?,
    })
}

fn load_all(conn: &Connection) -> Result<Vec<VariantSet>, DbError> {
    let mut stmt =
        conn.prepare("SELECT agent_id, slots, selection FROM design_variants ORDER BY agent_id")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (agent, slots, selection) = row?;
        out.push(parse(agent, &slots, &selection)?);
    }
    Ok(out)
}

fn load_one(conn: &Connection, agent: &AgentId) -> Result<Option<VariantSet>, DbError> {
    let row = conn
        .query_row(
            "SELECT slots, selection FROM design_variants WHERE agent_id = ?1",
            params![agent.0],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?;
    row.map(|(slots, selection)| parse(agent.0.clone(), &slots, &selection))
        .transpose()
}

fn store(conn: &Connection, set: &VariantSet) -> Result<(), DbError> {
    let text = |e: serde_json::Error| DbError::Corrupt(format!("variants: {e}"));
    conn.execute(
        "INSERT INTO design_variants (agent_id, slots, selection) VALUES (?1, ?2, ?3)
         ON CONFLICT(agent_id) DO UPDATE SET slots = ?2, selection = ?3",
        params![
            set.agent.0,
            serde_json::to_string(&set.slots).map_err(text)?,
            serde_json::to_string(&set.selection).map_err(text)?
        ],
    )?;
    Ok(())
}

fn declare(
    conn: &Connection,
    agent: AgentId,
    slots: Vec<VariantSlot>,
) -> Result<Option<VariantSet>, DbError> {
    if slots.is_empty() {
        conn.execute(
            "DELETE FROM design_variants WHERE agent_id = ?1",
            params![agent.0],
        )?;
        return Ok(None);
    }
    let previous: BTreeMap<String, String> = load_one(conn, &agent)?
        .map(|s| s.selection)
        .unwrap_or_default();
    let set = VariantSet::declared(agent, slots, &previous);
    store(conn, &set)?;
    Ok(Some(set))
}

fn select(
    conn: &Connection,
    agent: AgentId,
    slot: &str,
    option: &str,
) -> Result<VariantSet, DbError> {
    let Some(mut set) = load_one(conn, &agent)? else {
        return Err(DbError::Refused(format!(
            "{} has declared no variants",
            agent.0
        )));
    };
    let Some(found) = set.slots.iter().find(|s| s.name == slot) else {
        return Err(DbError::Refused(format!(
            "there is no variant slot `{slot}`"
        )));
    };
    if !found.options.iter().any(|o| o == option) {
        return Err(DbError::Refused(format!(
            "`{option}` is not an option of `{slot}`"
        )));
    }
    set.selection.insert(slot.to_owned(), option.to_owned());
    store(conn, &set)?;
    Ok(set)
}

fn keep(conn: &Connection, agent: AgentId) -> Result<(VariantSet, VariantSet), DbError> {
    let Some(before) = load_one(conn, &agent)? else {
        return Err(DbError::Refused(format!(
            "{} has declared no variants",
            agent.0
        )));
    };
    let mut after = before.clone();
    for slot in &mut after.slots {
        slot.kept = before.selection.get(&slot.name).cloned();
    }
    store(conn, &after)?;
    Ok((before, after))
}
