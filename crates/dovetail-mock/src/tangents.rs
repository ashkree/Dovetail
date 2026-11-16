//! The mock's tangents (ADR 0017): two tangents in Tangents, both new, each with an answer and
//! its sources, and the session of the first in the side panel, so the panel, the triage list
//! and reading mode's Tangents section can be tried with `dovetail --mock`.

use std::collections::BTreeMap;

use dovetail_api::research::{
    FindingId, FindingSummary, ResearchContent, ResearchDocument, ResearchListing, ResearchSummary,
    ResearchWhat, SourceId, SourceKind, SourceSummary,
};
use dovetail_api::{
    ProjectId, ProjectKind, ProjectSummary, TANGENTS_PROJECT, TangentSummary, Task, TaskId,
    TaskStatus, tangent_agent,
};

use crate::world::research_project_dir;

/// Tangents, the built-in research project, as the switcher lists it.
pub(crate) fn project() -> ProjectSummary {
    ProjectSummary {
        id: ProjectId(TANGENTS_PROJECT.into()),
        name: "Tangents".into(),
        repo_path: research_project_dir(TANGENTS_PROJECT),
        kind: ProjectKind::Research,
        ..ProjectSummary::default()
    }
}

/// The question of the seeded tangent whose session is in the side panel.
pub(crate) const QUESTION: &str = "How does kitty's graphics protocol differ from sixel?";

/// Its answer, citing the seeded sources.
pub(crate) const ANSWER: &str = "kitty sends images as base64 chunks with an id, so they can be \
placed, moved and deleted later without resending them [S1]. Sixel paints pixels as text \
cells and keeps no image to refer back to [S2].\n\nFor a map canvas that redraws often, \
kitty's placements are the cheaper choice.";

/// The second tangent's question.
const QUESTION_2: &str = "What hydration should a first sourdough loaf have?";

/// Its answer.
const ANSWER_2: &str = "Start around 70 % hydration: wet enough for an open crumb, firm \
enough to shape by hand [S3]. Go higher once shaping feels easy.";

/// The answer a tangent asked in the mock gets, whatever it asks.
pub(crate) const REPLY: &str = "Short answer: it depends on what you weigh most, and the \
sources disagree on the edges. This is the mock, so no page was fetched; a real tangent cites \
what it read as [S n].";

fn folder(task: TaskId, slug: &str) -> String {
    format!(
        "{}/research/{}-{slug}",
        research_project_dir(TANGENTS_PROJECT).display(),
        task.0
    )
}

fn task(id: i64, title: &str, notes: &str) -> Task {
    Task {
        id: TaskId(id),
        title: title.into(),
        status: TaskStatus::Todo,
        p_level: None,
        bucket: None,
        order: id,
        deps: vec![],
        notes: notes.into(),
        pr: None,
        follow_up_of: None,
        coordinator_created: false,
        related: vec![],
        effort: Default::default(),
        criterion: None,
    }
}

/// One tangent's content, for `LoadTangent`.
struct Content {
    listing: ResearchListing,
    documents: Vec<(ResearchWhat, ResearchDocument)>,
}

/// What the mock keeps about tangents' content.
pub(crate) struct Tangents {
    content: BTreeMap<TaskId, Content>,
    next_finding: i64,
}

impl Tangents {
    /// The answer to a `LoadTangent`.
    pub(crate) fn load(&self, tangent: TaskId, what: &ResearchWhat) -> Option<ResearchContent> {
        let content = self.content.get(&tangent)?;
        match what {
            ResearchWhat::Listing => Some(ResearchContent::Listing(content.listing.clone())),
            what => content
                .documents
                .iter()
                .find(|(w, _)| w == what)
                .map(|(_, d)| ResearchContent::Document(d.clone())),
        }
    }

    /// Files `body` as a new answer of `tangent`, as the core does when a turn ends; returns
    /// the tangent's research summary now.
    pub(crate) fn answer(&mut self, tangent: TaskId, body: &str, now_ms: i64) -> ResearchSummary {
        let id = FindingId(self.next_finding);
        self.next_finding += 1;
        let dir = folder(tangent, "tangent");
        let content = self.content.entry(tangent).or_insert_with(|| Content {
            listing: ResearchListing {
                findings: vec![],
                sources: vec![],
            },
            documents: vec![],
        });
        let n = content.listing.findings.len() as u32 + 1;
        let path = format!("{dir}/findings/{n:02}-answer.md");
        content.listing.findings.push(FindingSummary {
            id,
            round: None,
            n,
            agent: tangent_agent(tangent),
            label: "answer".into(),
            path: path.clone().into(),
            created_ms: now_ms,
            sources: vec![],
        });
        content.documents.push((
            ResearchWhat::Finding(id),
            ResearchDocument::text(Some(path.into()), format!("# Answer\n\n{body}\n")),
        ));
        summary(tangent, &dir, &content.listing)
    }
}

