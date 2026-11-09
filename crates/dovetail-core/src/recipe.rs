//! Per-project worktree recipe (Decision 12b), read from `~/.config/dovetail/projects/<id>.toml`.
//!
//! ```toml
//! name = "dovetail"
//! profile = "maveron"       # Claude account, from config.toml [profiles] (ADR 0009)
//! [worktree]
//! symlink = []              # paths relative to the repo root; symlinked, never copied (12c)
//! setup = ["cargo fetch"]   # run in order in the new worktree; each must be safe to re-run (B5)
//! [env]                     # set for the setup commands and for the slice's session
//! ```
//!
//! `[env]` values may use two placeholders: `{worktree}`, the slice's worktree (or, in a folder
//! without git, the shared project folder), and `{worktree_hash}`, eight hex digits from that
//! path, for short per-worktree paths such as a socket directory under `/tmp`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// A project's worktree recipe. Every section is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recipe {
    /// Display name.
    #[serde(default)]
    pub name: Option<String>,
    /// The Claude profile every session of the project runs under; `None` for the default.
    #[serde(default)]
    pub profile: Option<String>,
    /// How slice worktrees are prepared.
    #[serde(default)]
    pub worktree: WorktreeRecipe,
    /// Environment for setup commands and the slice session.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// The `[worktree]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorktreeRecipe {
    /// Paths relative to the repo root, symlinked into each new worktree.
    #[serde(default)]
    pub symlink: Vec<PathBuf>,
    /// Shell commands run in order in the new worktree.
    #[serde(default)]
    pub setup: Vec<String>,
}

/// Errors reading a recipe.
#[derive(Debug, thiserror::Error)]
pub enum RecipeError {
    /// The file exists but could not be read.
    #[error("reading {path}: {source}")]
    Io {
        /// The recipe file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The file is not a valid recipe.
    #[error("parsing {path}: {source}")]
    Parse {
        /// The recipe file.
        path: PathBuf,
        /// The cause.
        source: toml::de::Error,
    },
}

impl Recipe {
    /// `[env]` with the placeholders filled in for one worktree.
    pub fn env_for(&self, worktree: &Path) -> BTreeMap<String, String> {
        let path = worktree.to_string_lossy();
        let hash = format!("{:08x}", fnv1a(path.as_bytes()) as u32);
        self.env
            .iter()
            .map(|(k, v)| {
                let v = v
                    .replace("{worktree_hash}", &hash)
                    .replace("{worktree}", &path);
                (k.clone(), v)
            })
            .collect()
    }

    /// Parse recipe text.
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// Read a recipe file; a missing file is an empty recipe.
    pub fn load(path: &Path) -> Result<Self, RecipeError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|source| RecipeError::Parse {
                path: path.to_owned(),
                source,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(RecipeError::Io {
                path: path.to_owned(),
                source,
            }),
        }
    }
}

/// FNV-1a: a stable, dependency-free hash for `{worktree_hash}`.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recipe_may_pick_a_profile() {
        let recipe = Recipe::parse("name = \"d\"\nprofile = \"maveron\"\n").unwrap();
        assert_eq!(recipe.profile.as_deref(), Some("maveron"));
        assert_eq!(Recipe::parse("").unwrap().profile, None);
    }

    #[test]
    fn env_placeholders_name_the_worktree() {
        let recipe = Recipe::parse(
            "[env]\nDOVETAIL_DATA_DIR = \"{worktree}/target/dev/data\"\n\
             DOVETAIL_RUNTIME_DIR = \"/tmp/dvt-{worktree_hash}\"\n",
        )
        .unwrap();
        let env = recipe.env_for(Path::new("/w/slice-a"));
        assert_eq!(env["DOVETAIL_DATA_DIR"], "/w/slice-a/target/dev/data");
        let runtime = &env["DOVETAIL_RUNTIME_DIR"];
        assert_eq!(runtime.len(), "/tmp/dvt-".len() + 8);
        assert_ne!(
            runtime,
            &recipe.env_for(Path::new("/w/slice-b"))["DOVETAIL_RUNTIME_DIR"]
        );
    }
}
