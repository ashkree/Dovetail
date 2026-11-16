//! Canvases, cards and links in `state.db` (ADR 0025).
//!
//! A card that points at something keeps no text of its own: its title is read from the target
//! when the card is read, and its body only on request ([`Db::load_card_body`]). The target is
//! checked to exist inside the same request that adds the card. A target that is gone anyway
//! (a damaged database) makes the card read as "missing"; it never fails a load.

use dovetail_api::{
    Canvas, CanvasId, Card, CardId, CardKind, CardLink, CardRef, EffortId, Framework, NewCard,
    RefKind, TaskId, UNSORTED_EFFORT, card_title,
};
use dovetail_api::research::FindingId;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use super::sessions::{from_text, to_text};
use super::{Db, DbError, Pending};

/// What a card reads as when its target is gone.
pub const MISSING_TITLE: &str = "(missing)";

/// Longest body [`Db::load_card_body`] sends: a source's page can be long.
const BODY_MAX: usize = 20_000;

/// Everything canvas-shaped in a project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CanvasState {
    /// Canvases, in id order.
    pub canvases: Vec<Canvas>,
    /// Cards, in id order.
    pub cards: Vec<Card>,
    /// Links.
    pub links: Vec<CardLink>,
}

impl Db {
    /// Every canvas, card and link of the open project.
    pub fn load_canvas_state(&self) -> Pending<CanvasState> {
        self.run(|c, _| load_state(c.state()?))
    }

    /// Creates a canvas for an effort. An empty title becomes the effort's name.
    pub fn create_canvas(
        &self,
        effort: EffortId,
        title: String,
        framework: Framework,
    ) -> Pending<Canvas> {
        self.run(move |c, now| create_canvas(c.state()?, effort, &title, framework, now))
    }

    /// Renames a canvas.
    pub fn rename_canvas(&self, id: CanvasId, title: String) -> Pending<Canvas> {
        self.run(move |c, _| rename_canvas(c.state()?, id, &title))
    }

    /// Adds a card, checking in this request that its effort, canvas and target exist, and links
    /// it from `link_from` when given. Resolves to the card and the link.
    pub fn add_card(
        &self,
        card: NewCard,
        link_from: Option<CardId>,
        added_by: String,
    ) -> Pending<(Card, Option<CardLink>)> {
        self.run(move |c, now| add_card_linked(c.state()?, card, link_from, &added_by, now))
    }

    /// Places an incoming card on a canvas of its effort.
    pub fn place_card(&self, id: CardId, canvas: CanvasId, x: i64, y: i64) -> Pending<Card> {
        self.run(move |c, _| place_card(c.state()?, id, canvas, x, y))
    }

    /// Moves a placed card.
    pub fn move_card(&self, id: CardId, x: i64, y: i64) -> Pending<Card> {
        self.run(move |c, _| move_card(c.state()?, id, x, y))
    }

    /// Deletes a card and its links. With `incoming_only`, refuses a placed card.
    pub fn delete_card(&self, id: CardId, incoming_only: bool) -> Pending<()> {
        self.run(move |c, _| delete_card(c.state()?, id, incoming_only))
    }

    /// Links two cards of one effort; linking them again changes the label.
    pub fn link_cards(&self, from: CardId, to: CardId, label: String) -> Pending<CardLink> {
        self.run(move |c, _| link_cards(c.state()?, from, to, &label))
    }

    /// Removes a link.
    pub fn unlink_cards(&self, from: CardId, to: CardId) -> Pending<()> {
        self.run(move |c, _| unlink_cards(c.state()?, from, to))
    }

    /// A card's body: its own words, or the target's text.
    pub fn load_card_body(&self, id: CardId) -> Pending<String> {
        self.run(move |c, _| card_body(c.state()?, id))
    }

    /// Puts one finding card in the incoming strip of the effort `task` belongs to. Resolves to
    /// the card.
    pub fn add_finding_card(
        &self,
        finding: FindingId,
        task: TaskId,
        added_by: String,
    ) -> Pending<Card> {
        self.run(move |c, now| add_finding_card(c.state()?, finding, task, &added_by, now))
    }
}

const CARD_COLUMNS: &str =
    "id, effort_id, canvas_id, kind, x, y, ref_kind, ref_id, text, added_by, created_ms";

type RawCard = (
    i64,
    i64,
    Option<i64>,
    String,
    i64,
    i64,
    Option<String>,
    Option<String>,
    String,
    String,
    i64,
);

fn raw_card(r: &Row<'_>) -> rusqlite::Result<RawCard> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
    ))
}

