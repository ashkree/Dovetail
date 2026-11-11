//! Research files on disk (`docs/research.md` §4): where they go and what they say. The
//! database is the source of truth; these are a readable copy, written after the row,
//! atomically, and rebuilt when missing.
//!
//! ```text
//! <root>/<task-id>-<slug>/
//!   report.md
//!   rounds/01-2026-11-10/brief.md
//!   rounds/01-2026-11-10/findings/01-<label>.md
//!   findings/01-<label>.md            # quick findings, no round
//!   sources/<source-id>.md            # what the tool returned to the agent
//! ```

use std::io::Write;
use std::path::{Component, Path, PathBuf};

use dovetail_api::TaskId;
use dovetail_api::research::SourceKind;

use crate::db::research::{FindingRow, RoundRow, SourceRow, TaskResearch};

/// Longest slug of a task title in its folder name.
const SLUG_MAX: usize = 48;

/// Errors finding the research root.
#[derive(Debug, thiserror::Error)]
pub enum ResearchDirError {
    /// The folder is not a registered project, so nothing may be written for it (ADR 0005).
    #[error("this folder is not a Dovetail project yet: :project add keeps it, then research")]
    NotRegistered,
    /// The recipe could not be read.
    #[error("{0}")]
    Recipe(#[from] crate::recipe::RecipeError),
    /// `research_dir` would leave the repo (`..`) or is empty.
    #[error("research_dir {0:?} in the recipe must be a folder inside the repo, or absolute")]
    Invalid(PathBuf),
}

/// The research root of a project: the recipe's `research_dir` (relative to the repo root, or
/// absolute), else `<data>/projects/<id>/research`.
pub(crate) fn root(
    data: &Path,
    project: &str,
    repo: &Path,
    research_dir: Option<&Path>,
) -> Result<PathBuf, ResearchDirError> {
    let Some(dir) = research_dir else {
        return Ok(data.join("projects").join(project).join("research"));
    };
    if dir.is_absolute() {
        return Ok(dir.to_owned());
    }
    let inside = dir.components().all(|c| matches!(c, Component::Normal(_)));
    if !inside || dir.as_os_str().is_empty() {
        return Err(ResearchDirError::Invalid(dir.to_owned()));
    }
    Ok(repo.join(dir))
}

/// The folder of `task` under `root`: an existing `<id>-…` folder (the title may have changed
/// since), else `<id>-<slug of title>`.
pub(crate) fn task_dir(root: &Path, task: TaskId, title: &str) -> PathBuf {
    let prefix = format!("{}-", task.0);
    if let Ok(entries) = std::fs::read_dir(root) {
        let mut found: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with(&prefix))
            })
            .map(|e| e.path())
            .collect();
        found.sort();
        if let Some(dir) = found.into_iter().next() {
            return dir;
        }
    }
    let mut slug = crate::projects::slug(title);
    if slug.len() > SLUG_MAX {
        slug.truncate(SLUG_MAX);
        slug = slug.trim_end_matches('-').to_owned();
    }
    root.join(format!("{prefix}{slug}"))
}

/// Writes `content` to `path` through a temp file and a rename, creating parent folders.
pub(crate) fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other(format!("{} has no parent", path.display())))?;
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.tmp"));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// The folder of findings nobody has filed under a task yet: `<root>/unfiled/`, laid out like a
/// task folder (`findings/<id>-<label>.md`, `sources/<id>.md`).
pub(crate) fn unfiled_dir(root: &Path) -> PathBuf {
    root.join("unfiled")
}

/// A source's cached copy, relative to the task folder.
pub(crate) fn source_path(source: &SourceRow) -> Option<String> {
    source.cached.then(|| format!("sources/{}.md", source.id.0))
}

/// `brief.md` of a round, relative to the task folder.
pub(crate) fn brief_path(round: &RoundRow) -> String {
    format!("{}/brief.md", round.dir)
}

