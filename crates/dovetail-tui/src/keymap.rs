//! The keymap: one data-like table of (mode, scope, key sequence) → [`Action`], and the
//! resolver that walks it as keys arrive. The table is hardcoded in v1 and shaped so it can
//! be read from TOML later.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::Action;
use crate::mode::Mode;
use crate::region::BufferKind;

/// How long a half-typed sequence waits for its next key (plan B27).
pub const SEQUENCE_TIMEOUT_MS: u64 = 1000;

/// A key with its modifiers, normalised so that `A` is `Char('A')` with no Shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    code: KeyCode,
    mods: KeyModifiers,
}

impl Key {
    /// A plain key.
    pub fn new(code: KeyCode) -> Self {
        Self {
            code,
            mods: KeyModifiers::NONE,
        }
    }

    /// Normalises a terminal key event.
    pub fn from_event(event: KeyEvent) -> Self {
        let mut mods = event.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT);
        let code = match event.code {
            KeyCode::Char(c) => KeyCode::Char(c),
            // Terminals report Shift-Tab as BackTab, some with Shift and some without; keep it
            // distinct from Tab, and drop the Shift so both match `<S-Tab>`.
            KeyCode::BackTab => KeyCode::BackTab,
            other => {
                mods |= event.modifiers & KeyModifiers::SHIFT;
                other
            }
        };
        Self { code, mods }
    }

    /// How the key is shown in the status bar while a sequence is pending.
    pub fn label(&self) -> String {
        let base = match self.code {
            KeyCode::Char(' ') => "␣".to_string(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Enter => "⏎".into(),
            KeyCode::Tab => "⇥".into(),
            KeyCode::Esc => "⎋".into(),
            other => format!("{other:?}"),
        };
        if self.mods.contains(KeyModifiers::CONTROL) {
            format!("^{base}")
        } else if self.mods.contains(KeyModifiers::ALT) {
            format!("⌥{base}")
        } else {
            base
        }
    }

    /// Parses a sequence such as `gg`, `<leader>ss`, `<C-j>` or `<CR>`.
    pub fn parse_sequence(text: &str) -> Result<Vec<Key>, KeyError> {
        let mut keys = Vec::new();
        let mut rest = text;
        while let Some(c) = rest.chars().next() {
            if c == '<'
                && let Some(end) = rest.find('>')
            {
                keys.push(Self::parse_named(&rest[1..end])?);
                rest = &rest[end + 1..];
            } else {
                keys.push(Key::new(KeyCode::Char(c)));
                rest = &rest[c.len_utf8()..];
            }
        }
        if keys.is_empty() {
            return Err(KeyError::Empty);
        }
        Ok(keys)
    }

    fn parse_named(name: &str) -> Result<Key, KeyError> {
        let lower = name.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("c-") {
            let mut key = Self::parse_named(rest)?;
            key.mods |= KeyModifiers::CONTROL;
            return Ok(key);
        }
        if let Some(rest) = lower.strip_prefix("a-") {
            let mut key = Self::parse_named(rest)?;
            key.mods |= KeyModifiers::ALT;
            return Ok(key);
        }
        let code = match lower.as_str() {
            "leader" | "space" => KeyCode::Char(' '),
            "tab" => KeyCode::Tab,
            "s-tab" | "backtab" => KeyCode::BackTab,
            "cr" | "enter" => KeyCode::Enter,
            "esc" => KeyCode::Esc,
            "bs" => KeyCode::Backspace,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "pageup" => KeyCode::PageUp,
            "pagedown" => KeyCode::PageDown,
            "lt" => KeyCode::Char('<'),
            s if s.chars().count() == 1 => KeyCode::Char(s.chars().next().unwrap_or(' ')),
            _ => return Err(KeyError::Unknown(name.to_owned())),
        };
        Ok(Key::new(code))
    }
}

/// A key sequence that does not parse. The keymap table becomes user TOML later, so this is
/// what a user would see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// Nothing to parse.
    Empty,
    /// `<name>` is not a key Dovetail knows.
    Unknown(String),
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyError::Empty => f.write_str("empty key sequence"),
            KeyError::Unknown(name) => write!(f, "unknown key <{name}>"),
        }
    }
}

impl std::error::Error for KeyError {}

/// A binding whose keys do not parse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapError {
    /// The binding's keys as written.
    pub keys: String,
    /// What is wrong with them.
    pub source: KeyError,
}

impl std::fmt::Display for KeymapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "binding {:?}: {}", self.keys, self.source)
    }
}

impl std::error::Error for KeymapError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Where a binding applies. Movement is bound in [`Scope::Global`]; actions in a buffer's scope.
/// A buffer-local binding wins over a global one on the same keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Every buffer.
    Global,
    /// Only when this buffer has focus.
    Buffer(BufferKind),
}