/// A card from its row, with its title read from the target.
fn make_card(conn: &Connection, raw: RawCard) -> Result<Card, DbError> {
    let (id, effort, canvas, kind, x, y, ref_kind, ref_id, text, added_by, created_ms) = raw;
    let reference = match (ref_kind, ref_id) {
        (Some(kind), Some(id)) => Some(CardRef {
            kind: from_text("card reference kind", &kind)?,
            id,
        }),
        _ => None,
    };
    let (title, missing) = match &reference {
        Some(r) => match ref_title(conn, r)? {
            Some(title) => (title, false),
            None => (MISSING_TITLE.to_owned(), true),
        },
        None => (card_title(&text), false),
    };
    Ok(Card {
        id: CardId(id),
        effort: EffortId(effort),
        canvas: canvas.map(CanvasId),
        kind: from_text("card kind", &kind)?,
        x,
        y,
        reference,
        title,
        text,
        missing,
        added_by,
        created_ms,
    })
}

/// The title of what a reference points at; `None` when it is not there (or the id does not fit
/// the kind).
fn ref_title(conn: &Connection, r: &CardRef) -> Result<Option<String>, DbError> {
    let sql = match r.kind {
        RefKind::Finding => "SELECT label FROM findings WHERE id = ?1",
        RefKind::Source => "SELECT locator FROM sources WHERE id = ?1",
        RefKind::Decision => "SELECT text FROM decisions WHERE id = ?1",
        RefKind::Map => "SELECT title FROM maps WHERE id = ?1",
        RefKind::Task => "SELECT title FROM tasks WHERE id = ?1",
    };
    let found: Option<String> = if r.kind == RefKind::Map {
        conn.query_row(sql, [&r.id], |row| row.get(0)).optional()?
    } else {
        match r.id.parse::<i64>() {
            Ok(n) => conn.query_row(sql, [n], |row| row.get(0)).optional()?,
            Err(_) => None,
        }
    };
    Ok(found.map(|t| card_title(&t)))
}

fn load_card(conn: &Connection, id: CardId) -> Result<Card, DbError> {
    let raw = conn
        .query_row(
            &format!("SELECT {CARD_COLUMNS} FROM cards WHERE id = ?1"),
            [id.0],
            raw_card,
        )
        .optional()?
        .ok_or_else(|| no_card(id))?;
    make_card(conn, raw)
}

fn no_card(id: CardId) -> DbError {
    DbError::NotFound(format!("no card {}", id.0))
}

fn no_canvas(id: CanvasId) -> DbError {
    DbError::NotFound(format!("no canvas {}", id.0))
}

fn load_state(conn: &Connection) -> Result<CanvasState, DbError> {
    let mut canvases = Vec::new();
    let mut stmt = conn.prepare("SELECT id, effort_id, title, framework FROM canvases ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
    })?;
    for row in rows {
        let (id, effort, title, framework) = row?;
        canvases.push(Canvas {
            id: CanvasId(id),
            effort: EffortId(effort),
            title,
            framework: from_text("canvas framework", &framework)?,
        });
    }
    let mut cards = Vec::new();
    let mut stmt = conn.prepare(&format!("SELECT {CARD_COLUMNS} FROM cards ORDER BY id"))?;
    let rows = stmt.query_map([], raw_card)?;
    for row in rows {
        cards.push(make_card(conn, row?)?);
    }
    let mut links = Vec::new();
    let mut stmt =
        conn.prepare("SELECT from_card, to_card, label FROM card_links ORDER BY from_card, to_card")?;
    let rows = stmt.query_map([], |r| {
        Ok(CardLink {
            from: CardId(r.get(0)?),
            to: CardId(r.get(1)?),
            label: r.get(2)?,
        })
    })?;
    for row in rows {
        links.push(row?);
    }
    Ok(CanvasState {
        canvases,
        cards,
        links,
    })
}

