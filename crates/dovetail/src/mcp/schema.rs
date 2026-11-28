//! MCP `tools/list` definitions and JSON input schemas for Dovetail's tools.

use dovetail_tools::{Suite, ToolSpec, tools_in};
use serde_json::{Value, json};

/// The `tools/list` entries of `suites`: each tool in table order, then, for one release, each
/// renamed tool's old name with an open schema and a description naming the new tool.
pub(super) fn tools_list(suites: &[Suite]) -> Vec<Value> {
    let defs = tool_defs();
    let current = tools_in(suites).filter_map(|spec| {
        let def = defs.iter().find(|d| d.name == spec.name)?;
        Some(json!({
            "name": def.name,
            "description": def.description,
            "inputSchema": (def.schema)(),
        }))
    });
    let old = tools_in(suites).filter_map(|spec: &ToolSpec| {
        let old = spec.old_name?;
        Some(json!({
            "name": old,
            "description": format!(
                "Deprecated: the old name of `{}`, kept for conversations from before the \
                 rename. Call `{}` instead; it takes the same arguments.",
                spec.name, spec.name
            ),
            "inputSchema": {"type": "object"},
        }))
    });
    current.chain(old).collect()
}

struct ToolDef {
    name: &'static str,
    description: &'static str,
    schema: fn() -> Value,
}