/// One row of the keymap table.
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    /// Mode it applies in.
    pub mode: Mode,
    /// Where it applies.
    pub scope: Scope,
    /// The keys, in the notation [`Key::parse_sequence`] reads.
    pub keys: &'static str,
    /// What it does.
    pub action: Action,
}

const fn bind(mode: Mode, scope: Scope, keys: &'static str, action: Action) -> Binding {
    Binding {
        mode,
        scope,
        keys,
        action,
    }
}

const N: Mode = Mode::Normal;
const M: Mode = Mode::Map;
const G: Scope = Scope::Global;
const TREE: Scope = Scope::Buffer(BufferKind::Tree);
const INBOX: Scope = Scope::Buffer(BufferKind::Inbox);
const ACTIVITY: Scope = Scope::Buffer(BufferKind::Activity);
const MESSAGES: Scope = Scope::Buffer(BufferKind::Messages);
const DETAIL: Scope = Scope::Buffer(BufferKind::TaskDetail);
const INBOX_DETAIL: Scope = Scope::Buffer(BufferKind::InboxDetail);
const CONVERSATION: Scope = Scope::Buffer(BufferKind::Conversation);
const BOARD: Scope = Scope::Buffer(BufferKind::Board);
const WORKTREES: Scope = Scope::Buffer(BufferKind::Worktrees);
const MAP_SHELF: Scope = Scope::Buffer(BufferKind::MapShelf);
const MAP_OUTLINE: Scope = Scope::Buffer(BufferKind::MapOutline);
const SIDE_PANEL: Scope = Scope::Buffer(BufferKind::SidePanel);
const LIBRARY: Scope = Scope::Buffer(BufferKind::Library);
const READER: Scope = Scope::Buffer(BufferKind::Reader);
const ASK: Scope = Scope::Buffer(BufferKind::Ask);

