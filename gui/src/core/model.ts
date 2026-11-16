// The client's copy of the core's state: plain data, shaped after design v2's nouns
// (docs/design-v2.md §3). Field names are snake_case because that is what the Rust API
// serialises; the exact shapes are reconciled when the real core is wired (plan §3).
// Every reference is an id, never an array position or a name.

export type Id = string;

/** The five kinds of effort (design v2 §3 "Terms"). */
export type EffortKind = 'feature' | 'fix' | 'cleanup' | 'research' | 'experiment';

/** One of the five tag colours in `styles/tokens.css` (`--tag-<colour>`). */
export type TagColour = 'blue' | 'coral' | 'lilac' | 'mint' | 'gold';

/** A project as Home lists it. Only the open project's details are in the rest of the snapshot. */
export interface ProjectSummary {
  id: Id;
  name: string;
  repos: string[];
  /** The Claude account its sessions run under. */
  account: string;
  efforts: number;
  /** Efforts running in it now (counts against the machine's soft limit). */
  running: number;
  /** Inbox items waiting on the user. */
  needs_you: number;
  /** When it was last opened, as shown. */
  opened: string;
  note: string;
}

/** `open`: nobody claims it; `claimed`: an agent says it is met and waits for review; `met`: the user approved. */
export type CriterionState = 'open' | 'claimed' | 'met';
export interface Criterion { id: Id; text: string; state: CriterionState }

/** The "where was I" card each effort keeps (design v2 §4). Markdown. */
export interface WhereWasI { last: string; waits: string; next: string }

export interface Effort {
  id: Id;
  /** Lower kebab case (`payment-providers`). */
  name: string;
  kind: EffortKind;
  question_or_goal: string;
  running: boolean;
  /** Stable colour of its tag dot. */
  colour: TagColour;
  where_was_i: WhereWasI;
  criteria: Criterion[];
}

/** Set by Dovetail, never by hand (design v2 §4 "Task status"). */
export type TaskStatus = 'todo' | 'in_progress' | 'review' | 'done';
export interface Task { id: Id; effort_id: Id; title: string; status: TaskStatus }

export type AgentKind = 'slice' | 'subagent';
export type AgentState = 'working' | 'needs_you' | 'done' | 'idle';
export interface Agent {
  id: Id;
  task_id: Id;
  /** The agent that spawned it (a subagent's slice). */
  parent_id?: Id;
  kind: AgentKind;
  name: string;
  state: AgentState;
  branch?: string;
}

export type InboxKind = 'permission' | 'criterion' | 'question' | 'finished';
export interface InboxItem {
  id: Id;
  kind: InboxKind;
  effort_id: Id;
  /** The agent asking. */
  agent_id: Id;
  title: string;
  /** Age as shown ("2m"). */
  at: string;
  /** Markdown. */
  body: string;
  /** For a criterion review: the criterion it would tick. */
  criterion_id?: Id;
}

/** Who wrote a chat entry: the user, the coordinator, or an agent (by id). */
export type Speaker = 'you' | 'coordinator' | Id;
export interface ChatEntry {
  id: Id;
  effort_id?: Id;
  who: Speaker;
  /** A slice report: shown as one collapsed line. */
  report: boolean;
  /** Clock time as shown ("09:14"). */
  at: string;
  /** Markdown. */
  text: string;
}

export type Framework = 'question_tree' | 'options_compare' | 'free_board';
export interface Canvas { id: Id; effort_id: Id; framework: Framework; title: string }

export type CardKind = 'finding' | 'source' | 'idea' | 'decision' | 'question' | 'map';
export interface Card {
  /** Short and shown ("Q1", "F3"); also what a chat reference names. */
  id: Id;
  canvas_id: Id;
  kind: CardKind;
  x: number;
  y: number;
  title: string;
  body: string;
  /** A source card this one cites. */
  ref?: Id;
  /** Long cards open in the reading view: the report's id. */
  report_id?: Id;
}
export interface CardLink { from: Id; to: Id }

/** A card an agent proposed; it waits in the incoming strip until the user places or drops it. */
export interface IncomingCard {
  id: Id;
  canvas_id: Id;
  kind: CardKind;
  title: string;
  body: string;
  /** The card it would hang under when placed. */
  under: Id;
}

export type VariantLevel = 'global' | 'component' | 'content';
export interface VariantSlot { id: Id; level: VariantLevel; name: string; options: string[] }
/** The variants one slice declared for its web preview (design v2 §7). */
export interface VariantSet {
  agent_id: Id;
  effort_id: Id;
  /** The slice's preview. */
  url: string;
  slots: VariantSlot[];
  /** Option index per slot, in slot order: the kept combination. */
  kept: number[];
}

export interface Report { id: Id; effort_id: Id; title: string; markdown: string }

/** Everything the bottom bar shows besides the UI's own mode and focus. */
export interface Status {
  /** Soft limit on running efforts on this machine (never blocks). */
  effort_limit: number;
  /** Efforts running on this machine across all projects. */
  machine_running: number;
  account: string;
  /** Coordinator context fill, 0–1. */
  context_fill: number;
  usage: { five_hour: number; week: number };
  model: string;
  effort_level: string;
  permission_mode: string;
  /** A turn is running somewhere in the project. */
  busy: boolean;
}

export interface Snapshot {
  seq: number;
  projects: ProjectSummary[];
  open_project: Id | null;
  efforts: Effort[];
  tasks: Task[];
  agents: Agent[];
  inbox: InboxItem[];
  chat: ChatEntry[];
  canvases: Canvas[];
  cards: Card[];
  card_links: CardLink[];
  incoming: IncomingCard[];
  variants: VariantSet[];
  reports: Report[];
  status: Status;
}

/** A snapshot with nothing in it: what the client holds before the core's first message. */
export function emptySnapshot(): Snapshot {
  return {
    seq: 0, projects: [], open_project: null, efforts: [], tasks: [], agents: [], inbox: [], chat: [],
    canvases: [], cards: [], card_links: [], incoming: [], variants: [], reports: [],
    status: {
      effort_limit: 0, machine_running: 0, account: '', context_fill: 0, usage: { five_hour: 0, week: 0 },
      model: '', effort_level: '', permission_mode: '', busy: false,
    },
  };
}
