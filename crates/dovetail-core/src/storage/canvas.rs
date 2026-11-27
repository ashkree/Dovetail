//! Blocks, cards and links in `state.db` (ADR 0025, ADR 0027).
//!
//! Every position a block or a new card gets is chosen here, inside the request that stores it,
//! from what is already on the effort's canvas: a new block goes right of everything, a card in
//! a block goes to the first free cell (under its parent when it has one). Nothing already placed
//! moves.
//!
//! A card that points at something keeps no text of its own: its title is read from the target
//! when the card is read, and its body only on request ([`Db::load_card_body`]). The target is
//! checked to exist inside the same request that adds the card. A target that is gone anyway
//! (a damaged database) makes the card read as "missing"; it never fails a load.

use dovetail_api::research::FindingId;
use dovetail_api::tracking::{
    effort::{EffortId, UNSORTED_EFFORT},
    task::TaskId,
};
use dovetail_api::visual::canvas::{
    Block, BlockId, Card, CardId, CardKind, CardLink, CardRef, Layout, NewCard, Point, RefKind,
    card_title,
    geometry::{BLOCK_GAP, CARD_H, CARD_W, COLUMNS, GAP, HEADER, PAD},
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};

use super::sessions::{from_text, to_text};
use super::{Db, DbError, Pending};

/// What a card reads as when its target is gone.
pub const MISSING_TITLE: &str = "(missing)";

/// Longest body [`Db::load_card_body`] sends: a source's page can be long.
const BODY_MAX: usize = 20_000;

/// `added_by` of a card the user made: it starts seen.
pub const USER: &str = "user";

/// Everything canvas-shaped in a project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CanvasState {
    /// Blocks, in id order.
    pub blocks: Vec<Block>,
    /// Cards, in id order.
    pub cards: Vec<Card>,
    /// Links.
    pub links: Vec<CardLink>,
}

impl Db {
    /// Every block, card and link of the open project.
    pub fn load_canvas_state(&self) -> Pending<CanvasState> {
        self.run(|c, _| load_state(c.state()?))
    }

    /// Adds a block to an effort's canvas, in the empty space nearest `near`, else right of
    /// everything on it. An empty title becomes the layout's name.
    pub fn create_block(
        &self,
        effort: EffortId,
        title: String,
        layout: Layout,
        near: Option<Point>,
    ) -> Pending<Block> {
        self.run(move |c, now| create_block(c.state()?, effort, &title, layout, near, now))
    }

    /// One block.
    pub fn block(&self, id: BlockId) -> Pending<Block> {
        self.run(move |c, _| load_block(c.state()?, id))
    }

    /// Renames a block.
    pub fn rename_block(&self, id: BlockId, title: String) -> Pending<Block> {
        self.run(move |c, _| rename_block(c.state()?, id, &title))
    }

    /// Adds a card, checking in this request that its effort, block and target exist, and links
    /// it from `link_from` when given. A card with a block goes to its first free cell, under
    /// `link_from` when that card is in the same block; one an agent adds there starts unseen.
    /// Resolves to the card and the link.
    pub fn add_card(
        &self,
        card: NewCard,
        link_from: Option<CardId>,
        added_by: String,
    ) -> Pending<(Card, Option<CardLink>)> {
        self.run(move |c, now| add_card_linked(c.state()?, card, link_from, &added_by, now))
    }

    /// Places an incoming card in `block`, or in the block of the placed card that links to it,
    /// else the effort's newest block, else a new free area. Resolves to the card and the block
    /// when one was made for it.
    pub fn place_card(&self, id: CardId, block: Option<BlockId>) -> Pending<(Card, Option<Block>)> {
        self.run(move |c, now| place_card(c.state()?, id, block, now))
    }

    /// Marks a card seen.
    pub fn see_card(&self, id: CardId) -> Pending<Card> {
        self.run(move |c, _| see_card(c.state()?, id))
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
    "id, effort_id, block_id, kind, x, y, ref_kind, ref_id, text, added_by, created_ms, seen";

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
    bool,
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
        r.get(11)?,
    ))
}