/// The v1 keymap (Decisions 10, 17 and 24; plan B2).
pub const BINDINGS: &[Binding] = &[
    // Movement: global. `j`/`k` move inside the focused list and never leave it (B2).
    bind(N, G, "j", Action::MoveDown),
    bind(N, G, "k", Action::MoveUp),
    bind(N, G, "<Down>", Action::MoveDown),
    bind(N, G, "<Up>", Action::MoveUp),
    bind(N, G, "<PageDown>", Action::PageDown),
    bind(N, G, "<PageUp>", Action::PageUp),
    bind(N, G, "gg", Action::GoTop),
    bind(N, G, "G", Action::GoBottom),
    bind(N, G, "h", Action::FocusLeft),
    bind(N, G, "l", Action::FocusRight),
    bind(N, G, "<C-j>", Action::FocusDown),
    bind(N, G, "<C-k>", Action::FocusUp),
    bind(N, G, "<leader>n", Action::FocusInbox),
    bind(N, G, ":", Action::EnterCommand),
    // Ctrl-C quits in two steps and works in every mode, as in Claude Code. Interrupting a turn
    // stays on `<leader>sx`.
    bind(N, G, "<C-c>", Action::QuitOnRepeat),
    bind(Mode::Insert, G, "<C-c>", Action::QuitOnRepeat),
    bind(Mode::Command, G, "<C-c>", Action::QuitOnRepeat),
    bind(M, G, "<C-c>", Action::QuitOnRepeat),
    bind(N, G, "i", Action::EnterInsert),
    bind(N, G, "<leader>c", Action::FocusCoordinator),
    bind(N, G, "<leader><Tab>", Action::AlternateAgent),
    bind(N, G, "<leader>z", Action::ToggleFocusMode),
    bind(N, G, "<leader>t", Action::QuickCapture),
    bind(N, G, "/", Action::Search),
    bind(N, G, "n", Action::SearchNext),
    bind(N, G, "N", Action::SearchPrev),
    bind(N, G, "<leader>ss", Action::PickAgent),
    bind(N, G, "<leader><leader>", Action::PickAny),
    bind(N, G, "<leader>a", Action::ShowActivity),
    bind(N, G, "<leader>m", Action::ShowMessages),
    bind(N, G, "<leader>b", Action::ShowBoard),
    bind(N, G, "<leader>g", Action::ShowWorktrees),
    bind(N, G, "<leader>M", Action::ShowMaps),
    bind(N, G, "<leader>v", Action::MapMode),
    bind(N, G, "<leader>sn", Action::NewSlice),
    bind(N, G, "<leader>sq", Action::NewSideChat),
    bind(N, G, "<leader>?", Action::ToggleSidePanel),
    bind(N, G, "<leader>sx", Action::Interrupt),
    bind(N, G, "<leader>sm", Action::CyclePermissionMode),
    bind(N, G, "<leader>st", Action::TakeOver),
    bind(N, G, "<leader>pp", Action::SwitchProject),
    bind(N, G, "<leader>pr", Action::RegisterProject),
    // Feeds and the task detail go back with Esc (or the key that opened them).
    bind(N, ACTIVITY, "<Esc>", Action::BackToConversation),
    bind(N, MESSAGES, "<Esc>", Action::BackToConversation),
    bind(N, DETAIL, "<Esc>", Action::BackToConversation),
    bind(N, ACTIVITY, "<CR>", Action::FeedOpen),
    bind(N, MESSAGES, "<CR>", Action::FeedOpen),
    bind(N, DETAIL, "s", Action::TaskCycleStatus),
    bind(N, DETAIL, "b", Action::TaskCycleBucket),
    bind(N, DETAIL, "p", Action::TaskCyclePLevel),
    bind(N, DETAIL, "e", Action::TaskEditNotes),
    bind(N, DETAIL, "R", Action::TaskRelate),
    bind(N, DETAIL, "a", Action::TaskAddDecision),
    // Research rows (R3): `Enter` reads the row under the cursor; in the reading view `e` hands
    // the file to the editor (`TaskEditNotes` there) and `Esc` returns to the detail.
    bind(N, DETAIL, "<CR>", Action::ResearchOpen),
    // `j`/`k` scroll line by line; `Tab`/`Shift-Tab` jump between the research rows; `e` on a
    // selected row edits its file, and edits the notes when no row is selected.
    bind(N, DETAIL, "<Tab>", Action::ResearchNext),
    bind(N, DETAIL, "<S-Tab>", Action::ResearchPrev),
    // Conversation: reading keys (the brief's "Reading keys").
    bind(N, CONVERSATION, "]]", Action::NextTurn),
    bind(N, CONVERSATION, "[[", Action::PrevTurn),
    bind(N, CONVERSATION, "zc", Action::FoldTurn),
    bind(N, CONVERSATION, "zo", Action::UnfoldTurn),
    // The chat bar and the conversation above it are two stops: `Ctrl-k` goes up to read with
    // the draft kept, `Ctrl-j` (or `i`) comes back down to it. `PageUp`/`PageDown` scroll
    // without leaving the chat bar.
    bind(N, CONVERSATION, "<C-j>", Action::EnterInsert),
    bind(Mode::Insert, CONVERSATION, "<C-k>", Action::LeaveChatBar),
    bind(Mode::Insert, CONVERSATION, "<PageDown>", Action::PageDown),
    bind(Mode::Insert, CONVERSATION, "<PageUp>", Action::PageUp),
    // The chat bar only: not quick capture, task notes or inbox answers (task #1).
    bind(Mode::Insert, CONVERSATION, "<C-v>", Action::PasteClipboard),
    // Backspace on an empty chat input drops the last image chip; otherwise it deletes text.
    bind(
        Mode::Insert,
        CONVERSATION,
        "<BS>",
        Action::DropLastAttachment,
    ),
    // The side panel: the conversation's reading keys, plus switching and closing side chats.
    // Its chat bar takes the conversation's Insert-mode keys (`App::input_key`).
    bind(N, SIDE_PANEL, "]]", Action::NextTurn),
    bind(N, SIDE_PANEL, "[[", Action::PrevTurn),
    bind(N, SIDE_PANEL, "zc", Action::FoldTurn),
    bind(N, SIDE_PANEL, "zo", Action::UnfoldTurn),
    bind(N, SIDE_PANEL, "<C-j>", Action::EnterInsert),
    bind(Mode::Insert, SIDE_PANEL, "<PageDown>", Action::PageDown),
    bind(Mode::Insert, SIDE_PANEL, "<PageUp>", Action::PageUp),
    bind(N, SIDE_PANEL, "<Tab>", Action::NextSideChat),
    bind(N, SIDE_PANEL, "<S-Tab>", Action::PrevSideChat),
    bind(N, SIDE_PANEL, "dd", Action::CloseSideChat),
    // Reading mode (ADR 0018, decided 2026-11-11). `Esc` in Normal mode leaves it from any of
    // its columns; in Ask's chat bar `Esc` only returns to Normal. `i` asks about the open
    // report from the reading column and Ask, and does nothing in the library.
    bind(N, G, "<leader>r", Action::ToggleReadingMode),
    bind(N, LIBRARY, "<Esc>", Action::LeaveReadingMode),
    bind(N, READER, "<Esc>", Action::LeaveReadingMode),
    bind(N, ASK, "<Esc>", Action::LeaveReadingMode),
    bind(N, LIBRARY, "<CR>", Action::LibraryOpen),
    bind(N, LIBRARY, "o", Action::LibraryToggleFold),
    bind(N, LIBRARY, "i", Action::Ignore),
    bind(N, READER, "]s", Action::NextCitation),
    bind(N, READER, "[s", Action::PrevCitation),
    bind(N, READER, "gd", Action::OpenCitation),
    bind(N, READER, "gx", Action::BrowseCitation),
    bind(N, READER, "<C-o>", Action::ReaderBack),
    bind(N, READER, "]]", Action::NextTurn),
    bind(N, READER, "[[", Action::PrevTurn),
    bind(N, READER, "zc", Action::FoldTurn),
    bind(N, READER, "zo", Action::UnfoldTurn),
    bind(N, READER, "i", Action::AskAboutReport),
    bind(N, ASK, "i", Action::AskAboutReport),
    bind(N, ASK, "<C-j>", Action::AskAboutReport),
    bind(N, ASK, "]s", Action::NextCitation),
    bind(N, ASK, "[s", Action::PrevCitation),
    bind(N, ASK, "gd", Action::OpenCitation),
    bind(N, ASK, "gx", Action::BrowseCitation),
    bind(N, ASK, "]]", Action::NextTurn),
    bind(N, ASK, "[[", Action::PrevTurn),
    bind(N, ASK, "dd", Action::CloseAsk),
    bind(Mode::Insert, ASK, "<PageDown>", Action::PageDown),
    bind(Mode::Insert, ASK, "<PageUp>", Action::PageUp),
    // Task tree (Decision 24).
    bind(N, TREE, "<CR>", Action::TreeOpen),
    bind(N, TREE, "o", Action::TreeToggleFold),
    bind(N, TREE, "a", Action::QuickCapture),
    bind(N, TREE, "e", Action::TaskEditTitle),
    bind(N, TREE, "s", Action::TaskCycleStatus),
    bind(N, TREE, "b", Action::TaskCycleBucket),
    bind(N, TREE, "p", Action::TaskCyclePLevel),
    bind(N, TREE, "J", Action::TreeMoveDown),
    bind(N, TREE, "K", Action::TreeMoveUp),
    bind(N, TREE, "dd", Action::TaskArchive),
    // Board (task #22, decided 2026-11-09): `Tab`/`Shift-Tab` move between columns, `H`/`L`
    // move the card to the previous or next status, `<`/`>` move it up or down its column.
    // `h`/`l` stay region focus (B2).
    bind(N, BOARD, "<Esc>", Action::BackToConversation),
    bind(N, BOARD, "<Tab>", Action::BoardColumnRight),
    bind(N, BOARD, "<S-Tab>", Action::BoardColumnLeft),
    bind(N, BOARD, "H", Action::BoardMoveCardLeft),
    bind(N, BOARD, "L", Action::BoardMoveCardRight),
    bind(N, BOARD, "<lt>", Action::BoardCardUp),
    bind(N, BOARD, ">", Action::BoardCardDown),
    bind(N, BOARD, "<CR>", Action::BoardOpen),
    bind(N, BOARD, "a", Action::QuickCapture),
    bind(N, BOARD, "s", Action::TaskCycleStatus),
    // Worktree view (the brief's "Worktrees"): Enter opens the session, `e` the editor, `c`
    // copies the path, `X` removes (asking first if work is unpushed).
    bind(N, WORKTREES, "<Esc>", Action::BackToConversation),
    bind(N, WORKTREES, "<CR>", Action::WorktreeOpen),
    bind(N, WORKTREES, "e", Action::WorktreeEdit),
    bind(N, WORKTREES, "c", Action::WorktreeCopyPath),
    bind(N, WORKTREES, "X", Action::WorktreeRemove),
    // Maps (M15e): the shelf opens a map in the outline; `o` folds a group, `f` switches the
    // flow, `]]`/`[[` walk its steps. Esc goes outline -> shelf -> conversation.
    bind(N, MAP_SHELF, "<Esc>", Action::BackToConversation),
    bind(N, MAP_SHELF, "<CR>", Action::MapOpen),
    bind(N, MAP_OUTLINE, "<Esc>", Action::BackToConversation),
    bind(N, MAP_OUTLINE, "o", Action::MapToggleFold),
    bind(N, MAP_OUTLINE, "f", Action::MapNextFlow),
    bind(N, MAP_OUTLINE, "]]", Action::MapNextStep),
    bind(N, MAP_OUTLINE, "[[", Action::MapPrevStep),
    // MAP mode (decisions 23f, 23g, 24): the map fills the screen. `hjkl` select the nearest
    // node that way, `Tab`/`Shift-Tab` follow connections, `Enter` drills into a group and
    // `Backspace` comes back up, `n`/`p` walk the flow and `f` switches it, `+ - 0` zoom,
    // `Ctrl-hjkl` pan (B2), `v` shows or hides the outline, `u` asks for a redraw (23h).
    // `c` (comments) is v1.1 and `s` stays unbound (B6).
    bind(M, G, "<Esc>", Action::LeaveMapMode),
    bind(M, G, ":", Action::EnterCommand),
    bind(M, G, "h", Action::MapSelectLeft),
    bind(M, G, "j", Action::MapSelectDown),
    bind(M, G, "k", Action::MapSelectUp),
    bind(M, G, "l", Action::MapSelectRight),
    bind(M, G, "<Left>", Action::MapSelectLeft),
    bind(M, G, "<Down>", Action::MapSelectDown),
    bind(M, G, "<Up>", Action::MapSelectUp),
    bind(M, G, "<Right>", Action::MapSelectRight),
    bind(M, G, "<Tab>", Action::MapFollow),
    bind(M, G, "<S-Tab>", Action::MapFollowBack),
    bind(M, G, "<CR>", Action::MapDrillIn),
    bind(M, G, "<BS>", Action::MapDrillOut),
    bind(M, G, "n", Action::MapNextStep),
    bind(M, G, "p", Action::MapPrevStep),
    bind(M, G, "f", Action::MapNextFlow),
    bind(M, G, "+", Action::MapZoomIn),
    // `=` is `+` without Shift on most keyboards.
    bind(M, G, "=", Action::MapZoomIn),
    bind(M, G, "-", Action::MapZoomOut),
    bind(M, G, "0", Action::MapZoomFit),
    bind(M, G, "<C-h>", Action::MapPanLeft),
    bind(M, G, "<C-j>", Action::MapPanDown),
    bind(M, G, "<C-k>", Action::MapPanUp),
    bind(M, G, "<C-l>", Action::MapPanRight),
    bind(M, G, "v", Action::MapToggleOutline),
    bind(M, G, "u", Action::MapRedraw),
    // An inbox item in full goes back with Esc; the answer keys are in `INBOX_ANSWERS`.
    bind(N, INBOX_DETAIL, "<Esc>", Action::BackToConversation),
];