fn load_canvas(conn: &Connection, id: CanvasId) -> Result<Canvas, DbError> {
    let (effort, title, framework): (i64, String, String) = conn
        .query_row(
            "SELECT effort_id, title, framework FROM canvases WHERE id = ?1",
            [id.0],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .ok_or_else(|| no_canvas(id))?;
    Ok(Canvas {
        id,
        effort: EffortId(effort),
        title,
        framework: from_text("canvas framework", &framework)?,
    })
}

fn effort_name(conn: &Connection, id: EffortId) -> Result<String, DbError> {
    conn.query_row("SELECT name FROM efforts WHERE id = ?1", [id.0], |r| r.get(0))
        .optional()?
        .ok_or_else(|| DbError::NotFound(format!("no effort {}", id.0)))
}

fn create_canvas(
    conn: &Connection,
    effort: EffortId,
    title: &str,
    framework: Framework,
    now: i64,
) -> Result<Canvas, DbError> {
    let name = effort_name(conn, effort)?;
    let title = match title.trim() {
        "" => name,
        t => t.to_owned(),
    };
    conn.execute(
        "INSERT INTO canvases (effort_id, title, framework, created_ms) VALUES (?1, ?2, ?3, ?4)",
        params![effort.0, title, to_text(&framework)?, now],
    )?;
    load_canvas(conn, CanvasId(conn.last_insert_rowid()))
}

fn rename_canvas(conn: &Connection, id: CanvasId, title: &str) -> Result<Canvas, DbError> {
    if title.trim().is_empty() {
        return Err(DbError::Refused("a canvas needs a title".into()));
    }
    let changed = conn.execute(
        "UPDATE canvases SET title = ?2 WHERE id = ?1",
        params![id.0, title.trim()],
    )?;
    if changed == 0 {
        return Err(no_canvas(id));
    }
    load_canvas(conn, id)
}

fn add_card_linked(
    conn: &mut Connection,
    new: NewCard,
    link_from: Option<CardId>,
    added_by: &str,
    now: i64,
) -> Result<(Card, Option<CardLink>), DbError> {
    let tx = conn.transaction()?;
    let id = insert_card(&tx, &new, added_by, now)?;
    let link = match link_from {
        Some(from) => Some(link_cards(&tx, from, id, "")?),
        None => None,
    };
    let card = load_card(&tx, id)?;
    tx.commit()?;
    Ok((card, link))
}

/// Inserts a card after checking it: its effort and canvas, its kind against its reference, and
/// that the reference's target exists.
fn insert_card(
    tx: &Transaction<'_>,
    new: &NewCard,
    added_by: &str,
    now: i64,
) -> Result<CardId, DbError> {
    effort_name(tx, new.effort)?;
    if let Some(canvas) = new.canvas {
        let canvas = load_canvas(tx, canvas)?;
        if canvas.effort != new.effort {
            return Err(DbError::Refused(format!(
                "canvas {} belongs to another effort",
                canvas.id.0
            )));
        }
    }
    let (ref_kind, ref_id, text) = if new.kind.has_own_text() {
        if new.reference.is_some() {
            return Err(DbError::Refused(
                "an idea or question card holds its own text, not a reference".into(),
            ));
        }
        let text = new.text.trim();
        if text.is_empty() {
            return Err(DbError::Refused("an idea or question card needs text".into()));
        }
        (None, None, text.to_owned())
    } else {
        let Some(r) = &new.reference else {
            return Err(DbError::Refused(
                "this kind of card needs a reference to what it shows".into(),
            ));
        };
        if r.kind.card_kind() != new.kind {
            return Err(DbError::Refused(format!(
                "a {} reference makes a {} card",
                to_text(&r.kind)?,
                to_text(&r.kind.card_kind())?
            )));
        }
        if ref_title(tx, r)?.is_none() {
            return Err(DbError::NotFound(format!(
                "there is no {} {}",
                to_text(&r.kind)?,
                r.id
            )));
        }
        (Some(to_text(&r.kind)?), Some(r.id.clone()), String::new())
    };
    let (x, y) = if new.canvas.is_some() {
        (new.x, new.y)
    } else {
        (0, 0)
    };
    tx.execute(
        "INSERT INTO cards (effort_id, canvas_id, kind, x, y, ref_kind, ref_id, text, added_by, \
         created_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            new.effort.0,
            new.canvas.map(|c| c.0),
            to_text(&new.kind)?,
            x,
            y,
            ref_kind,
            ref_id,
            text,
            added_by,
            now
        ],
    )?;
    Ok(CardId(tx.last_insert_rowid()))
}

fn place_card(
    conn: &Connection,
    id: CardId,
    canvas: CanvasId,
    x: i64,
    y: i64,
) -> Result<Card, DbError> {
    let card = load_card(conn, id)?;
    if card.canvas.is_some() {
        return Err(DbError::Refused(format!("card {} is already placed", id.0)));
    }
    let target = load_canvas(conn, canvas)?;
    if target.effort != card.effort {
        return Err(DbError::Refused(format!(
            "canvas {} belongs to another effort than card {}",
            canvas.0, id.0
        )));
    }
    conn.execute(
        "UPDATE cards SET canvas_id = ?2, x = ?3, y = ?4 WHERE id = ?1",
        params![id.0, canvas.0, x, y],
    )?;
    load_card(conn, id)
}

fn move_card(conn: &Connection, id: CardId, x: i64, y: i64) -> Result<Card, DbError> {
    let card = load_card(conn, id)?;
    if card.canvas.is_none() {
        return Err(DbError::Refused(format!(
            "card {} is incoming: place it first",
            id.0
        )));
    }
    conn.execute(
        "UPDATE cards SET x = ?2, y = ?3 WHERE id = ?1",
        params![id.0, x, y],
    )?;
    load_card(conn, id)
}