fn tool_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "core_ask_user",
            description: "Ask the user a question and get the answer through Dovetail's inbox. \
                Use this for questions that must wait while the user is away, or for what only \
                the user can give, such as credentials or access; questions for a user who is in \
                the conversation go in your reply. The call returns at once: the question goes to \
                the user's inbox and conversation, and the answer arrives later as a new user \
                message. Keep working \
                on anything that does not depend on it, or end your turn if nothing is left. \
                Make the question self-contained, with the options you see.",
            schema: || {
                json!({"type": "object", "properties": {
                    "question": {"type": "string", "description": "The question, self-contained, with the options if there are any."}
                }, "required": ["question"]})
            },
        },
        ToolDef {
            name: "core_report_status",
            description: "Set your one-line status, shown next to you in Dovetail's task tree. \
                Call it when you start, when your focus changes, and when you finish. Plain \
                words, present tense, at most 120 characters (longer text is cut).",
            schema: || {
                json!({"type": "object", "properties": {
                    "status": {"type": "string", "maxLength": 120, "description": "One line, 120 characters at most."}
                }, "required": ["status"]})
            },
        },
        ToolDef {
            name: "tasks_create",
            description: "Create a task in the user's task list. Use it to file work you have \
                split out of the feature, or follow-ups you notice, instead of only mentioning \
                them in chat. The task lands in Unsorted (or the bucket you give), flagged as \
                coordinator-created until the user has seen it; it never goes straight into \
                Now. Returns the new task's id; pass that id to `project_start_slice` or \
                `core_log_decision`.",
            schema: || {
                json!({"type": "object", "properties": {
                    "title": {"type": "string", "description": "Short title, 6 words at most."},
                    "notes": {"type": "string", "description": "Context, acceptance criteria, links."},
                    "p_level": {"type": "string", "enum": ["P0", "P1", "P2", "P3"], "description": "Priority, P0 highest. Omit if unsure."},
                    "bucket": {"type": "string", "enum": ["next", "later"], "description": "Omit to land in Unsorted. Never Now: the user decides what is current."},
                    "follow_up_of": {"type": "integer", "description": "Id of the task this follows up."},
                    "related": {"type": "array", "items": {"type": "integer"}, "description": "Ids of related tasks."}
                }, "required": ["title"]})
            },
        },
        ToolDef {
            name: "core_log_decision",
            description: "Record a decision in the task's decision log, so it outlives this \
                conversation. Use it when you or the user settle a design choice, scope cut or \
                approach, in one line, with the reason and the alternatives that lost. Entries \
                are append-only; to change an earlier decision, log a new one that supersedes \
                it. Slices log on their own task by default; the coordinator must name `task`.",
            schema: || {
                json!({"type": "object", "properties": {
                    "task": {"type": "integer", "description": "Task id. Required for the coordinator; slices default to their own task."},
                    "decision": {"type": "string", "description": "The decision, one line."},
                    "why": {"type": "string", "description": "The reason."},
                    "alternatives": {"type": "array", "description": "Options considered and rejected.", "items": {
                        "type": "object", "properties": {
                            "option": {"type": "string"},
                            "why_not": {"type": "string", "description": "Why it lost."}
                        }, "required": ["option"]}},
                    "supersedes": {"type": "integer", "description": "Id of the earlier decision this replaces."}
                }, "required": ["decision"]})
            },
        },
        ToolDef {
            name: "project_start_slice",
            description: "Start a slice session (its own git worktree) on an existing task. \
                For new work call `tasks_create` first. Returns at once with `started`, \
                `queued` (the maximum number of slices is running; it starts when one is \
                archived), `denied`, or `pending` (the user is asked to approve; the answer \
                arrives later as a message, \"project_start_slice <name> approved\" or \"... \
                denied\"). Do not message the slice until Dovetail sends you \"slice <name> \
                ready\"; a setup failure arrives the same way. The brief becomes the slice's \
                first prompt, so make it complete: goal, scope, files, checks to run. The \
                slice works in one of the project's repos: name it with `repo` when the \
                project has several (refused without it); with one it may be left out. Pick \
                `driver` (`claude` by default, or `agy`; an agy slice cannot receive \
                SendMessage), the session's `model` (optional, default `sonnet` on `claude`, \
                or an agy family such as `claude-opus-5-5` or `gemini-3.8-flash` on `agy`), and \
                `effort` for the work. An unknown model is `denied`; if the model or effort did not take, \
                you are told in a message.",
            schema: || {
                json!({"type": "object", "properties": {
                    "task": {"type": "integer", "description": "Id of an existing task."},
                    "name": {"type": "string", "description": "Short slice name, used for the branch and worktree."},
                    "brief": {"type": "string", "description": "The slice's first prompt."},
                    "repo": {"type": "string", "description": "The repo it works in, by name, as Dovetail listed it to you. Required when the project has several repos."},
                    "driver": {"type": "string", "enum": ["claude", "agy"], "description": "Optional, default claude. Set to `agy` to run the slice on agy (note: an agy slice cannot receive SendMessage)."},
                    "model": {"type": "string", "description": "Optional (default sonnet on claude, or agy's default on agy). The model the slice runs on: a Claude alias/id, or an agy family (`claude-opus-5-5`, `gemini-3.8-flash`)."},
                    "effort": {"type": "string", "enum": ["low", "medium", "high", "xhigh", "max"], "description": "The slice's effort level. Left out, the driver's default applies."}
                }, "required": ["task", "name", "brief"]})
            },
        },
        ToolDef {
            name: "project_add",
            description: "Make the folder you run in a Dovetail project, so its tasks, inbox \
                and this conversation are kept. Call it only when the user asks for that. Until \
                then everything lives in memory and is lost when Dovetail quits. Takes no \
                arguments; calling it on a folder that is already a project does nothing.",
            schema: || json!({"type": "object", "properties": {}}),
        },
        ToolDef {
            name: "project_archive_slice",
            description: "Archive a slice once its work is done, normally right after its PR \
                is merged and you have pulled. It stops the slice's session and removes its \
                worktree and `dovetail/*` branch, freeing a place for a queued slice. Returns \
                `archived` when it is gone, or `waiting_for_user` when the worktree has \
                uncommitted or unpushed work: the user is asked before anything is lost, and \
                their answer arrives later as a message (\"slice <name> archived\" or \"... \
                kept\"). Fails for an unknown or already archived name.",
            schema: || {
                json!({"type": "object", "properties": {
                    "name": {"type": "string", "description": "The slice's name, as given to `project_start_slice`."}
                }, "required": ["name"]})
            },
        },
        ToolDef {
            name: "tasks_archive",
            description: "Archive a finished task. Its live slices are archived first, with \
                `project_archive_slice`'s rules (unsaved work waits for the user), and its queued \
                slice starts are dropped. Returns `archived`, or `waiting_for_user` if a slice \
                still waits for the user.",
            schema: || {
                json!({"type": "object", "properties": {
                    "id": {"type": "integer", "description": "The task's id."}
                }, "required": ["id"]})
            },
        },
        ToolDef {
            name: "tasks_list",
            description: "Find tasks: one line each with id, status, bucket, priority, title \
                and live slice, ordered Now, Next, Unsorted, Later, then by priority. Archived \
                tasks are left out unless asked for. Use `tasks_get` for one task's notes and \
                decisions.",
            schema: || {
                json!({"type": "object", "properties": {
                    "bucket": {"type": "string", "enum": ["now", "next", "unsorted", "later"], "description": "Only tasks in this bucket."},
                    "status": {"type": "string", "enum": ["todo", "in_progress", "done", "archived"], "description": "Only tasks with this status."},
                    "include_archived": {"type": "boolean", "default": false, "description": "Also list archived tasks."}
                }})
            },
        },
        ToolDef {
            name: "tasks_get",
            description: "Read one task in full: status, bucket, priority, notes, \
                follow-up and related ids, its live slices, and its decision log oldest \
                first (text, why, rejected alternatives, what superseded what, user or agent). \
                Use it to brief a slice or hand a reviewer the decisions as spec.",
            schema: || {
                json!({"type": "object", "properties": {
                    "id": {"type": "integer", "description": "The task's id."}
                }, "required": ["id"]})
            },
        },
        ToolDef {
            name: "tasks_update",
            description: "Change a task's bucket, priority, title or notes; only the \
                fields you give change. Use it to re-sort and re-prioritise the list. Status \
                is not yours to set: Dovetail moves a task to in progress, review and done \
                from what its slice and pull request do. Moving a task \
                into Now is allowed; the user sees it flagged as new until they look at it. To archive a task use `tasks_archive`. Returns the task's new \
                `tasks_list` line. Fails for an unknown or archived task.",
            schema: || {
                json!({"type": "object", "properties": {
                    "id": {"type": "integer", "description": "The task's id."},
                    "bucket": {"type": "string", "enum": ["now", "next", "later", "unsorted"], "description": "New bucket; `unsorted` clears it."},
                    "p_level": {"type": ["string", "null"], "enum": ["P0", "P1", "P2", "P3", null], "description": "New priority, P0 highest; null clears it."},
                    "title": {"type": "string", "description": "New short imperative title."},
                    "notes": {"type": "string", "description": "New notes; they replace the old ones, so include what should stay."}
                }, "required": ["id"]})
            },
        },
        ToolDef {
            name: "coordination_report",
            description: "Report to the coordinator about your task. Use it instead of \
                SendMessage for reports: when something worth knowing happened (`progress`), \
                when your work is ready for review, normally with its pull request open \
                (`done`), and when you cannot go on without the coordinator (`blocked`). The \
                coordinator gets the report as a message and the user sees it as one line. \
                `done` moves your task to review.",
            schema: || {
                json!({"type": "object", "properties": {
                    "state": {"type": "string", "enum": ["progress", "done", "blocked"], "description": "Where the work stands."},
                    "summary": {"type": "string", "description": "What happened, in a line or two: what changed, what you ran, what you need."},
                    "pr": {"type": "integer", "description": "The pull request number, if the report is about one."}
                }, "required": ["state", "summary"]})
            },
        },
        ToolDef {
            name: "coordination_claim_criterion",
            description: "Say an acceptance criterion of an effort is met, with the evidence: \
                a pull request, a test that passes, a finding. The criterion becomes claimed \
                and waits in the user's inbox; only the user can approve it as met, or send it \
                back. Claim only what you can show. Fails for an unknown criterion, one already \
                met, or empty evidence.",
            schema: || {
                json!({"type": "object", "properties": {
                    "criterion": {"type": "integer", "description": "The criterion's id."},
                    "evidence": {"type": "string", "description": "What shows it is met, specific enough for the user to check: PR number, test name, finding id."}
                }, "required": ["criterion", "evidence"]})
            },
        },
        ToolDef {
            name: "maps_draw",
            description: "Draw a map the user can explore in Dovetail, to explain structure, \
                flow or architecture instead of drawing ASCII diagrams. A `structure` map shows \
                what exists and how it connects: nodes of nine kinds with plain-English \
                descriptions, groups to drill into, labelled connections, and guided flows. A \
                `swimlane` map shows what happens in order: one lane per participant and one \
                flow whose steps name their lane; a lane can link a node of a structure map. \
                Send the whole map each time. Every problem comes back at once as a list to \
                fix. On success it returns `{\"id\", \"version\"}`: call again with that `id` \
                to redraw the map as a new version; a new subject is a new map. Maps are kept \
                per project and linked to your task.",
            schema: dovetail_maps::input_schema,
        },
        ToolDef {
            name: "canvas_add_block",
            description: "Add a block to an effort's canvas, to lay out what you found in a \
                shape that shows the gaps. `question_tree`: a root question with sub-questions \
                and the findings that answer them. `options_compare`: options side by side, \
                each with its evidence below it. `free_area`: no structure. Dovetail puts the \
                block in empty space; nothing on the canvas moves. Pick the layout that fits \
                the question. `effort` defaults to the effort the user last tagged. Returns the \
                block's id, for `canvas_add_card`.",
            schema: || {
                json!({"type": "object", "properties": {
                    "layout": {"type": "string", "enum": ["question_tree", "options_compare", "free_area"], "description": "How the block lays out its cards."},
                    "title": {"type": "string", "description": "What the block is about: the root question, the choice being compared."},
                    "effort": {"type": "integer", "description": "The effort's id; defaults to the one the user last tagged."}
                }, "required": ["layout", "title"]})
            },
        },
        ToolDef {
            name: "canvas_add_card",
            description: "Add a card to an effort's canvas, so the user sees what you found or \
                thought. With `block`, Dovetail puts it in that block where there is room: \
                with `under`, below that card and linked from it (a question tree: the \
                question it answers; an options compare: the option it is evidence for). \
                Without `block` it goes to the incoming strip and the user places it. You \
                cannot move or delete a card. An `idea` or `question` carries its own `text`. \
                A `finding`, `source`, `decision`, `map` or `task` card points at something \
                that already exists: give its id as `reference` (a finding's or source's \
                number, a decision's number, a map's id, a task's number); the card shows its \
                title and nothing is copied. Fails for a reference that does not exist. \
                `effort` defaults to the block's effort, else the one the user last tagged. \
                Returns the card's id, for `under` and `canvas_link_cards`.",
            schema: || {
                json!({"type": "object", "properties": {
                    "kind": {"type": "string", "enum": ["idea", "question", "finding", "source", "decision", "map", "task"], "description": "What the card is."},
                    "text": {"type": "string", "description": "The words of an idea or question card; leave out for the other kinds."},
                    "reference": {"type": "string", "description": "The id of the finding, source, decision, map or task the card points at; leave out for an idea or question."},
                    "block": {"type": "integer", "description": "The block to put it in (from `canvas_add_block`); leave out for the incoming strip."},
                    "under": {"type": "integer", "description": "A card in that block to put it under and link it from."},
                    "effort": {"type": "integer", "description": "The effort's id; defaults to the block's, else the one the user last tagged."}
                }, "required": ["kind"]})
            },
        },
        ToolDef {
            name: "canvas_link_cards",
            description: "Link two cards of one effort, such as a question to the finding that \
                answers it. Linking them again changes the label. Cards are named by the ids \
                `canvas_add_card` returned.",
            schema: || {
                json!({"type": "object", "properties": {
                    "from": {"type": "integer", "description": "The card the link starts at (the question, the parent)."},
                    "to": {"type": "integer", "description": "The card it leads to."},
                    "label": {"type": "string", "description": "What the link says, if anything."}
                }, "required": ["from", "to"]})
            },
        },
        ToolDef {
            name: "design_declare_variants",
            description: "Declare what the user can switch in your running web preview, so \
                Dovetail's variant panel lists it. Send the whole list each time; it replaces \
                the last one. A slot is one thing to switch, with a `level`: `global` (the \
                whole page: a theme, a layout), `component` (one part: the header, the footer) \
                or `content` (one piece of content: a line of text), and its `options` (1 to 8 \
                strings). Name slots so they work as a JavaScript property: `theme`, \
                `header`, `hero_text`. Dovetail sends the user's picks to your page with \
                `postMessage` on load and on every change, to the page's window: \
                `{ dovetailVariants: { <slot name>: <option>, ... } }`, every declared slot \
                each time. You write the page code that applies it, for example \
                `addEventListener('message', e => { const v = e.data?.dovetailVariants; if (v) \
                for (const [k, o] of Object.entries(v)) document.documentElement.dataset[k] = \
                o; })`, and the CSS or components that react. Nothing else is added to your \
                page. The user mixes options freely, then presses \"Keep this combination\": \
                you then get a message naming the chosen options. Make them permanent, delete \
                the other options and the switching code, and call this tool again with what \
                is left (`slots: []` when nothing is). Slice only. Every problem with the list \
                comes back at once.",
            schema: || {
                json!({"type": "object", "properties": {
                    "slots": {"type": "array", "maxItems": 12, "description": "Every slot, in the order the panel shows them. Empty clears the list.", "items": {
                        "type": "object", "properties": {
                            "name": {"type": "string", "description": "The slot's key; unique; the page reads it."},
                            "level": {"type": "string", "enum": ["global", "component", "content"]},
                            "options": {"type": "array", "items": {"type": "string"}, "minItems": 1, "maxItems": 8, "description": "The choices; the page receives one of these strings."},
                            "kept": {"type": "string", "description": "The option already made permanent, if any; must be one of `options`."}
                        }, "required": ["name", "level", "options"]
                    }}
                }, "required": ["slots"]})
            },
        },
        ToolDef {
            name: "research_start_round",
            description: "Open the next round of a research task: a fan-out of subagents under \
                the user's criteria. Call it before you launch research subagents; while it is \
                open, Dovetail files each subagent's full report and the sources it actually \
                fetched or read, outside your context. Put the user's criteria in `brief`, \
                verbatim where you can. Set `kind` to `decision` when the user wants a recommendation \
                to choose by, `question` for a plain question. One round is open at a time: a \
                round requested while another is open is queued, and you are told when it \
                opens. `research_write_report` closes the open one. Returns \
                `{round, dir, status}` with status `open`, or `{round, status: \"queued\", \
                position}`: then wait for the message \"Round N for #T is now open\" before \
                you launch its subagents. Errors: unknown or archived task, an empty brief.",
            schema: || {
                json!({"type": "object", "properties": {
                    "task": {"type": "integer", "description": "Id of an existing task (the question)."},
                    "brief": {"type": "string", "description": "The user's criteria for this round, verbatim where possible."},
                    "kind": {"type": "string", "enum": ["decision", "question"], "description": "Whether the task makes a decision (report follows the decision template) or asks a question. Optional."}
                }, "required": ["task", "brief"]})
            },
        },
        ToolDef {
            name: "research_write_report",
            description: "Store the report of a research task and close its open round. Send the \
                whole report each time; it replaces the last one. Cite findings as [F3] and \
                sources as [S12], using the ids Dovetail shows; a citation that does not exist \
                refuses the write and the error lists the ids that do. Dovetail also flags \
                factual claims without a citation and key figures without a label (Sourced, \
                Derived or Estimate) in the result, and writes the \"Evidence: N sources, M \
                independent\" line itself. A task marked as a decision needs the template \
                sections (Key figures, Options, What would change this, Open questions). \
                Returns `{written, uncited, unlabelled_figures, citations_resolved}`.",
            schema: || {
                json!({"type": "object", "properties": {
                    "task": {"type": "integer", "description": "Id of the research task."},
                    "body": {"type": "string", "description": "The whole report in markdown, citing findings as [F3] and sources as [S12]. Rewrites the previous report."}
                }, "required": ["task", "body"]})
            },
        },
        ToolDef {
            name: "research_propose",
            description: "Propose an idea or a decision from research to the user. It never \
                writes anything itself: the proposal goes to the user's inbox, and the call \
                returns `pending` at once. If the user accepts, an idea becomes a follow-up \
                task in Unsorted (or a new round), and a decision is logged with its evidence; \
                either way the answer arrives later as a message. A `decision` needs \
                `evidence`: the finding and source ids it rests on, like [\"F3\", \"S12\"]. Use \
                this instead of settling a research question in prose or with `core_log_decision`.",
            schema: || {
                json!({"type": "object", "properties": {
                    "task": {"type": "integer", "description": "Id of the research task."},
                    "kind": {"type": "string", "enum": ["idea", "decision"]},
                    "text": {"type": "string", "description": "The idea, or the decision in one line."},
                    "evidence": {"type": "array", "items": {"type": "string"}, "description": "Finding and source ids, e.g. [\"F3\", \"S12\"]. Required for a decision."},
                    "why": {"type": "string"},
                    "alternatives": {"type": "array", "items": {
                        "type": "object", "properties": {
                            "option": {"type": "string"},
                            "why_not": {"type": "string"}
                        }, "required": ["option"]}}
                }, "required": ["task", "kind", "text"]})
            },
        },
        ToolDef {
            name: "research_get",
            description: "Read one stored research document, so you can cite it without keeping \
                it in context: the latest `report`, a round's brief (`round:N`), a `finding:N` \
                or a `source:N` (the numbers in [F3] and [S12]). Long documents are cut, with \
                the file's path at the end.",
            schema: || {
                json!({"type": "object", "properties": {
                    "task": {"type": "integer", "description": "Id of the research task."},
                    "what": {"type": "string", "description": "`report`, `round:N`, `finding:N` or `source:N`."}
                }, "required": ["task", "what"]})
            },
        },
    ]
}