/// The inbox answer keys (Decision 17), bound in both the inbox list and an inbox item in full
/// (Normal mode). Enter opens the item from the list, and its agent from the item.
pub const INBOX_ANSWERS: &[(&str, Action)] = &[
    ("y", Action::InboxAllow),
    ("A", Action::InboxAllowAlways),
    ("d", Action::InboxDeny),
    ("D", Action::InboxDenyWithReason),
    ("r", Action::InboxReply),
    ("x", Action::InboxDismiss),
    ("<CR>", Action::InboxOpen),
];

/// The whole built-in table: [`BINDINGS`] plus [`INBOX_ANSWERS`] in each inbox scope.
pub fn default_bindings() -> Vec<Binding> {
    let answers = [INBOX, INBOX_DETAIL].into_iter().flat_map(|scope| {
        INBOX_ANSWERS
            .iter()
            .map(move |&(keys, action)| bind(N, scope, keys, action))
    });
    BINDINGS.iter().copied().chain(answers).collect()
}

#[derive(Debug, Clone)]
struct Entry {
    mode: Mode,
    scope: Scope,
    keys: Vec<Key>,
    action: Action,
}

/// A parsed keymap table.
#[derive(Debug, Clone)]
pub struct Keymap {
    entries: Vec<Entry>,
}

