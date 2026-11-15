//! What the mock answers to `<leader>fr` (ADR 0018, decided 2026-11-11 later): the library of
//! another registered project, read-only. `Market sizing` (research) has one research task,
//! numbered #3 like the open project's fuzzy-matcher research so a mix-up of the two shows;
//! `notes-site` (code) has no research and one doc.

use std::path::Path;

use dovetail_api::research::{
    BrowseWhat, BrowsedListing, BrowsedProject, DocEntry, Evidence, FindingId, FindingSummary,
    ResearchDocument, ResearchListing, ResearchSummary, ResearchWhat, RoundSummary, SourceId,
    SourceKind, SourceSummary,
};
use dovetail_api::{
    Decision, DecisionAuthor, DecisionId, ProjectSummary, Task, TaskId, TaskStatus,
};

use crate::world::agent_id;

const DIR: &str = "/home/you/.local/share/dovetail/projects/market-sizing/research/3-eu-market";

/// The browsed research task's report.
pub(crate) const REPORT: &str = "# How big is the EU market?\n\n\
    **Answer:** about 40k teams [F1][S1]. Confidence: low.\n";

const NOTES_DOC: &str = "# Notes site\n\nA static site of notes.\n";

/// The library of `project`, as the mock knows it; `None` for a project it has nothing for.
pub(crate) fn project(project: &ProjectSummary, now_ms: i64) -> Option<BrowsedProject> {
    match project.id.0.as_str() {
        "market-sizing" => Some(market_sizing(project.clone(), now_ms)),
        "notes-site" => Some(BrowsedProject {
            project: project.clone(),
            tasks: Vec::new(),
            decisions: Vec::new(),
            research: Vec::new(),
            listings: Vec::new(),
            docs: vec![DocEntry {
                path: "intro.md".into(),
                title: "Notes site".into(),
            }],
        }),
        _ => None,
    }
}

/// One document of a browsed project; `None` when the mock has no such thing.
pub(crate) fn load(project: &str, what: &BrowseWhat) -> Option<ResearchDocument> {
    match (project, what) {
        (
            "market-sizing",
            BrowseWhat::Research {
                task: TaskId(3),
                what,
            },
        ) => {
            let text = match what {
                ResearchWhat::Report => REPORT,
                ResearchWhat::Finding(FindingId(1)) => "# Team counts\n\nSources: S1.\n",
                ResearchWhat::Source(SourceId(1)) => "# Eurostat\n\nTable of firms.\n",
                ResearchWhat::Brief { round: 1 } => "# Round 1\n\nCount the teams.\n",
                _ => return None,
            };
            Some(ResearchDocument::text(None, text.into()))
        }
        ("notes-site", BrowseWhat::Doc(path)) if path == Path::new("intro.md") => {
            Some(ResearchDocument::text(None, NOTES_DOC.into()))
        }
        _ => None,
    }
}

fn market_sizing(project: ProjectSummary, now_ms: i64) -> BrowsedProject {
    let task = TaskId(3);
    BrowsedProject {
        project,
        tasks: vec![Task {
            id: task,
            title: "EU market size".into(),
            status: TaskStatus::InProgress,
            p_level: None,
            bucket: None,
            order: 0,
            deps: vec![],
            notes: String::new(),
            pr: None,
            follow_up_of: None,
            coordinator_created: false,
            related: vec![],
            effort: Default::default(),
            criterion: None,
        }],
        decisions: vec![Decision {
            id: DecisionId(1),
            task,
            at_ms: now_ms - 600_000,
            by: DecisionAuthor::User,
            text: "Size by teams, not seats".into(),
            why: Some("Teams buy.".into()),
            alternatives: vec![],
            supersedes: None,
            evidence: vec![Evidence::Finding(FindingId(1))],
        }],
        research: vec![ResearchSummary {
            task,
            dir: DIR.into(),
            rounds: vec![RoundSummary {
                id: 1,
                n: 1,
                brief: "Count the teams.".into(),
                dir: "rounds/01-2026-11-01".into(),
                opened_ms: now_ms - 86_400_000,
                closed_ms: Some(now_ms - 80_000_000),
                findings: 1,
                queued: false,
            }],
            findings: 1,
            quick_findings: 0,
            sources: 1,
            report_ms: Some(now_ms - 70_000_000),
            kind: None,
        }],
        listings: vec![BrowsedListing {
            task,
            listing: ResearchListing {
                findings: vec![FindingSummary {
                    id: FindingId(1),
                    round: Some(1),
                    n: 1,
                    agent: agent_id("explore"),
                    label: "team-counts".into(),
                    path: format!("{DIR}/rounds/01-2026-11-01/findings/01-team-counts.md").into(),
                    created_ms: now_ms - 80_000_000,
                    sources: vec![SourceId(1)],
                }],
                sources: vec![SourceSummary {
                    id: SourceId(1),
                    kind: SourceKind::Web,
                    locator: "https://ec.europa.eu/eurostat".into(),
                    tool: "WebFetch".into(),
                    path: None,
                    is_error: false,
                    fetched_ms: now_ms - 80_000_000,
                }],
            },
        }],
        docs: Vec::new(),
    }
}
