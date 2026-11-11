//! The mock's research (R3): one task (#3, the fuzzy picker) with a closed round, an open round,
//! three findings (one quick), three sources and a report, so the research views in the task
//! detail can be tried with `dovetail --mock` and drive the TUI tests.

use std::path::Path;

use dovetail_api::TaskId;
use dovetail_api::research::{
    DocEntry, FindingId, FindingSummary, ResearchContent, ResearchDocument, ResearchListing,
    ResearchSummary, ResearchWhat, RoundSummary, SourceId, SourceKind, SourceSummary,
};

use crate::world::agent_id;

/// The task the mock has research for.
pub(crate) const TASK: i64 = 3;

const DIR: &str = "/home/you/.local/share/dovetail/projects/notes-site/research/3-fuzzy-matcher";

/// What the mock keeps: the summary the snapshot carries and the content answers.
pub(crate) struct Research {
    /// Goes into the snapshot.
    pub summary: ResearchSummary,
    listing: ResearchListing,
    documents: Vec<(ResearchWhat, ResearchDocument)>,
}

impl Research {
    /// The answer to a [`dovetail_api::Command::LoadResearch`]; `None` when the mock has no such
    /// research.
    pub(crate) fn load(&self, task: TaskId, what: &ResearchWhat) -> Option<ResearchContent> {
        if task != TaskId(TASK) {
            return None;
        }
        match what {
            ResearchWhat::Listing => Some(ResearchContent::Listing(self.listing.clone())),
            what => self
                .documents
                .iter()
                .find(|(w, _)| w == what)
                .map(|(_, d)| ResearchContent::Document(d.clone())),
        }
    }
}

/// The repo docs reading mode's library lists for the mock's project (ADR 0018).
const DOCS: &[(&str, &str, &str)] = &[
    (
        "research.md",
        "Research sessions",
        "# Research sessions\n\nRounds, findings and sources are captured from the stream; the \
         coordinator writes a cited report with `research_write_report`.\n\n## Reading mode\n\n\
         `<leader>r` opens the library, the reading column and Ask.\n",
    ),
    (
        "adr/0018-reading-mode-layout.md",
        "Reading mode: a second layout preset",
        "# Reading mode: a second layout preset\n\nReading mode is a full-screen layout with \
         three columns: Library, Reading and Ask. Layout B stays the default.\n\n\
         ![The three columns](reading-mode.png)\n",
    ),
];

/// The mock's repo docs, as `Command::ListDocs` answers.
pub(crate) fn docs() -> Vec<DocEntry> {
    DOCS.iter()
        .map(|(path, title, _)| DocEntry {
            path: (*path).into(),
            title: (*title).into(),
        })
        .collect()
}

/// One of [`docs`], as `Command::LoadDoc` answers.
pub(crate) fn doc(path: &Path) -> Option<ResearchDocument> {
    DOCS.iter()
        .find(|(p, _, _)| Path::new(p) == path)
        .map(|(p, _, text)| {
            ResearchDocument::text(
                Some(format!("/home/you/code/notes-site/docs/{p}").into()),
                (*text).into(),
            )
        })
}

fn document(path: Option<&str>, markdown: &str) -> ResearchDocument {
    ResearchDocument::text(path.map(Into::into), markdown.into())
}

