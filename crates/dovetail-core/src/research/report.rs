//! Report validation (R2, `docs/research.md` §7): citations must resolve, claims and figures are
//! flagged when uncited or unlabelled, a decision report needs its sections, and Dovetail
//! rewrites the "Evidence: N sources, M independent" line itself.
//!
//! Pure text in, verdict out; the tool handler owns storage and files.

use std::collections::{BTreeMap, BTreeSet};

use dovetail_api::research::{FindingId, SourceId, SourceKind};

/// The headings a decision report must have (§7), matched case-insensitively at the start of a
/// `##` heading.
const DECISION_SECTIONS: [&str; 4] = [
    "Key figures",
    "Options",
    "What would change this",
    "Open questions",
];

/// A source as the checker needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KnownSource {
    pub(crate) kind: SourceKind,
    pub(crate) locator: String,
}

/// What a task has to cite.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Known {
    /// Findings with the sources each reached.
    pub(crate) findings: BTreeMap<FindingId, Vec<SourceId>>,
    /// Sources cited by the task's findings.
    pub(crate) sources: BTreeMap<SourceId, KnownSource>,
}

/// Why a report is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Refusal {
    /// A `[Fn]` or `[Sn]` names nothing in this task. One line per marker.
    #[error("{}", .0.join("\n"))]
    Unresolved(Vec<String>),
    /// A decision report lacks a required heading.
    #[error(
        "this task is a decision: the report needs the sections {}. See the template in \
         docs/research.md section 7",
        .0.iter().map(|s| format!("`## {s}`")).collect::<Vec<_>>().join(", ")
    )]
    MissingSections(Vec<&'static str>),
}

/// A report that passed: the body to store and what was flagged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Checked {
    /// The body with the evidence line rewritten.
    pub(crate) body: String,
    /// Factual paragraphs and table rows without a citation, shortened.
    pub(crate) uncited: Vec<String>,
    /// Key figures that lack a label, a citation or a calculation, one line each.
    pub(crate) unlabelled_figures: Vec<String>,
    /// Distinct findings and sources cited.
    pub(crate) citations_resolved: usize,
    /// Distinct sources behind the cited findings and sources (searches excluded).
    pub(crate) sources: usize,
    /// Of those, distinct registrable domains (files: distinct paths).
    pub(crate) independent: usize,
}

/// A `[F3]` or `[S12]` marker in a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Cite {
    Finding(FindingId),
    Source(SourceId),
}