impl Keymap {
    /// Parses a table. Fails on a malformed key sequence.
    pub fn new(bindings: &[Binding]) -> Result<Self, KeymapError> {
        let entries = bindings
            .iter()
            .map(|b| {
                Ok(Entry {
                    mode: b.mode,
                    scope: b.scope,
                    keys: Key::parse_sequence(b.keys).map_err(|source| KeymapError {
                        keys: b.keys.to_owned(),
                        source,
                    })?,
                    action: b.action,
                })
            })
            .collect::<Result<_, KeymapError>>()?;
        Ok(Self { entries })
    }

    /// The built-in keymap.
    pub fn default_map() -> Self {
        // The table is constant and checked by a test, so parsing cannot fail.
        Self::new(&default_bindings()).unwrap_or(Self {
            entries: Vec::new(),
        })
    }

    /// The global single-key binding for `key` in `mode`, if any. Insert and Command mode type
    /// text, so only such chords (like Ctrl-C) are looked up there; the rest of the keys are text.
    pub fn global_chord(&self, mode: Mode, key: Key) -> Option<Action> {
        self.chord(mode, Scope::Global, key)
    }

    /// The chord typed into `buffer`'s text input in `mode`: its own binding first (Ctrl-V and
    /// Backspace in a chat bar), then a global one.
    pub fn text_chord(&self, mode: Mode, buffer: BufferKind, key: Key) -> Option<Action> {
        self.chord(mode, Scope::Buffer(buffer), key)
            .or_else(|| self.global_chord(mode, key))
    }