fn delete_card(conn: &mut Connection, id: CardId, incoming_only: bool) -> Result<(), DbError> {
    let tx = conn.transaction()?;
    let card = load_card(&tx, id)?;
    if incoming_only && card.canvas.is_some() {
        return Err(DbError::Refused(format!(
            "card {} is on a canvas: delete it there",
            id.0
        )));
    }
    tx.execute(
        "DELETE FROM card_links WHERE from_card = ?1 OR to_card = ?1",
        [id.0],
    )?;
    tx.execute("DELETE FROM cards WHERE id = ?1", [id.0])?;
    tx.commit()?;
    Ok(())
}

fn link_cards(conn: &Connection, from: CardId, to: CardId, label: &str) -> Result<CardLink, DbError> {
    if from == to {
        return Err(DbError::Refused("a card cannot link to itself".into()));
    }
    let a = load_card(conn, from)?;
    let b = load_card(conn, to)?;
    if a.effort != b.effort {
        return Err(DbError::Refused(
            "cards of different efforts cannot be linked".into(),
        ));
    }
    let label = label.trim().to_owned();
    conn.execute(
        "INSERT INTO card_links (from_card, to_card, label) VALUES (?1, ?2, ?3) \
         ON CONFLICT (from_card, to_card) DO UPDATE SET label = excluded.label",
        params![from.0, to.0, label],
    )?;
    Ok(CardLink { from, to, label })
}

fn unlink_cards(conn: &Connection, from: CardId, to: CardId) -> Result<(), DbError> {
    let n = conn.execute(
        "DELETE FROM card_links WHERE from_card = ?1 AND to_card = ?2",
        params![from.0, to.0],
    )?;
    if n == 0 {
        return Err(DbError::NotFound(format!(
            "card {} is not linked to card {}",
            from.0, to.0
        )));
    }
    Ok(())
}

fn add_finding_card(
    conn: &mut Connection,
    finding: FindingId,
    task: TaskId,
    added_by: &str,
    now: i64,
) -> Result<Card, DbError> {
    let tx = conn.transaction()?;
    let effort: Option<Option<i64>> = tx
        .query_row("SELECT effort_id FROM tasks WHERE id = ?1", [task.0], |r| {
            r.get(0)
        })
        .optional()?;
    let Some(effort) = effort else {
        return Err(DbError::NotFound(format!("there is no task #{}", task.0)));
    };
    let new = NewCard {
        effort: effort.map_or(UNSORTED_EFFORT, EffortId),
        canvas: None,
        kind: CardKind::Finding,
        x: 0,
        y: 0,
        reference: Some(CardRef {
            kind: RefKind::Finding,
            id: finding.0.to_string(),
        }),
        text: String::new(),
    };
    let id = insert_card(&tx, &new, added_by, now)?;
    let card = load_card(&tx, id)?;
    tx.commit()?;
    Ok(card)
}

/// A card's body: an idea's or question's own words, else the target's text. A missing target
/// reads as such.
fn card_body(conn: &Connection, id: CardId) -> Result<String, DbError> {
    let card = load_card(conn, id)?;
    let Some(r) = &card.reference else {
        return Ok(card.text);
    };
    if card.missing {
        return Ok(format!("*The {} this card pointed at is missing.*", to_text(&r.kind)?));
    }
    let body: Option<String> = match r.kind {
        RefKind::Finding => one(conn, "SELECT body FROM findings WHERE id = ?1", &r.id)?,
        RefKind::Source => one(
            conn,
            "SELECT locator || char(10) || char(10) || content FROM sources WHERE id = ?1",
            &r.id,
        )?,
        RefKind::Decision => one(
            conn,
            "SELECT text || COALESCE(char(10) || char(10) || 'Why: ' || why, '') \
             FROM decisions WHERE id = ?1",
            &r.id,
        )?,
        RefKind::Map => conn
            .query_row(
                "SELECT title || ' (' || type || ', version ' || current_version || ')' \
                 FROM maps WHERE id = ?1",
                [&r.id],
                |row| row.get(0),
            )
            .optional()?,
        RefKind::Task => one(conn, "SELECT title || char(10) || char(10) || notes FROM tasks WHERE id = ?1", &r.id)?,
    };
    let mut body = body.unwrap_or_default();
    if body.chars().count() > BODY_MAX {
        body = body.chars().take(BODY_MAX).collect();
        body.push_str("\n\n*(cut)*");
    }
    Ok(body)
}

/// The one text `sql` finds for the numeric id `id`; `None` when there is none or `id` is no number.
fn one(conn: &Connection, sql: &str, id: &str) -> Result<Option<String>, DbError> {
    match id.parse::<i64>() {
        Ok(n) => Ok(conn.query_row(sql, [n], |r| r.get(0)).optional()?),
        Err(_) => Ok(None),
    }
}
