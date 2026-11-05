//! Per-project worktree recipe (Decision 12b), read from `~/.config/dovetail/projects/<id>.toml`.
//!
//! ```toml
//! name = "dovetail"
//! [worktree]
//! symlink = []              # paths relative to the repo root; symlinked, never copied (12c)
//! setup = ["cargo fetch"]   # run in order in the new worktree; each must be safe to re-run (B5)
//! [env]                     # set for the setup commands and for the slice's session
//! ```

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