fn summary(tangent: TaskId, dir: &str, listing: &ResearchListing) -> ResearchSummary {
    ResearchSummary {
        task: tangent,
        dir: dir.into(),
        rounds: vec![],
        findings: listing.findings.len() as u32,
        quick_findings: listing.findings.len() as u32,
        sources: listing.sources.len() as u32,
        report_ms: None,
        kind: None,
    }
}

/// The seeded tangents for the snapshot, and their content.
pub(crate) fn seeded(now_ms: i64) -> (Vec<TangentSummary>, Tangents) {
    const HOUR: i64 = 3_600_000;
    let source = |id, kind, locator: &str, tool: &str, dir: &str| SourceSummary {
        id: SourceId(id),
        kind,
        locator: locator.into(),
        tool: tool.into(),
        path: (kind != SourceKind::Search).then(|| format!("{dir}/sources/{id}.md").into()),
        is_error: false,
        fetched_ms: now_ms - 20 * HOUR,
    };
    let one = TaskId(1);
    let two = TaskId(2);
    let dir_one = folder(one, "how-does-kitty-s-graphics-protocol");
    let dir_two = folder(two, "what-hydration-should-a-first");
    let finding = |id: i64, tangent: TaskId, dir: &str, sources: &[i64]| FindingSummary {
        id: FindingId(id),
        round: None,
        n: 1,
        agent: tangent_agent(tangent),
        label: "answer".into(),
        path: format!("{dir}/findings/01-answer.md").into(),
        created_ms: now_ms - 20 * HOUR,
        sources: sources.iter().map(|&s| SourceId(s)).collect(),
    };
    let listing_one = ResearchListing {
        findings: vec![finding(1, one, &dir_one, &[1, 2])],
        sources: vec![
            source(
                1,
                SourceKind::Web,
                "https://sw.kovidgoyal.net/kitty/graphics-protocol/",
                "WebFetch",
                &dir_one,
            ),
            source(
                2,
                SourceKind::Search,
                "sixel graphics terminal",
                "WebSearch",
                &dir_one,
            ),
        ],
    };
    let listing_two = ResearchListing {
        findings: vec![finding(2, two, &dir_two, &[3])],
        sources: vec![source(
            3,
            SourceKind::Web,
            "https://www.theperfectloaf.com/beginners-sourdough-bread/",
            "WebFetch",
            &dir_two,
        )],
    };
    let doc = |path: String, markdown: String| ResearchDocument::text(Some(path.into()), markdown);
    let content_one = Content {
        documents: vec![
            (
                ResearchWhat::Finding(FindingId(1)),
                doc(
                    format!("{dir_one}/findings/01-answer.md"),
                    format!("# Answer\n\nSources: S1, S2.\n\n{ANSWER}\n"),
                ),
            ),
            (
                ResearchWhat::Source(SourceId(1)),
                doc(
                    format!("{dir_one}/sources/1.md"),
                    "# Terminal graphics protocol\n\nThe goal of this specification is to \
                     create a flexible and performant protocol that allows the program running \
                     in the terminal to display arbitrary pixel data.\n"
                        .into(),
                ),
            ),
            (
                ResearchWhat::Source(SourceId(2)),
                ResearchDocument::text(
                    None,
                    "Search: sixel graphics terminal\n\n1. Sixel - Wikipedia\n".into(),
                ),
            ),
        ],
        listing: listing_one,
    };
    let content_two = Content {
        documents: vec![
            (
                ResearchWhat::Finding(FindingId(2)),
                doc(
                    format!("{dir_two}/findings/01-answer.md"),
                    format!("# Answer\n\nSources: S3.\n\n{ANSWER_2}\n"),
                ),
            ),
            (
                ResearchWhat::Source(SourceId(3)),
                doc(
                    format!("{dir_two}/sources/3.md"),
                    "# Beginner's sourdough bread\n\nThis recipe uses 70 % hydration.\n".into(),
                ),
            ),
        ],
        listing: listing_two,
    };
    let tangents = vec![
        TangentSummary {
            task: task(1, QUESTION, "Asked from Dovetail on 2026-11-10."),
            research: Some(summary(one, &dir_one, &content_one.listing)),
        },
        TangentSummary {
            task: task(2, QUESTION_2, "Asked from notes-site on 2026-11-10."),
            research: Some(summary(two, &dir_two, &content_two.listing)),
        },
    ];
    let store = Tangents {
        content: BTreeMap::from([(one, content_one), (two, content_two)]),
        next_finding: 3,
    };
    (tangents, store)
}