/// A card from its row, with its title read from the target.
fn make_card(conn: &Connection, raw: RawCard) -> Result<Card, DbError> {
    let (id, effort, block, kind, x, y, ref_kind, ref_id, text, added_by, created_ms, seen) = raw;
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
        block: block.map(BlockId),
        kind: from_text("card kind", &kind)?,
        x,
        y,
        reference,
        title,
        text,
        missing,
        added_by,
        created_ms,
        seen,
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

fn no_block(id: BlockId) -> DbError {
    DbError::NotFound(format!("no block {}", id.0))
}

const BLOCK_COLUMNS: &str = "id, effort_id, title, layout, x, y";

type RawBlock = (i64, i64, String, String, i64, i64);

fn raw_block(r: &Row<'_>) -> rusqlite::Result<RawBlock> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
    ))
}

fn make_block((id, effort, title, layout, x, y): RawBlock) -> Result<Block, DbError> {
    Ok(Block {
        id: BlockId(id),
        effort: EffortId(effort),
        title,
        layout: from_text("block layout", &layout)?,
        x,
        y,
    })
}

fn load_state(conn: &Connection) -> Result<CanvasState, DbError> {
    let mut blocks = Vec::new();
    let mut stmt = conn.prepare(&format!("SELECT {BLOCK_COLUMNS} FROM blocks ORDER BY id"))?;
    for row in stmt.query_map([], raw_block)? {
        blocks.push(make_block(row?)?);
    }
    let mut cards = Vec::new();
    let mut stmt = conn.prepare(&format!("SELECT {CARD_COLUMNS} FROM cards ORDER BY id"))?;
    let rows = stmt.query_map([], raw_card)?;
    for row in rows {
        cards.push(make_card(conn, row?)?);
    }
    let mut links = Vec::new();
    let mut stmt = conn
        .prepare("SELECT from_card, to_card, label FROM card_links ORDER BY from_card, to_card")?;
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
        blocks,
        cards,
        links,
    })
}

fn load_block(conn: &Connection, id: BlockId) -> Result<Block, DbError> {
    let raw = conn
        .query_row(
            &format!("SELECT {BLOCK_COLUMNS} FROM blocks WHERE id = ?1"),
            [id.0],
            raw_block,
        )
        .optional()?
        .ok_or_else(|| no_block(id))?;
    make_block(raw)
}

fn effort_name(conn: &Connection, id: EffortId) -> Result<String, DbError> {
    conn.query_row("SELECT name FROM efforts WHERE id = ?1", [id.0], |r| {
        r.get(0)
    })
    .optional()?
    .ok_or_else(|| DbError::NotFound(format!("no effort {}", id.0)))
}

/// What an untitled block is called.
fn layout_name(layout: Layout) -> &'static str {
    match layout {
        Layout::QuestionTree => "Question tree",
        Layout::OptionsCompare => "Options compare",
        Layout::FreeArea => "Free area",
    }
}

fn create_block(
    conn: &Connection,
    effort: EffortId,
    title: &str,
    layout: Layout,
    near: Option<Point>,
    now: i64,
) -> Result<Block, DbError> {
    effort_name(conn, effort)?;
    let title = match title.trim() {
        "" => layout_name(layout).to_owned(),
        t => t.to_owned(),
    };
    let (x, y) = Board::read(conn, effort)?.new_block_spot(near);
    conn.execute(
        "INSERT INTO blocks (effort_id, title, layout, x, y, created_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![effort.0, title, to_text(&layout)?, x, y, now],
    )?;
    load_block(conn, BlockId(conn.last_insert_rowid()))
}

fn rename_block(conn: &Connection, id: BlockId, title: &str) -> Result<Block, DbError> {
    if title.trim().is_empty() {
        return Err(DbError::Refused("a block needs a title".into()));
    }
    let changed = conn.execute(
        "UPDATE blocks SET title = ?2 WHERE id = ?1",
        params![id.0, title.trim()],
    )?;
    if changed == 0 {
        return Err(no_block(id));
    }
    load_block(conn, id)
}

/// What is on one effort's canvas, for choosing where something new goes.
struct Board {
    blocks: Vec<Block>,
    /// Every placed card of the effort: its block and position.
    cards: Vec<(BlockId, i64, i64)>,
}