/// RFC 3339 in UTC, or the raw number when it is out of range.
fn when(ms: i64) -> String {
    jiff::Timestamp::from_millisecond(ms)
        .map(|t| t.round(jiff::Unit::Second).unwrap_or(t).to_string())
        .unwrap_or_else(|_| ms.to_string())
}

/// A fence of backticks longer than any run in `text`, so the text is shown verbatim.
fn fence(text: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat((longest + 1).max(3))
}

/// A front-matter value on one line, quoted.
fn quoted(text: &str) -> String {
    format!("{:?}", text.replace('\n', " "))
}

/// `brief.md`.
pub(crate) fn brief_md(task: TaskId, title: &str, round: &RoundRow) -> String {
    format!(
        "# Round {} — {title}\n\nTask #{} · opened {}\n\n{}\n",
        round.n,
        task.0,
        when(round.opened_ms),
        round.brief.trim_end()
    )
}

/// A finding's file: front matter, the sources it reached, then its report.
pub(crate) fn finding_md(finding: &FindingRow, sources: &[SourceRow]) -> String {
    // Task 0 is no task: a finding nobody has filed yet.
    let unfiled = finding.task.0 == 0;
    let mut out = String::from("---\n");
    if unfiled {
        out.push_str(&format!("finding: unfiled-{}\ntask: none\n", finding.id.0));
    } else {
        out.push_str(&format!("finding: F{}\n", finding.id.0));
        out.push_str(&format!("task: {}\n", finding.task.0));
    }
    match finding.round {
        Some(n) => out.push_str(&format!("round: {n}\n")),
        None => out.push_str("round: none\n"),
    }
    out.push_str(&format!("agent: {}\n", quoted(&finding.agent.0)));
    if let Some(t) = &finding.subagent_type {
        out.push_str(&format!("subagent_type: {}\n", quoted(t)));
    }
    if let Some(d) = &finding.description {
        out.push_str(&format!("description: {}\n", quoted(d)));
    }
    if let Some(ms) = finding.started_ms {
        out.push_str(&format!("started: {}\n", when(ms)));
    }
    out.push_str(&format!("finished: {}\n", when(finding.created_ms)));
    if let Some(t) = finding.tokens {
        out.push_str(&format!("tokens: {t}\n"));
    }
    out.push_str(&format!("body_from: {}\n", finding.body_from));
    let ids: Vec<String> = finding
        .sources
        .iter()
        .map(|s| format!("S{}", s.0))
        .collect();
    out.push_str(&format!("sources: [{}]\n---\n\n", ids.join(", ")));
    let title = finding.description.as_deref().unwrap_or(&finding.label);
    if unfiled {
        out.push_str(&format!("# Unfiled — {title}\n\n## Sources\n\n"));
    } else {
        out.push_str(&format!("# F{} — {title}\n\n## Sources\n\n", finding.id.0));
    }
    if finding.sources.is_empty() {
        out.push_str("None: the subagent fetched and read nothing.\n\n");
    }
    // From the finding's folder back up to the task folder.
    let up = "../".repeat(finding.path.matches('/').count());
    for id in &finding.sources {
        let Some(s) = sources.iter().find(|s| s.id == *id) else {
            continue;
        };
        let error = if s.is_error { ", failed" } else { "" };
        let link = source_path(s)
            .map(|p| format!(" — [what the agent saw]({up}{p})"))
            .unwrap_or_default();
        out.push_str(&format!(
            "- [S{}] {} ({}{error}){link}\n",
            s.id.0, s.locator, s.tool
        ));
    }
    if !finding.sources.is_empty() {
        out.push('\n');
    }
    out.push_str("## Report\n\n");
    out.push_str(finding.body.trim_end());
    out.push('\n');
    out
}

