//! Paths for `@` completion in a chat bar ([`dovetail_api::conversation::ConversationCommand::ListPaths`]). Claude Code
//! reads `@path` itself, headless too (docs/spikes/slash-and-at.md), so this only helps type it.

use std::path::Path;
use std::process::{Command, Stdio};

use crate::profile;

/// Most paths sent for one listing; a huge tree is cut, not slow.
pub(crate) const MAX_PATHS: usize = 20_000;

/// Files under `root` that git does not ignore (tracked and untracked), relative to it. A folder
/// without git, or a git failure, gives none: completion then just has nothing to offer.
pub(crate) fn tree(root: &Path) -> Vec<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(out) if out.status.success() => out
            .stdout
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .take(MAX_PATHS)
            .map(|p| String::from_utf8_lossy(p).into_owned())
            .collect(),
        Ok(_) | Err(_) => Vec::new(),
    }
}

/// The entries of `dir` as typed after `@` (`~/`, absolute, or relative to `root`), directories
/// with a trailing `/`, sorted. Hidden entries are included; the chat bar shows them only when
/// a dot is typed.
pub(crate) fn dir(root: &Path, dir: &str) -> Vec<String> {
    let typed = profile::expand(dir);
    let path = if typed.is_absolute() {
        typed
    } else {
        root.join(typed)
    };
    let Ok(entries) = std::fs::read_dir(&path) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir())
                || e.path().metadata().is_ok_and(|m| m.is_dir());
            if is_dir { format!("{name}/") } else { name }
        })
        .take(MAX_PATHS)
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_directory_lists_dirs_with_a_slash() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("a.md"), "").unwrap();
        std::fs::write(tmp.path().join(".env"), "").unwrap();
        assert_eq!(dir(tmp.path(), "./"), vec![".env", "a.md", "src/"]);
    }
}