/// The markers in `text`, in order, with repeats.
fn citations(text: &str) -> Vec<Cite> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' && i + 2 < bytes.len() && matches!(bytes[i + 1], b'F' | b'S') {
            let digits = bytes[i + 2..]
                .iter()
                .take_while(|b| b.is_ascii_digit())
                .count();
            let close = i + 2 + digits;
            if digits > 0 && bytes.get(close) == Some(&b']') {
                // Digits are ASCII, so the slice is valid UTF-8.
                if let Ok(id) = text[i + 2..close].parse::<i64>() {
                    out.push(if bytes[i + 1] == b'F' {
                        Cite::Finding(FindingId(id))
                    } else {
                        Cite::Source(SourceId(id))
                    });
                }
                i = close + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// `text` without its citation markers.
fn strip_citations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('[') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let marker = citations(&rest[..rest.find(']').map_or(rest.len(), |e| e + 1)]);
        match (marker.len(), rest.find(']')) {
            (1, Some(end)) => rest = &rest[end + 1..],
            _ => {
                out.push('[');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `F1..F4, F7` for the ids, `none` for no ids.
pub(crate) fn id_ranges(letter: char, ids: &[i64]) -> String {
    let mut ids = ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let mut parts = Vec::new();
    let mut i = 0;
    while i < ids.len() {
        let mut j = i;
        while j + 1 < ids.len() && ids[j + 1] == ids[j] + 1 {
            j += 1;
        }
        parts.push(match j - i {
            0 => format!("{letter}{}", ids[i]),
            1 => format!("{letter}{}, {letter}{}", ids[i], ids[j]),
            _ => format!("{letter}{}..{letter}{}", ids[i], ids[j]),
        });
        i = j + 1;
    }
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join(", ")
    }
}

/// Checks `body` against what the task has. `decision` says the task uses the decision
/// template.
pub(crate) fn check(body: &str, decision: bool, known: &Known) -> Result<Checked, Refusal> {
    let cited = citations(body);
    let mut unresolved = Vec::new();
    let mut seen = BTreeSet::new();
    for c in &cited {
        if !seen.insert(*c) {
            continue;
        }
        match c {
            Cite::Finding(id) if !known.findings.contains_key(id) => {
                let ids: Vec<i64> = known.findings.keys().map(|f| f.0).collect();
                unresolved.push(format!(
                    "[F{}] does not resolve; findings in this task: {}",
                    id.0,
                    id_ranges('F', &ids)
                ));
            }
            Cite::Source(id) if !known.sources.contains_key(id) => {
                let ids: Vec<i64> = known.sources.keys().map(|s| s.0).collect();
                unresolved.push(format!(
                    "[S{}] does not resolve; sources in this task: {}",
                    id.0,
                    id_ranges('S', &ids)
                ));
            }
            _ => {}
        }
    }
    if !unresolved.is_empty() {
        return Err(Refusal::Unresolved(unresolved));
    }
    if decision {
        let headings: Vec<String> = body
            .lines()
            .filter_map(|l| l.trim_start().strip_prefix("## "))
            .map(|h| h.trim().to_lowercase())
            .collect();
        let missing: Vec<&'static str> = DECISION_SECTIONS
            .into_iter()
            .filter(|s| !headings.iter().any(|h| h.starts_with(&s.to_lowercase())))
            .collect();
        if !missing.is_empty() {
            return Err(Refusal::MissingSections(missing));
        }
    }

    // Sources behind the citations: a cited finding stands for the sources it reached.
    let mut behind = BTreeSet::new();
    for c in &seen {
        match c {
            Cite::Finding(id) => behind.extend(known.findings[id].iter().copied()),
            Cite::Source(id) => {
                behind.insert(*id);
            }
        }
    }
    let evidence: Vec<&KnownSource> = behind
        .iter()
        .filter_map(|id| known.sources.get(id))
        .filter(|s| s.kind != SourceKind::Search)
        .collect();
    let independent: BTreeSet<String> = evidence.iter().map(|s| independence_key(s)).collect();
    let sources = evidence.len();
    let line = evidence_line(sources, independent.len());

    let (uncited, unlabelled_figures) = flags(body);
    Ok(Checked {
        body: with_evidence_line(body, &line, decision),
        uncited,
        unlabelled_figures,
        citations_resolved: seen.len(),
        sources,
        independent: independent.len(),
    })
}

fn evidence_line(sources: usize, independent: usize) -> String {
    let s = if sources == 1 { "source" } else { "sources" };
    format!("**Evidence:** {sources} {s}, {independent} independent")
}

/// Replaces every `**Evidence:**` line; a decision report without one gets it after the
/// confidence line (else before the first section).
fn with_evidence_line(body: &str, line: &str, decision: bool) -> String {
    let is_evidence = |l: &str| l.trim_start().starts_with("**Evidence:**");
    let mut out: Vec<String> = Vec::new();
    let mut found = false;
    for l in body.lines() {
        if is_evidence(l) {
            found = true;
            out.push(line.to_owned());
        } else {
            out.push(l.to_owned());
        }
    }
    if !found && decision {
        let at = out
            .iter()
            .position(|l| l.trim_start().starts_with("**Confidence:**"))
            .map(|i| i + 1)
            .or_else(|| out.iter().position(|l| l.trim_start().starts_with("## ")));
        match at {
            Some(i) if i > 0 && out[i - 1].trim().is_empty() => {
                out.insert(i - 1, line.to_owned());
            }
            Some(i) => {
                out.insert(i, line.to_owned());
                if out.get(i + 1).is_some_and(|l| l.starts_with("## ")) {
                    out.insert(i + 1, String::new());
                }
            }
            None => {
                out.push(String::new());
                out.push(line.to_owned());
            }
        }
    }
    let mut text = out.join("\n");
    if body.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// What makes two sources count as one: a web page's registrable domain, a file's path.
fn independence_key(source: &KnownSource) -> String {
    match source.kind {
        SourceKind::Web => registrable_domain(&source.locator),
        _ => source.locator.clone(),
    }
}

/// The registrable domain of a URL, without a public-suffix list: the last two labels of the
/// host, or the last three under a two-letter country code with a common second level
/// (`bbc.co.uk`). An IP address or a bare name stays as it is.
pub(crate) fn registrable_domain(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    let host = host.trim_end_matches('.').to_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() <= 2 || host.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return host;
    }
    let (second, last) = (labels[labels.len() - 2], labels[labels.len() - 1]);
    let take = if last.len() == 2
        && matches!(
            second,
            "co" | "com" | "org" | "net" | "gov" | "edu" | "ac" | "or" | "ne"
        ) {
        3
    } else {
        2
    };
    labels[labels.len() - take..].join(".")
}

/// Which section a body line is in, for the rules that skip some.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    /// Anything outside the sections below.
    Body,
    /// "Key figures": checked by the figure rules instead.
    Figures,
    /// "What would change this" and "Open questions": conditions and questions, not claims.
    Prospective,
}

fn section_of(heading: &str) -> Section {
    let h = heading.trim().to_lowercase();
    if h.starts_with("key figures") {
        Section::Figures
    } else if h.starts_with("what would change this") || h.starts_with("open questions") {
        Section::Prospective
    } else {
        Section::Body
    }
}

/// The uncited claims and the flawed key figures of a body.
fn flags(body: &str) -> (Vec<String>, Vec<String>) {
    let mut uncited = Vec::new();
    let mut figures = Vec::new();
    let mut section = Section::Body;
    let mut in_code = false;
    let mut paragraph: Vec<&str> = Vec::new();
    let mut table_row = 0usize;

    fn flush(paragraph: &mut Vec<&str>, section: Section, uncited: &mut Vec<String>) {
        if !paragraph.is_empty() && section == Section::Body {
            let text = paragraph.join(" ");
            if is_uncited_claim(&text) {
                uncited.push(excerpt(&text));
            }
        }
        paragraph.clear();
    }

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            flush(&mut paragraph, section, &mut uncited);
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix('#') {
            flush(&mut paragraph, section, &mut uncited);
            let heading = heading.trim_start_matches('#');
            if trimmed.starts_with("## ") {
                section = section_of(heading);
            }
            table_row = 0;
            continue;
        }
        if trimmed.is_empty() {
            flush(&mut paragraph, section, &mut uncited);
            table_row = 0;
            continue;
        }
        if trimmed.starts_with('|') {
            flush(&mut paragraph, section, &mut uncited);
            table_row += 1;
            let cells: Vec<&str> = trimmed
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect();
            let separator = cells
                .iter()
                .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')));
            if separator || table_row == 1 {
                continue;
            }
            match section {
                Section::Figures => {
                    if let Some(problem) = figure_problem(&cells) {
                        figures.push(problem);
                    }
                }
                Section::Body if is_uncited_claim(trimmed) => uncited.push(excerpt(trimmed)),
                _ => {}
            }
            continue;
        }
        let skipped = trimmed.starts_with('>')
            || ["**Recommendation:**", "**Confidence:**", "**Evidence:**"]
                .iter()
                .any(|p| trimmed.starts_with(p));
        if skipped {
            flush(&mut paragraph, section, &mut uncited);
            continue;
        }
        paragraph.push(trimmed);
    }
    flush(&mut paragraph, section, &mut uncited);
    (uncited, figures)
}