    /// The single-key binding for `key` in `mode` and exactly `scope`, if any.
    fn chord(&self, mode: Mode, scope: Scope, key: Key) -> Option<Action> {
        self.entries
            .iter()
            .find(|e| e.mode == mode && e.scope == scope && e.keys == [key])
            .map(|e| e.action)
    }

    /// Entries for a mode whose scope is one of `scopes`, as (scope rank, keys, action); a lower
    /// rank is more specific.
    fn candidates<'a>(
        &'a self,
        mode: Mode,
        scopes: &'a [Scope],
    ) -> impl Iterator<Item = (usize, &'a [Key], Action)> + 'a {
        self.entries.iter().filter_map(move |e| {
            if e.mode != mode {
                return None;
            }
            let rank = scopes.iter().position(|s| *s == e.scope)?;
            Some((rank, e.keys.as_slice(), e.action))
        })
    }
}

/// What the resolver made of a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    /// A complete sequence: run the action.
    Action(Action),
    /// A prefix of a longer sequence: wait for more keys.
    Pending,
    /// Nothing is bound to this; the pending keys were dropped.
    Unmatched,
}

/// Walks the keymap as keys arrive, with the sequence timeout.
#[derive(Debug, Default)]
pub struct Resolver {
    pending: Vec<Key>,
    /// When the first pending key arrived.
    since_ms: u64,
    /// The action to run if the timeout fires: set when the pending keys are a complete
    /// sequence that is also the start of a longer one (`g` against `gg`).
    ambiguous: Option<Action>,
}

impl Resolver {
    /// Feeds one key. `scopes` lists the scopes in force, most specific first; `now_ms` is a
    /// monotonic clock.
    pub fn feed(
        &mut self,
        map: &Keymap,
        mode: Mode,
        scopes: &[Scope],
        key: Key,
        now_ms: u64,
    ) -> Resolved {
        if self.pending.is_empty() {
            self.since_ms = now_ms;
        }
        self.pending.push(key);
        let mut exact: Option<(usize, Action)> = None;
        let mut longer = false;
        for (rank, keys, action) in map.candidates(mode, scopes) {
            if keys.starts_with(&self.pending) {
                if keys.len() == self.pending.len() {
                    if exact.is_none_or(|(best, _)| rank < best) {
                        exact = Some((rank, action));
                    }
                } else {
                    longer = true;
                }
            }
        }
        match (exact, longer) {
            (Some((_, action)), false) => {
                self.reset();
                Resolved::Action(action)
            }
            (Some((_, action)), true) => {
                self.ambiguous = Some(action);
                Resolved::Pending
            }
            (None, true) => {
                self.ambiguous = None;
                Resolved::Pending
            }
            (None, false) => {
                self.reset();
                Resolved::Unmatched
            }
        }
    }

    /// When the pending keys give up, if any are pending.
    pub fn deadline_ms(&self) -> Option<u64> {
        (!self.pending.is_empty()).then_some(self.since_ms + SEQUENCE_TIMEOUT_MS)
    }

    /// The deadline passed: runs the ambiguous shorter sequence if there is one.
    pub fn on_timeout(&mut self) -> Option<Action> {
        let action = self.ambiguous;
        self.reset();
        action
    }

    /// Keys typed so far in an unfinished sequence.
    pub fn pending(&self) -> &[Key] {
        &self.pending
    }

    /// Drops any unfinished sequence.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.ambiguous = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TREE_SCOPES: [Scope; 2] = [TREE, G];

    fn keys(text: &str) -> Vec<Key> {
        Key::parse_sequence(text).expect("valid sequence")
    }

    fn feed_all(
        resolver: &mut Resolver,
        map: &Keymap,
        scopes: &[Scope],
        text: &str,
        start_ms: u64,
    ) -> Resolved {
        let mut last = Resolved::Unmatched;
        for (i, key) in keys(text).into_iter().enumerate() {
            last = resolver.feed(map, Mode::Normal, scopes, key, start_ms + i as u64);
        }
        last
    }

    #[test]
    fn the_builtin_table_parses() {
        assert!(Keymap::new(&default_bindings()).is_ok());
        assert!(!Keymap::default_map().entries.is_empty());
    }