/// A rectangle in canvas pixels.
#[derive(Debug, Clone, Copy)]
struct Rect {
    x: i64,
    y: i64,
    w: i64,
    h: i64,
}

impl Rect {
    fn card(x: i64, y: i64) -> Self {
        Self {
            x,
            y,
            w: CARD_W,
            h: CARD_H,
        }
    }

    fn overlaps(self, o: Rect) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }
}

/// How many rings of `BLOCK_GAP` steps a new block's search goes out from the view's centre.
const NEAR_RINGS: i64 = 25;

/// How far a search for a free cell looks before it gives up and goes below everything.
const SEARCH_ROWS: i64 = 200;
const SEARCH_COLUMNS: i64 = 50;

impl Board {
    fn read(conn: &Connection, effort: EffortId) -> Result<Self, DbError> {
        let mut blocks = Vec::new();
        let mut stmt = conn.prepare(&format!(
            "SELECT {BLOCK_COLUMNS} FROM blocks WHERE effort_id = ?1 ORDER BY id"
        ))?;
        for row in stmt.query_map([effort.0], raw_block)? {
            blocks.push(make_block(row?)?);
        }
        let mut stmt = conn.prepare(
            "SELECT block_id, x, y FROM cards WHERE effort_id = ?1 AND block_id IS NOT NULL",
        )?;
        let cards = stmt
            .query_map([effort.0], |r| {
                Ok((BlockId(r.get(0)?), r.get(1)?, r.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { blocks, cards })
    }

    /// The frame a block draws: around its cards, at least one cell big.
    fn extent(&self, b: &Block) -> Rect {
        let (mut right, mut bottom) = (b.x + 2 * PAD + CARD_W, b.y + HEADER + CARD_H + PAD);
        for &(_, x, y) in self.cards.iter().filter(|c| c.0 == b.id) {
            right = right.max(x + CARD_W + PAD);
            bottom = bottom.max(y + CARD_H + PAD);
        }
        Rect {
            x: b.x,
            y: b.y,
            w: right - b.x,
            h: bottom - b.y,
        }
    }

    /// Where a new block goes: centred on `near` when that space is empty, else the empty spot
    /// nearest it on a `BLOCK_GAP` grid; with no `near` (an agent's block), or nothing empty in
    /// reach, right of every block at the top. Empty: its smallest frame, with `BLOCK_GAP / 2`
    /// around it, touches no frame and no card.
    fn new_block_spot(&self, near: Option<Point>) -> (i64, i64) {
        let (w, h) = (2 * PAD + CARD_W, HEADER + CARD_H + PAD);
        if let Some(p) = near {
            let margin = BLOCK_GAP / 2;
            let free = |x: i64, y: i64| {
                let r = Rect {
                    x: x - margin,
                    y: y - margin,
                    w: w + 2 * margin,
                    h: h + 2 * margin,
                };
                !self.blocks.iter().any(|b| r.overlaps(self.extent(b)))
                    && !self
                        .cards
                        .iter()
                        .any(|&(_, cx, cy)| r.overlaps(Rect::card(cx, cy)))
            };
            let (x0, y0) = (p.x - w / 2, p.y - h / 2);
            for ring in 0..NEAR_RINGS {
                // The cells of this ring, nearest first.
                let mut cells: Vec<(i64, i64)> = (-ring..=ring)
                    .flat_map(|i| (-ring..=ring).map(move |j| (i, j)))
                    .filter(|&(i, j)| i.abs().max(j.abs()) == ring)
                    .collect();
                cells.sort_by_key(|&(i, j)| i * i + j * j);
                for (i, j) in cells {
                    let (x, y) = (x0 + i * BLOCK_GAP, y0 + j * BLOCK_GAP);
                    if free(x, y) {
                        return (x, y);
                    }
                }
            }
        }
        let right = self
            .blocks
            .iter()
            .map(|b| {
                let e = self.extent(b);
                e.x + e.w + BLOCK_GAP
            })
            .max();
        (right.unwrap_or(0), 0)
    }

    /// The first free cell of `block` for a new card: under `under` (a card of that block, at
    /// its position) when given, else as the layout fills. A cell is free when it touches no
    /// card and no other block's frame.
    fn card_spot(&self, block: &Block, under: Option<(i64, i64)>) -> (i64, i64) {
        let step_x = CARD_W + GAP;
        let step_y = CARD_H + GAP;
        let cell = |col: i64, row: i64| {
            (
                block.x + PAD + col * step_x,
                block.y + HEADER + row * step_y,
            )
        };
        let free = |(x, y): (i64, i64)| {
            let r = Rect::card(x, y);
            // The block's frame grows by PAD around a card: that must not reach another frame.
            let grown = Rect {
                w: CARD_W + PAD,
                h: CARD_H + PAD,
                ..r
            };
            !self
                .cards
                .iter()
                .any(|&(_, cx, cy)| r.overlaps(Rect::card(cx, cy)))
                && !self
                    .blocks
                    .iter()
                    .filter(|b| b.id != block.id)
                    .any(|b| grown.overlaps(self.extent(b)))
        };
        let (rows, cols): (std::ops::Range<i64>, std::ops::Range<i64>) = match (under, block.layout)
        {
            (Some((ux, uy)), layout) => {
                let col = ((ux - block.x - PAD) / step_x).max(0);
                let row = ((uy - block.y - HEADER) / step_y).max(0) + 1;
                let cols = match layout {
                    // Evidence stays in its option's column.
                    Layout::OptionsCompare => col..col + 1,
                    Layout::QuestionTree | Layout::FreeArea => col..col + COLUMNS,
                };
                (row..row + SEARCH_ROWS, cols)
            }
            // A new option heads the next column.
            (None, Layout::OptionsCompare) => (0..1, 0..SEARCH_COLUMNS),
            (None, Layout::QuestionTree | Layout::FreeArea) => (0..SEARCH_ROWS, 0..COLUMNS),
        };
        for row in rows {
            for col in cols.clone() {
                if free(cell(col, row)) {
                    return cell(col, row);
                }
            }
        }
        // Nowhere free in reach: below everything in the block.
        let e = self.extent(block);
        (block.x + PAD, e.y + e.h)
    }
}

fn add_card_linked(
    conn: &mut Connection,
    new: NewCard,
    link_from: Option<CardId>,
    added_by: &str,
    now: i64,
) -> Result<(Card, Option<CardLink>), DbError> {
    let tx = conn.transaction()?;
    // The parent's position, when it sits in the block the card goes to.
    let under = match (link_from, new.block) {
        (Some(from), Some(block)) => {
            let parent = load_card(&tx, from)?;
            (parent.block == Some(block)).then_some((parent.x, parent.y))
        }
        _ => None,
    };
    let id = insert_card(&tx, &new, under, added_by, now)?;
    let link = match link_from {
        Some(from) => Some(link_cards(&tx, from, id, "")?),
        None => None,
    };
    let card = load_card(&tx, id)?;
    tx.commit()?;
    Ok((card, link))
}

/// Inserts a card after checking it: its effort and block, its kind against its reference, and
/// that the reference's target exists. In a block it goes to the first free cell, under the card
/// at `under` when given.
fn insert_card(
    tx: &Transaction<'_>,
    new: &NewCard,
    under: Option<(i64, i64)>,
    added_by: &str,
    now: i64,
) -> Result<CardId, DbError> {
    effort_name(tx, new.effort)?;
    let block = match new.block {
        Some(id) => {
            let block = load_block(tx, id)?;
            if block.effort != new.effort {
                return Err(DbError::Refused(format!(
                    "block {} belongs to another effort",
                    block.id.0
                )));
            }
            Some(block)
        }
        None => None,
    };
    let (ref_kind, ref_id, text) = if new.kind.has_own_text() {
        if new.reference.is_some() {
            return Err(DbError::Refused(
                "an idea or question card holds its own text, not a reference".into(),
            ));
        }
        let text = new.text.trim();
        if text.is_empty() {
            return Err(DbError::Refused(
                "an idea or question card needs text".into(),
            ));
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
    let (x, y) = match &block {
        Some(block) => Board::read(tx, new.effort)?.card_spot(block, under),
        None => (0, 0),
    };
    // A card the user makes, or one waiting in incoming, needs no "new" mark.
    let seen = block.is_none() || added_by == USER;
    tx.execute(
        "INSERT INTO cards (effort_id, block_id, kind, x, y, ref_kind, ref_id, text, added_by, \
         created_ms, seen) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            new.effort.0,
            new.block.map(|b| b.0),
            to_text(&new.kind)?,
            x,
            y,
            ref_kind,
            ref_id,
            text,
            added_by,
            now,
            seen
        ],
    )?;
    Ok(CardId(tx.last_insert_rowid()))
}

fn place_card(
    conn: &mut Connection,
    id: CardId,
    block: Option<BlockId>,
    now: i64,
) -> Result<(Card, Option<Block>), DbError> {
    let tx = conn.transaction()?;
    let card = load_card(&tx, id)?;
    if card.block.is_some() {
        return Err(DbError::Refused(format!("card {} is already placed", id.0)));
    }
    // The placed card that links to it: it goes under that one.
    let parent: Option<Card> = {
        let mut stmt = tx.prepare("SELECT from_card FROM card_links WHERE to_card = ?1")?;
        let froms = stmt
            .query_map([id.0], |r| r.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut found = None;
        for from in froms {
            let c = load_card(&tx, CardId(from))?;
            if c.block.is_some() {
                found = Some(c);
                break;
            }
        }
        found
    };
    let newest: Option<i64> = tx
        .query_row(
            "SELECT MAX(id) FROM blocks WHERE effort_id = ?1",
            [card.effort.0],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let mut made = None;
    let target = match block
        .or(parent.as_ref().and_then(|p| p.block))
        .or(newest.map(BlockId))
    {
        Some(b) => load_block(&tx, b)?,
        None => {
            let b = create_block(&tx, card.effort, "", Layout::FreeArea, None, now)?;
            made = Some(b.clone());
            b
        }
    };
    if target.effort != card.effort {
        return Err(DbError::Refused(format!(
            "block {} belongs to another effort than card {}",
            target.id.0, id.0
        )));
    }
    let under = parent
        .filter(|p| p.block == Some(target.id))
        .map(|p| (p.x, p.y));
    let (x, y) = Board::read(&tx, card.effort)?.card_spot(&target, under);
    tx.execute(
        "UPDATE cards SET block_id = ?2, x = ?3, y = ?4, seen = 1 WHERE id = ?1",
        params![id.0, target.id.0, x, y],
    )?;
    let card = load_card(&tx, id)?;
    tx.commit()?;
    Ok((card, made))
}

fn see_card(conn: &Connection, id: CardId) -> Result<Card, DbError> {
    if conn.execute("UPDATE cards SET seen = 1 WHERE id = ?1", [id.0])? == 0 {
        return Err(no_card(id));
    }
    load_card(conn, id)
}

fn move_card(conn: &Connection, id: CardId, x: i64, y: i64) -> Result<Card, DbError> {
    let card = load_card(conn, id)?;
    if card.block.is_none() {
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
    if incoming_only && card.block.is_some() {
        return Err(DbError::Refused(format!(
            "card {} is on the canvas: delete it there",
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

fn link_cards(
    conn: &Connection,
    from: CardId,
    to: CardId,
    label: &str,
) -> Result<CardLink, DbError> {
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
        block: None,
        kind: CardKind::Finding,
        reference: Some(CardRef {
            kind: RefKind::Finding,
            id: finding.0.to_string(),
        }),
        text: String::new(),
    };
    let id = insert_card(&tx, &new, None, added_by, now)?;
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
        return Ok(format!(
            "*The {} this card pointed at is missing.*",
            to_text(&r.kind)?
        ));
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
        RefKind::Task => one(
            conn,
            "SELECT title || char(10) || char(10) || notes FROM tasks WHERE id = ?1",
            &r.id,
        )?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Db, STATE_MIGRATIONS, migrate};
    use crate::paths::Paths;
    use dovetail_api::tracking::{decision::NewDecision, task::NewTask};

    fn db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths {
            data: dir.path().join("data"),
            config: dir.path().join("config"),
            state: dir.path().join("state"),
            runtime: dir.path().join("run"),
        };
        let db = Db::open(&paths, Some("p")).unwrap();
        (dir, db)
    }

    fn reference(kind: RefKind, id: &str) -> NewCard {
        NewCard {
            effort: UNSORTED_EFFORT,
            block: None,
            kind: kind.card_kind(),
            reference: Some(CardRef {
                kind,
                id: id.into(),
            }),
            text: String::new(),
        }
    }

    #[test]
    fn the_tables_come_from_a_migration_over_version_17_and_from_empty() {
        let canvas_steps = STATE_MIGRATIONS
            .iter()
            .position(|(name, _)| name.ends_with("0018_canvases.sql"))
            .expect("the canvases migration is listed");
        assert_eq!(canvas_steps, 17, "it follows 0017_efforts");
        let tables = |conn: &Connection| -> Vec<String> {
            conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
                .unwrap()
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        // Version 17: a project with an effort and a task, as released before canvases.
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        migrate(
            &mut conn,
            std::path::Path::new(":memory:"),
            &STATE_MIGRATIONS[..17],
        )
        .unwrap();
        assert!(!tables(&conn).iter().any(|t| t == "cards"));
        let before: i64 = conn
            .query_row("SELECT count(*) FROM efforts", [], |r| r.get(0))
            .unwrap();
        migrate(
            &mut conn,
            std::path::Path::new(":memory:"),
            STATE_MIGRATIONS,
        )
        .unwrap();
        for t in ["blocks", "cards", "card_links"] {
            assert!(tables(&conn).iter().any(|x| x == t), "{t}");
        }
        let after: i64 = conn
            .query_row("SELECT count(*) FROM efforts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after, "existing rows are untouched");
        assert_eq!(load_state(&conn).unwrap(), CanvasState::default());
        // From empty.
        let mut empty = Connection::open_in_memory().unwrap();
        migrate(
            &mut empty,
            std::path::Path::new(":memory:"),
            STATE_MIGRATIONS,
        )
        .unwrap();
        assert!(tables(&empty).iter().any(|t| t == "card_links"));
    }

    /// ADR 0027: state migration 0021 from version 20. Each canvas becomes a block around its
    /// cards, `free_board` becomes `free_area`, cards keep their positions and links and are seen.
    #[test]
    fn canvases_become_blocks_and_cards_keep_their_places() {
        let blocks_step = STATE_MIGRATIONS
            .iter()
            .position(|(name, _)| name.ends_with("0021_blocks.sql"))
            .expect("the blocks migration is listed");
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        let at = std::path::Path::new(":memory:");
        migrate(&mut conn, at, &STATE_MIGRATIONS[..blocks_step]).unwrap();
        conn.execute_batch(
            "INSERT INTO canvases (id, effort_id, title, framework, created_ms) VALUES \
               (1, 1, 'Which provider?', 'question_tree', 5), (2, 1, 'Notes', 'free_board', 6); \
             INSERT INTO cards (id, effort_id, canvas_id, kind, x, y, text, added_by, created_ms) VALUES \
               (1, 1, 1, 'question', 360, 20, 'Root', 'user', 7), \
               (2, 1, 1, 'idea', 40, 170, 'Child', 'user', 8), \
               (3, 1, NULL, 'idea', 0, 0, 'Waiting', 'a', 9); \
             INSERT INTO card_links (from_card, to_card, label) VALUES (1, 2, 'answers');",
        )
        .unwrap();
        migrate(&mut conn, at, STATE_MIGRATIONS).unwrap();
        let state = load_state(&conn).unwrap();
        let layouts: Vec<(i64, Layout, i64, i64)> = state
            .blocks
            .iter()
            .map(|b| (b.id.0, b.layout, b.x, b.y))
            .collect();
        assert_eq!(
            layouts,
            [
                (1, Layout::QuestionTree, 40 - PAD, 20 - HEADER),
                (2, Layout::FreeArea, 0, 0)
            ]
        );
        let cards: Vec<(i64, Option<i64>, i64, i64, bool)> = state
            .cards
            .iter()
            .map(|c| (c.id.0, c.block.map(|b| b.0), c.x, c.y, c.seen))
            .collect();
        assert_eq!(
            cards,
            [
                (1, Some(1), 360, 20, true),
                (2, Some(1), 40, 170, true),
                (3, None, 0, 0, true)
            ]
        );
        assert_eq!(state.links.len(), 1);
        // The foreign keys point at the new tables.
        assert!(
            conn.execute("INSERT INTO cards (effort_id, block_id, kind, added_by, created_ms) VALUES (1, 9, 'idea', 'u', 1)", [])
                .is_err()
        );
    }

    #[test]
    fn a_reference_is_checked_titled_and_read_on_request_and_a_missing_target_does_not_stop_a_load()
    {
        let (_dir, db) = db();
        let tasks = db
            .create_task(
                NewTask {
                    title: "Pick a provider".into(),
                    p_level: None,
                    bucket: None,
                    notes: "Compare fees".into(),
                    follow_up_of: None,
                    pr: None,
                    related: vec![],
                    effort: Default::default(),
                    criterion: None,
                },
                false,
            )
            .wait()
            .unwrap();
        let task = tasks[0].id;
        let decision = db
            .log_decision(
                NewDecision {
                    task,
                    text: "Use Stripe".into(),
                    why: Some("cheapest".into()),
                    alternatives: vec![],
                    supersedes: None,
                },
                dovetail_api::tracking::decision::DecisionAuthor::User,
            )
            .wait()
            .unwrap();
        let (card, _) = db
            .add_card(
                reference(RefKind::Task, &task.0.to_string()),
                None,
                "user".into(),
            )
            .wait()
            .unwrap();
        assert_eq!(
            (card.title.as_str(), card.missing, card.text.as_str()),
            ("Pick a provider", false, "")
        );
        let (dcard, _) = db
            .add_card(
                reference(RefKind::Decision, &decision.id.0.to_string()),
                None,
                "a".into(),
            )
            .wait()
            .unwrap();
        assert_eq!(dcard.title, "Use Stripe");
        let body = db.load_card_body(dcard.id).wait().unwrap();
        assert!(
            body.contains("Use Stripe") && body.contains("Why: cheapest"),
            "{body}"
        );
        let body = db.load_card_body(card.id).wait().unwrap();
        assert!(body.contains("Compare fees"), "{body}");

        // The target must exist when the card is made, and the kinds must agree.
        let err = db
            .add_card(reference(RefKind::Finding, "5"), None, "a".into())
            .wait()
            .unwrap_err();
        assert!(err.to_string().contains("there is no finding 5"), "{err}");
        let err = db
            .add_card(reference(RefKind::Map, "nope"), None, "a".into())
            .wait()
            .unwrap_err();
        assert!(err.to_string().contains("no map nope"), "{err}");
        let mut wrong = reference(RefKind::Task, "1");
        wrong.kind = CardKind::Finding;
        assert!(db.add_card(wrong, None, "a".into()).wait().is_err());
        assert_eq!(db.load_canvas_state().wait().unwrap().cards.len(), 2);

        // A damaged database: the target row is gone. The card reads "missing" and loads.
        db.run(move |c, _| {
            c.state()?.execute("PRAGMA foreign_keys = OFF", [])?;
            c.state()?.execute(
                "DELETE FROM decisions WHERE id = ?1",
                [decision.id.0 as i64],
            )?;
            Ok(())
        })
        .wait()
        .unwrap();
        let state = db.load_canvas_state().wait().unwrap();
        let missing = state.cards.iter().find(|c| c.id == dcard.id).unwrap();
        assert!(missing.missing);
        assert_eq!(missing.title, MISSING_TITLE);
        assert!(
            db.load_card_body(dcard.id)
                .wait()
                .unwrap()
                .contains("missing")
        );
    }

    #[test]
    fn a_finding_card_for_an_unknown_task_is_refused() {
        let (_dir, db) = db();
        let err = db
            .add_finding_card(FindingId(1), TaskId(9), "slice-a".into())
            .wait()
            .unwrap_err();
        assert!(err.to_string().contains("no task"), "{err}");
    }
}