/// The seeded research, relative to the mock's start time.
pub(crate) fn seeded(now_ms: i64) -> Research {
    const DAY: i64 = 86_400_000;
    let finding = |id, round, n, label: &str, sources: &[i64]| FindingSummary {
        id: FindingId(id),
        round,
        n,
        agent: agent_id("explore"),
        label: label.into(),
        path: match round {
            Some(r) => format!("{DIR}/rounds/{r:02}-2026-10-30/findings/{n:02}-{label}.md"),
            None => format!("{DIR}/findings/{n:02}-{label}.md"),
        }
        .into(),
        created_ms: now_ms - DAY,
        sources: sources.iter().map(|&s| SourceId(s)).collect(),
    };
    let source = |id, kind, locator: &str, tool: &str, cached: bool| SourceSummary {
        id: SourceId(id),
        kind,
        locator: locator.into(),
        tool: tool.into(),
        path: cached.then(|| format!("{DIR}/sources/{id}.md").into()),
        is_error: false,
        fetched_ms: now_ms - DAY,
    };
    let summary = ResearchSummary {
        task: TaskId(TASK),
        dir: DIR.into(),
        rounds: vec![
            RoundSummary {
                id: 1,
                queued: false,
                n: 1,
                brief: "Which fuzzy matcher should the picker use? Weigh speed on a few \
                        thousand rows, match quality on paths, and maintenance."
                    .into(),
                dir: "rounds/01-2026-10-30".into(),
                opened_ms: now_ms - 2 * DAY,
                closed_ms: Some(now_ms - DAY),
                findings: 2,
            },
            RoundSummary {
                id: 2,
                queued: false,
                n: 2,
                brief: "Does nucleo handle non-ASCII paths well?".into(),
                dir: "rounds/02-2026-10-31".into(),
                opened_ms: now_ms - DAY / 2,
                closed_ms: None,
                findings: 0,
            },
        ],
        findings: 3,
        quick_findings: 1,
        sources: 3,
        report_ms: Some(now_ms - DAY / 4),
        kind: None,
    };
    let listing = ResearchListing {
        findings: vec![
            finding(1, Some(1), 1, "nucleo", &[1, 2]),
            finding(2, Some(1), 2, "skim", &[3]),
            finding(3, None, 1, "scoring-notes", &[2]),
        ],
        sources: vec![
            source(
                1,
                SourceKind::Web,
                "https://docs.rs/nucleo",
                "WebFetch",
                true,
            ),
            source(
                2,
                SourceKind::Web,
                "https://github.com/helix-editor/nucleo",
                "WebFetch",
                true,
            ),
            source(
                3,
                SourceKind::Search,
                "skim fuzzy matcher rust",
                "WebSearch",
                false,
            ),
        ],
    };
    let documents = vec![
        (
            ResearchWhat::Report,
            document(
                Some(&format!("{DIR}/report.md")),
                "# Which fuzzy matcher for the picker?\n\n\
                 **Recommendation:** use `nucleo` [F1][S1]. Confidence: medium.\n\n\
                 ## Evidence\n\n\
                 - nucleo scores a few thousand rows well inside a frame, and is what Helix \
                 uses [F1, S2].\n\
                 - skim is slower to set up and pulls in a larger dependency tree [F2].\n\
                 - Scoring on paths wants a bonus for matches after `/` [F3].\n\n\
                 ## What would change this\n\n\
                 If non-ASCII paths score badly in round 2 [S2], revisit.\n",
            ),
        ),
        (
            ResearchWhat::Brief { round: 1 },
            document(
                Some(&format!("{DIR}/rounds/01-2026-10-30/brief.md")),
                "# Round 1\n\nWhich fuzzy matcher should the picker use? Weigh speed on a few \
                 thousand rows, match quality on paths, and maintenance.\n",
            ),
        ),
        (
            ResearchWhat::Brief { round: 2 },
            document(
                Some(&format!("{DIR}/rounds/02-2026-10-31/brief.md")),
                "# Round 2\n\nDoes nucleo handle non-ASCII paths well?\n",
            ),
        ),
        (
            ResearchWhat::Finding(FindingId(1)),
            document(
                Some(&listing.findings[0].path.to_string_lossy()),
                "# nucleo\n\nSources: S1, S2.\n\nnucleo is a port of fzf's algorithm with a \
                 parallel matcher; the docs show it matching 100k items in milliseconds [S1]. \
                 It is maintained by the Helix authors [S2].\n",
            ),
        ),
        (
            ResearchWhat::Finding(FindingId(2)),
            document(
                Some(&listing.findings[1].path.to_string_lossy()),
                "# skim\n\nSources: S3.\n\nskim is a full fuzzy finder; the library part is \
                 usable but heavier [S3].\n",
            ),
        ),
        (
            ResearchWhat::Finding(FindingId(3)),
            document(
                Some(&listing.findings[2].path.to_string_lossy()),
                "# Scoring notes\n\nSources: S2.\n\nBoth libraries reward matches at path \
                 separators [S2].\n",
            ),
        ),
        (
            ResearchWhat::Source(SourceId(1)),
            document(
                Some(&format!("{DIR}/sources/1.md")),
                "<!DOCTYPE html><html><head><title>nucleo - Rust</title><style>body{}</style>\
                 </head><body><h1>Crate nucleo</h1><p><strong>nucleo</strong> is a fast fuzzy \
                 matcher. See <a href=\"https://docs.rs/nucleo/latest/nucleo/struct.Matcher.html\">\
                 <code>Matcher</code></a>.</p><h2>Types</h2><ul><li><code>Matcher</code></li>\
                 <li><code>Pattern</code></li><li><code>Nucleo</code> (the threaded one)</li></ul>\
                 <script>track()</script></body></html>\n",
            ),
        ),
        (
            ResearchWhat::Source(SourceId(2)),
            document(
                Some(&format!("{DIR}/sources/2.md")),
                "# helix-editor/nucleo\n\nMatching algorithm based on fzf v2, with a few \
                 changes for speed. MPL-2.0.\n",
            ),
        ),
        (
            ResearchWhat::Source(SourceId(3)),
            document(
                None,
                "Search: skim fuzzy matcher rust\n\n1. skim-rs/skim: fuzzy finder in Rust\n",
            ),
        ),
    ];
    Research {
        summary,
        listing,
        documents,
    }
}