/// Why a Key figures row is flawed; `None` when it is sound.
fn figure_problem(cells: &[&str]) -> Option<String> {
    let name = cells.first().copied().unwrap_or("");
    if cells.len() < 3 {
        return Some(format!("{name}: expected Figure | Value | Basis"));
    }
    let basis = cells[2..].join(" | ");
    let plain = strip_citations(&basis).replace("**", "");
    let lower = plain.to_lowercase();
    let cited = !citations(&basis).is_empty();
    if lower.contains("sourced") {
        (!cited).then(|| format!("{name}: Sourced without a citation"))
    } else if lower.contains("derived") {
        let after = lower.split_once("derived").map_or("", |(_, rest)| rest);
        let calculates = after.contains(['÷', '×', '*', '/', '+', '=', '−'])
            || after.contains(" - ")
            || after.contains(" x ");
        if !cited {
            Some(format!("{name}: Derived without a citation"))
        } else if !calculates {
            Some(format!("{name}: Derived without a calculation"))
        } else {
            None
        }
    } else if lower.contains("estimate") {
        None
    } else {
        Some(format!("{name}: no label (Sourced, Derived or Estimate)"))
    }
}

/// Whether `text` has the shape of a factual claim (a number, "according to", or a named thing
/// in mid-sentence) and no citation.
fn is_uncited_claim(text: &str) -> bool {
    if !citations(text).is_empty() {
        return false;
    }
    let plain = text
        .trim_start_matches(['-', '*', '+', ' '])
        .replace(['*', '_', '`'], "");
    let lower = plain.to_lowercase();
    if plain.bytes().any(|b| b.is_ascii_digit()) || lower.contains("according to") {
        return true;
    }
    // A capitalised word that does not start a sentence is probably a name.
    let mut start = true;
    for word in plain.split_whitespace() {
        let bare = word.trim_matches(|c: char| !c.is_alphanumeric());
        let capital = bare.chars().next().is_some_and(char::is_uppercase)
            && bare.chars().count() > 1
            && bare.chars().skip(1).any(char::is_lowercase);
        if capital && !start {
            return true;
        }
        start = word.ends_with(['.', '?', '!', ':']);
    }
    false
}

