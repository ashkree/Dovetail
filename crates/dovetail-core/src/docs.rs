//! Reading mode's read-only documents (ADR 0018): the repo's `docs/` markdown for the library's
//! Docs · ADRs, and the local PNG images a document shows, measured so the UI can place them
//! with kitty graphics without reading any file itself.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use dovetail_api::research::{DocEntry, DocImage, ResearchDocument};

/// The repo folder the docs are read from.
pub(crate) const DOCS_DIR: &str = "docs";
/// Most docs listed; a repo with more lists the first ones by path.
const MAX_DOCS: usize = 500;
/// Folder depth walked under `docs/`.
const MAX_DEPTH: usize = 4;
/// Largest doc read, in bytes.
const MAX_DOC_BYTES: u64 = 2 * 1024 * 1024;
/// Most images measured in one document.
const MAX_IMAGES: usize = 32;

/// Why a doc could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DocError {
    /// The path leaves the docs folder, or is not a markdown file in it.
    #[error("{0} is not one of the project's docs")]
    Outside(String),
    /// Bigger than [`MAX_DOC_BYTES`].
    #[error("{0} is too large to read here")]
    TooLarge(String),
    /// The file could not be read.
    #[error("could not read {path}: {source}")]
    Io {
        /// The doc.
        path: String,
        /// Why.
        source: std::io::Error,
    },
}

/// The markdown files under `<repo>/docs/`, sorted by path. Symbolic links are not followed,
/// so nothing outside the folder is listed.
pub(crate) fn list(repo: &Path) -> Vec<DocEntry> {
    let root = repo.join(DOCS_DIR);
    let mut found = Vec::new();
    walk(&root, &root, 0, &mut found);
    found.sort();
    found.truncate(MAX_DOCS);
    found
        .into_iter()
        .map(|rel| {
            let title = title_of(&root.join(&rel)).unwrap_or_else(|| {
                rel.file_name()
                    .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
            });
            DocEntry { path: rel, title }
        })
        .collect()
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH || out.len() >= MAX_DOCS * 2 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            walk(root, &path, depth + 1, out);
        } else if kind.is_file()
            && path.extension().is_some_and(|e| e == "md")
            && let Ok(rel) = path.strip_prefix(root)
        {
            out.push(rel.to_path_buf());
        }
    }
}