#[cfg(test)]
mod tests {
    use dovetail_tools::{Role, TOOLS};

    use super::*;

    #[test]
    fn every_tool_has_one_definition_with_an_object_schema() {
        let defs = tool_defs();
        let mut names: Vec<&str> = defs.iter().map(|d| d.name).collect();
        let mut shared: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        names.sort_unstable();
        shared.sort_unstable();
        assert_eq!(
            names, shared,
            "tool_defs() must follow dovetail_tools::TOOLS"
        );
        for def in defs {
            assert_eq!((def.schema)()["type"], "object", "{}", def.name);
        }
    }

    #[test]
    fn tools_list_shows_the_suites_tools_then_their_old_names() {
        let names = |suites: &[Suite]| -> Vec<String> {
            tools_list(suites)
                .iter()
                .map(|t| t["name"].as_str().unwrap_or_default().to_owned())
                .collect()
        };
        assert_eq!(
            names(Role::SideChat.default_suites()),
            [
                "core_ask_user",
                "core_report_status",
                "core_log_decision",
                "ask_user",
                "report_status",
                "log_decision"
            ]
        );
        let slice = names(Role::Slice.default_suites());
        assert!(slice.contains(&"maps_draw".to_owned()) && slice.contains(&"draw_map".to_owned()));
        assert!(
            !slice
                .iter()
                .any(|n| n.contains("slice") || n.starts_with("tasks_"))
        );
        let coordinator = names(Role::Coordinator.default_suites());
        assert_eq!(coordinator.len(), TOOLS.len() + 12, "{coordinator:?}");
        let alias = tools_list(&[Suite::Project])
            .into_iter()
            .find(|t| t["name"] == "start_slice")
            .expect("the old name is listed");
        assert!(
            alias["description"]
                .as_str()
                .is_some_and(|d| d.contains("`project_start_slice`"))
        );
    }
}