    #[test]
    fn sequence_notation() {
        assert_eq!(
            keys("<leader>ss"),
            vec![
                Key::new(KeyCode::Char(' ')),
                Key::new(KeyCode::Char('s')),
                Key::new(KeyCode::Char('s'))
            ]
        );
        assert_eq!(
            keys("<C-j>"),
            vec![Key {
                code: KeyCode::Char('j'),
                mods: KeyModifiers::CONTROL
            }]
        );
        assert_eq!(keys("<CR>"), vec![Key::new(KeyCode::Enter)]);
        assert_eq!(keys("]]").len(), 2);
        assert!(Key::parse_sequence("<nope>").is_err());
        assert!(Key::parse_sequence("").is_err());
    }

    #[test]
    fn shift_is_dropped_from_letters() {
        let event = KeyEvent::new(KeyCode::Char('A'), KeyModifiers::SHIFT);
        assert_eq!(Key::from_event(event), Key::new(KeyCode::Char('A')));
    }

    #[test]
    fn shift_tab_matches_with_or_without_shift() {
        let expected = Key::parse_sequence("<S-Tab>").expect("parses")[0];
        for mods in [KeyModifiers::SHIFT, KeyModifiers::NONE] {
            let event = KeyEvent::new(KeyCode::BackTab, mods);
            assert_eq!(Key::from_event(event), expected);
        }
    }

    #[test]
    fn gg_waits_for_its_second_key() {
        let map = Keymap::default_map();
        let mut r = Resolver::default();
        let first = r.feed(&map, Mode::Normal, &TREE_SCOPES, keys("g")[0], 0);
        assert_eq!(first, Resolved::Pending);
        assert_eq!(r.deadline_ms(), Some(SEQUENCE_TIMEOUT_MS));
        let second = r.feed(&map, Mode::Normal, &TREE_SCOPES, keys("g")[0], 10);
        assert_eq!(second, Resolved::Action(Action::GoTop));
        assert_eq!(r.deadline_ms(), None);
    }

    #[test]
    fn dd_is_bound_in_the_tree_only() {
        let map = Keymap::default_map();
        let mut r = Resolver::default();
        assert_eq!(
            feed_all(&mut r, &map, &TREE_SCOPES, "dd", 0),
            Resolved::Action(Action::TaskArchive)
        );
        // In the inbox `d` is deny, complete on the first key.
        let inbox = [INBOX, G];
        assert_eq!(
            feed_all(&mut r, &map, &inbox, "d", 0),
            Resolved::Action(Action::InboxDeny)
        );
    }

    #[test]
    fn an_unbound_key_is_unmatched_and_clears_the_sequence() {
        let map = Keymap::default_map();
        let mut r = Resolver::default();
        // `gx` is bound in reading mode's columns only (ADR 0018), so in the tree it is not.
        assert_eq!(
            feed_all(&mut r, &map, &TREE_SCOPES, "gq", 0),
            Resolved::Unmatched
        );
        assert!(r.pending().is_empty());
        assert_eq!(
            feed_all(&mut r, &map, &TREE_SCOPES, "gx", 0),
            Resolved::Unmatched
        );
    }

    #[test]
    fn reading_mode_keys_resolve_in_its_columns() {
        let map = Keymap::default_map();
        let reader = [READER, G];
        let library = [LIBRARY, G];
        let ask = [ASK, G];
        let resolve = |scopes: &[Scope], text: &str| {
            let mut r = Resolver::default();
            feed_all(&mut r, &map, scopes, text, 0)
        };
        assert_eq!(
            resolve(&TREE_SCOPES, "<leader>r"),
            Resolved::Action(Action::ToggleReadingMode)
        );
        assert_eq!(
            resolve(&reader, "gx"),
            Resolved::Action(Action::BrowseCitation)
        );
        assert_eq!(
            resolve(&reader, "gd"),
            Resolved::Action(Action::OpenCitation)
        );
        assert_eq!(
            resolve(&reader, "]s"),
            Resolved::Action(Action::NextCitation)
        );
        assert_eq!(
            resolve(&reader, "[s"),
            Resolved::Action(Action::PrevCitation)
        );
        assert_eq!(
            resolve(&reader, "i"),
            Resolved::Action(Action::AskAboutReport)
        );
        for scopes in [&reader, &library, &ask] {
            assert_eq!(
                resolve(scopes, "<Esc>"),
                Resolved::Action(Action::LeaveReadingMode)
            );
        }
        assert_eq!(resolve(&library, "i"), Resolved::Action(Action::Ignore));
        // `gg` still goes to the top in the reading column.
        assert_eq!(resolve(&reader, "gg"), Resolved::Action(Action::GoTop));
    }