/// The first 80 characters of a claim, for the tool result.
fn excerpt(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(80) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> Known {
        let mut k = Known::default();
        k.findings
            .insert(FindingId(1), vec![SourceId(1), SourceId(2)]);
        k.findings.insert(FindingId(2), vec![SourceId(3)]);
        k.findings.insert(FindingId(4), vec![]);
        for (id, kind, locator) in [
            (1, SourceKind::Web, "https://www.example.com/a"),
            (2, SourceKind::Web, "https://docs.example.com/b"),
            (3, SourceKind::Web, "https://news.bbc.co.uk/x"),
            (4, SourceKind::File, "/repo/notes.txt"),
            (5, SourceKind::Search, "market size"),
        ] {
            k.sources.insert(
                SourceId(id),
                KnownSource {
                    kind,
                    locator: locator.into(),
                },
            );
        }
        k
    }

    #[test]
    fn markers_are_found_and_stripped() {
        let c = citations("a [F3] b [S12][F3] c [x1] [F] [Fx] [F9");
        assert_eq!(
            c,
            [
                Cite::Finding(FindingId(3)),
                Cite::Source(SourceId(12)),
                Cite::Finding(FindingId(3)),
            ]
        );
        assert_eq!(strip_citations("a [F3] b [S12]c [x]"), "a  b c [x]");
    }

    #[test]
    fn an_unresolved_citation_refuses_with_the_ids_that_exist() {
        let e = check("Claim [F9] and [S7].", false, &known()).unwrap_err();
        let text = e.to_string();
        assert!(
            text.contains("[F9] does not resolve; findings in this task: F1, F2, F4"),
            "{text}"
        );
        assert!(
            text.contains("[S7] does not resolve; sources in this task: S1..S5"),
            "{text}"
        );
    }

    #[test]
    fn domains_are_registrable_and_files_are_paths() {
        assert_eq!(
            registrable_domain("https://www.example.com/a?b#c"),
            "example.com"
        );
        assert_eq!(
            registrable_domain("http://user@docs.example.com:8080/x"),
            "example.com"
        );
        assert_eq!(registrable_domain("https://news.bbc.co.uk/x"), "bbc.co.uk");
        assert_eq!(registrable_domain("http://127.0.0.1:3000/"), "127.0.0.1");
    }

    #[test]
    fn the_evidence_line_counts_cited_sources_and_domains() {
        // F1 reached S1 and S2 (one domain), S3 is bbc, S4 is a file, S5 is a search (ignored).
        let body =
            "# Q\n\n**Evidence:** 99 sources, 99 independent\n\nSee [F1], [S3], [S4], [S5].\n";
        let out = check(body, false, &known()).unwrap();
        assert_eq!((out.sources, out.independent), (4, 3));
        assert!(
            out.body.contains("**Evidence:** 4 sources, 3 independent"),
            "{}",
            out.body
        );
        assert!(!out.body.contains("99"));
        assert_eq!(out.citations_resolved, 4);
    }

    #[test]
    fn a_plain_report_without_the_line_does_not_get_one() {
        let out = check("Nothing cited here\n", false, &known()).unwrap();
        assert!(!out.body.contains("Evidence"));
    }

    const DECISION: &str = "# Which database?\n\n**Recommendation:** SQLite.\n**Confidence:** high — tested.\n\n## Key figures\n| Figure | Value | Basis |\n| --- | --- | --- |\n| Rows | 4.2B | **Sourced** [S1] |\n| Per row | $31 | **Derived**: 4.2B ÷ 135M [S1][S3] |\n| Churn | 6 % | **Estimate**: assumed |\n\n## Options\n### A\nFor: fast [F1]. Against: slow [F2].\n\n## What would change this\n- A new benchmark\n\n## Open questions\n- Who pays?\n";

    #[test]
    fn a_sound_decision_report_passes_and_gets_the_evidence_line() {
        let out = check(DECISION, true, &known()).unwrap();
        assert!(out.uncited.is_empty(), "{:?}", out.uncited);
        assert!(
            out.unlabelled_figures.is_empty(),
            "{:?}",
            out.unlabelled_figures
        );
        let lines: Vec<&str> = out.body.lines().collect();
        assert_eq!(lines[3], "**Confidence:** high — tested.");
        assert!(
            lines[4].starts_with("**Evidence:** 3 sources, 2 independent"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_decision_report_missing_a_section_is_refused() {
        let body = DECISION.replace("## Open questions", "## Questions");
        let e = check(&body, true, &known()).unwrap_err();
        assert_eq!(e, Refusal::MissingSections(vec!["Open questions"]));
        // The same body is fine for a plain question.
        assert!(check(&body, false, &known()).is_ok());
    }

    #[test]
    fn figures_without_label_citation_or_calculation_are_flagged() {
        let body = DECISION
            .replace(
                "| Churn | 6 % | **Estimate**: assumed |",
                "| Churn | 6 % | guess |",
            )
            .replace("**Sourced** [S1]", "**Sourced**")
            .replace(
                "**Derived**: 4.2B ÷ 135M [S1][S3]",
                "**Derived**: from the above [S1]",
            );
        let out = check(&body, true, &known()).unwrap();
        assert_eq!(
            out.unlabelled_figures,
            [
                "Rows: Sourced without a citation",
                "Per row: Derived without a calculation",
                "Churn: no label (Sourced, Derived or Estimate)",
            ]
        );
    }

    #[test]
    fn factual_paragraphs_and_rows_without_citation_are_flagged() {
        let body = "# Q\n\nThe market grew 12 % last year.\n\nIt is quiet [F1].\n\nA smooth sentence about nothing.\n\nAccording to the vendor it works.\n\n| Option | Note |\n| --- | --- |\n| A | costs 5 |\n| B | fine [F2] |\n\n## Open questions\n- Is 7 enough?\n";
        let out = check(body, false, &known()).unwrap();
        assert_eq!(
            out.uncited,
            [
                "The market grew 12 % last year.",
                "According to the vendor it works.",
                "| A | costs 5 |",
            ]
        );
    }
}