/// The first `# ` heading in the first lines of a file.
fn title_of(path: &Path) -> Option<String> {
    let mut head = String::new();
    fs::File::open(path)
        .ok()?
        .take(8 * 1024)
        .read_to_string(&mut head)
        .ok()?;
    head.lines()
        .take(40)
        .find_map(|l| l.strip_prefix("# "))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// Reads one doc, `rel` as [`list`] named it.
pub(crate) fn load(repo: &Path, rel: &Path) -> Result<ResearchDocument, DocError> {
    let shown = rel.display().to_string();
    let plain = rel.components().all(|c| matches!(c, Component::Normal(_)));
    if !plain || rel.extension().is_none_or(|e| e != "md") {
        return Err(DocError::Outside(shown));
    }
    let root = repo.join(DOCS_DIR);
    let path = root.join(rel);
    // A link inside `docs/` could still point out of it.
    let io = |source| DocError::Io {
        path: shown.clone(),
        source,
    };
    let real = path.canonicalize().map_err(io)?;
    let real_root = root.canonicalize().map_err(io)?;
    if !real.starts_with(&real_root) {
        return Err(DocError::Outside(shown));
    }
    let size = fs::metadata(&real).map_err(io)?.len();
    if size > MAX_DOC_BYTES {
        return Err(DocError::TooLarge(shown));
    }
    let markdown = fs::read_to_string(&real).map_err(io)?;
    let base = path.parent().unwrap_or(&root);
    let images = images(&markdown, base);
    Ok(ResearchDocument {
        path: Some(path),
        markdown,
        images,
    })
}

/// The local PNG images `markdown` shows (`![alt](src)`), resolved against `base`, that exist,
/// with their size. Web images and other formats are left out: the UI shows them as text.
pub(crate) fn images(markdown: &str, base: &Path) -> Vec<DocImage> {
    let mut out: Vec<DocImage> = Vec::new();
    for src in image_sources(markdown) {
        if out.len() >= MAX_IMAGES {
            break;
        }
        if out.iter().any(|i| i.src == src) {
            continue;
        }
        let local = src.strip_prefix("file://").unwrap_or(&src);
        if local.contains("://") || local.starts_with("data:") {
            continue;
        }
        let path = if Path::new(local).is_absolute() {
            PathBuf::from(local)
        } else {
            base.join(local)
        };
        if let Some((width, height)) = png_size(&path) {
            out.push(DocImage {
                src: src.clone(),
                path,
                width,
                height,
            });
        }
    }
    out
}

/// Every `src` of `![alt](src "title")` in `markdown`, in order.
pub(crate) fn image_sources(markdown: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = markdown;
    while let Some(at) = rest.find("![") {
        rest = &rest[at + 2..];
        let Some(close) = rest.find("](") else {
            break;
        };
        // Alt text does not span lines.
        if rest[..close].contains('\n') {
            continue;
        }
        let after = &rest[close + 2..];
        let Some(end) = after.find(')') else {
            break;
        };
        let inner = after[..end].trim();
        let src = inner
            .split_whitespace()
            .next()
            .unwrap_or("")
            .trim_matches(['<', '>']);
        if !src.is_empty() {
            out.push(src.to_string());
        }
        rest = &after[end..];
    }
    out
}

/// Width and height of a PNG file, from its header; `None` for anything else.
fn png_size(path: &Path) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut head = [0u8; 24];
    fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
    if head[..8] != SIGNATURE || &head[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
    let height = u32::from_be_bytes([head[20], head[21], head[22], head[23]]);
    (width > 0 && height > 0).then_some((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13];
        out.extend_from_slice(b"IHDR");
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&[8, 6, 0, 0, 0]);
        out
    }

    #[test]
    fn image_sources_are_found_in_order() {
        let md = "a ![one](fig/a.png) b ![two](<b c.png> \"t\")\n![x](https://e.com/i.png)";
        assert_eq!(image_sources(md), ["fig/a.png", "b", "https://e.com/i.png"]);
    }

    #[test]
    fn local_pngs_are_measured_and_the_rest_left_out() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("a.png"), png(640, 480)).expect("write");
        fs::write(dir.path().join("b.jpg"), b"\xff\xd8\xff").expect("write");
        let md = "![a](a.png) ![b](b.jpg) ![c](missing.png) ![w](https://x/y.png)";
        let found = images(md, dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].width, found[0].height), (640, 480));
        assert_eq!(found[0].path, dir.path().join("a.png"));
    }

    #[test]
    fn docs_are_listed_with_titles_and_read_only_from_inside() {
        let repo = tempfile::tempdir().expect("tempdir");
        let docs = repo.path().join("docs/adr");
        fs::create_dir_all(&docs).expect("mkdir");
        fs::write(docs.join("0001-x.md"), "# Collapse turns\n\ntext").expect("write");
        fs::write(repo.path().join("docs/notes.md"), "no heading").expect("write");
        fs::write(repo.path().join("docs/skip.txt"), "x").expect("write");
        fs::write(repo.path().join("secret.md"), "# s").expect("write");
        let listed = list(repo.path());
        let paths: Vec<_> = listed.iter().map(|d| d.path.clone()).collect();
        assert_eq!(
            paths,
            [PathBuf::from("adr/0001-x.md"), PathBuf::from("notes.md")]
        );
        assert_eq!(listed[0].title, "Collapse turns");
        assert_eq!(listed[1].title, "notes.md");
        let doc = load(repo.path(), Path::new("adr/0001-x.md")).expect("loads");
        assert!(doc.markdown.starts_with("# Collapse"));
        for bad in ["../secret.md", "/etc/passwd", "skip.txt"] {
            assert!(matches!(
                load(repo.path(), Path::new(bad)),
                Err(DocError::Outside(_))
            ));
        }
    }
}