    #[test]
    fn a_pending_sequence_times_out_and_is_dropped() {
        let map = Keymap::default_map();
        let mut r = Resolver::default();
        assert_eq!(
            feed_all(&mut r, &map, &TREE_SCOPES, "<leader>", 100),
            Resolved::Pending
        );
        assert_eq!(r.deadline_ms(), Some(100 + SEQUENCE_TIMEOUT_MS));
        assert_eq!(r.on_timeout(), None);
        assert!(r.pending().is_empty());
    }

    fn table(rows: &[(&'static str, Action)]) -> Keymap {
        let bindings: Vec<Binding> = rows.iter().map(|(k, a)| bind(N, G, k, *a)).collect();
        Keymap::new(&bindings).expect("valid table")
    }

    #[test]
    fn an_ambiguous_prefix_runs_the_short_binding_on_timeout() {
        let map = table(&[("g", Action::FocusInbox), ("gg", Action::GoTop)]);
        let mut r = Resolver::default();
        assert_eq!(
            r.feed(&map, Mode::Normal, &[G], keys("g")[0], 0),
            Resolved::Pending
        );
        assert_eq!(r.on_timeout(), Some(Action::FocusInbox));
        // Typing the second key in time still gets the long binding.
        r.feed(&map, Mode::Normal, &[G], keys("g")[0], 2_000);
        assert_eq!(
            r.feed(&map, Mode::Normal, &[G], keys("g")[0], 2_100),
            Resolved::Action(Action::GoTop)
        );
    }

    #[test]
    fn buffer_local_bindings_win_over_global_ones() {
        let map = Keymap::new(&[
            bind(N, G, "x", Action::MoveDown),
            bind(N, TREE, "x", Action::TaskArchive),
        ])
        .expect("valid table");
        let mut r = Resolver::default();
        assert_eq!(
            r.feed(&map, Mode::Normal, &TREE_SCOPES, keys("x")[0], 0),
            Resolved::Action(Action::TaskArchive)
        );
        assert_eq!(
            r.feed(&map, Mode::Normal, &[G], keys("x")[0], 0),
            Resolved::Action(Action::MoveDown)
        );
    }

    #[test]
    fn bindings_only_apply_in_their_mode() {
        let map = Keymap::default_map();
        let mut r = Resolver::default();
        assert_eq!(
            r.feed(&map, Mode::Insert, &TREE_SCOPES, keys("j")[0], 0),
            Resolved::Unmatched
        );
    }

    #[test]
    fn leader_and_bracket_sequences_resolve() {
        let map = table(&[
            ("<leader>ss", Action::FocusInbox),
            ("<leader>sx", Action::GoTop),
            ("]]", Action::GoBottom),
        ]);
        let mut r = Resolver::default();
        assert_eq!(
            feed_all(&mut r, &map, &[G], "<leader>s", 0),
            Resolved::Pending
        );
        assert_eq!(
            feed_all(&mut r, &map, &[G], "s", 5),
            Resolved::Action(Action::FocusInbox)
        );
        assert_eq!(
            feed_all(&mut r, &map, &[G], "]]", 10),
            Resolved::Action(Action::GoBottom)
        );
    }

    #[test]
    fn map_keys_resolve_in_their_own_buffers_only() {
        let map = Keymap::default_map();
        let shelf = [MAP_SHELF, G];
        let outline = [MAP_OUTLINE, G];
        let resolve = |scopes: &[Scope], text: &str| {
            let mut r = Resolver::default();
            feed_all(&mut r, &map, scopes, text, 0)
        };
        assert_eq!(
            resolve(&[G], "<leader>M"),
            Resolved::Action(Action::ShowMaps)
        );
        assert_eq!(resolve(&shelf, "<CR>"), Resolved::Action(Action::MapOpen));
        assert_eq!(
            resolve(&outline, "o"),
            Resolved::Action(Action::MapToggleFold)
        );
        assert_eq!(
            resolve(&outline, "f"),
            Resolved::Action(Action::MapNextFlow)
        );
        assert_eq!(
            resolve(&outline, "]]"),
            Resolved::Action(Action::MapNextStep)
        );
        assert_eq!(
            resolve(&outline, "[["),
            Resolved::Action(Action::MapPrevStep)
        );
        assert_eq!(
            resolve(&outline, "<Esc>"),
            Resolved::Action(Action::BackToConversation)
        );
        // Not leaked into other buffers.
        assert_eq!(resolve(&shelf, "o"), Resolved::Unmatched);
        assert_eq!(resolve(&TREE_SCOPES, "]]"), Resolved::Unmatched);
    }

    #[test]
    fn leader_n_focuses_the_inbox() {
        let map = Keymap::default_map();
        let mut r = Resolver::default();
        assert_eq!(
            feed_all(&mut r, &map, &TREE_SCOPES, "<leader>n", 0),
            Resolved::Action(Action::FocusInbox)
        );
    }
}