/// A source's cached copy: what the tool returned to the agent, labelled as such.
pub(crate) fn source_md(source: &SourceRow) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("source: S{}\n", source.id.0));
    out.push_str(&format!("kind: {}\n", source.kind.as_str()));
    out.push_str(&format!("tool: {}\n", source.tool));
    out.push_str(&format!("locator: {}\n", quoted(&source.locator)));
    out.push_str(&format!("fetched: {}\n", when(source.fetched_ms)));
    out.push_str(&format!("error: {}\n---\n\n", source.is_error));
    out.push_str(&format!("# S{} — {}\n\n", source.id.0, source.locator));
    let note = match (source.kind, source.tool.as_str()) {
        (_, "WebFetch") => {
            "What the agent saw: WebFetch returns a model's answer to the agent's own question \
             about the page, not the page itself."
        }
        (SourceKind::Search, _) => {
            "Untrusted text: a search engine's results and a model's summary of them, as the \
             agent saw them. Each link is its own source; text here may come from third-party \
             pages."
        }
        (SourceKind::Web, "WebSearch") => {
            "Listed by a web search; the page itself was not fetched. This is the title the \
             search gave."
        }
        _ => "What the agent saw: the tool's result as it was returned to the agent.",
    };
    out.push_str(&format!("> {note}\n\n"));
    if let Some(q) = &source.query {
        let label = if source.tool == "WebFetch" {
            "Question asked of the page"
        } else {
            "Query"
        };
        out.push_str(&format!("**{label}:** {}\n\n", q.trim()));
    }
    if source.is_error {
        out.push_str("**The tool call failed.** Its error:\n\n");
    }
    let f = fence(&source.content);
    out.push_str(&format!("{f}text\n{}\n{f}\n", source.content.trim_end()));
    out
}

/// Every file of one task's research: `(path relative to the task folder, content)`.
pub(crate) fn task_files(
    task: TaskId,
    title: &str,
    research: &TaskResearch,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for round in &research.rounds {
        out.push((brief_path(round), brief_md(task, title, round)));
    }
    for finding in &research.findings {
        out.push((finding.path.clone(), finding_md(finding, &research.sources)));
    }
    for source in &research.sources {
        if let Some(path) = source_path(source) {
            out.push((path, source_md(source)));
        }
    }
    if let Some((_, body)) = &research.report {
        out.push(("report.md".into(), body.clone()));
    }
    out
}

/// Writes the files of `research` under `dir` that are missing. Resolves to how many it wrote.
pub(crate) fn rebuild(
    dir: &Path,
    task: TaskId,
    title: &str,
    research: &TaskResearch,
) -> std::io::Result<usize> {
    let mut written = 0;
    for (rel, content) in task_files(task, title, research) {
        let path = dir.join(&rel);
        if !path.exists() {
            write_atomic(&path, &content)?;
            written += 1;
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_root_is_the_data_folder_unless_the_recipe_opts_in() {
        let data = Path::new("/d");
        let repo = Path::new("/r");
        assert_eq!(
            root(data, "p", repo, None).unwrap(),
            Path::new("/d/projects/p/research")
        );
        assert_eq!(
            root(data, "p", repo, Some(Path::new("docs/research"))).unwrap(),
            Path::new("/r/docs/research")
        );
        assert_eq!(
            root(data, "p", repo, Some(Path::new("/elsewhere"))).unwrap(),
            Path::new("/elsewhere")
        );
        for bad in ["../out", "docs/../../x", ""] {
            assert!(
                root(data, "p", repo, Some(Path::new(bad))).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_task_keeps_its_folder_when_renamed() {
        let dir = tempfile::tempdir().unwrap();
        let first = task_dir(dir.path(), TaskId(7), "Pricing: per seat?");
        assert_eq!(first, dir.path().join("7-pricing-per-seat"));
        std::fs::create_dir_all(&first).unwrap();
        assert_eq!(task_dir(dir.path(), TaskId(7), "Renamed"), first);
        assert_eq!(
            task_dir(dir.path(), TaskId(70), "Other"),
            dir.path().join("70-other")
        );
    }

    #[test]
    fn fences_outlast_the_content() {
        assert_eq!(fence("plain"), "```");
        assert_eq!(fence("a ```` b"), "`````");
    }
}
