//! Per-project worktree recipe (Decision 12b), read from `~/.config/dovetail/projects/<id>.toml`.
//!
//! ```toml
//! name = "dovetail"
//! profile = "maveron"       # Claude account, from config.toml [profiles] (ADR 0009)
//! research_dir = "docs/research"  # opt-in: research files in the repo (ADR 0015)
//! [worktree]
//! symlink = []              # paths relative to the repo root; symlinked, never copied (12c)
//! setup = ["cargo fetch"]   # run in order in the new worktree; each must be safe to re-run (B5)
//! [env]                     # set for the setup commands and for the slice's session
//! preview = "npm run dev -- --port {port}"  # the slice's web preview (ADR 0026)
//! [repos.web]               # overrides for one repo of the project (ADR 0021)
//! setup = ["npm ci"]        # replaces [worktree] setup for that repo's slices
//! [repos.web.env]           # merged over [env]; its keys win
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
    /// Where research files go (ADR 0015 point 7): relative to the repo root, or absolute.
    /// `None`: Dovetail's data folder. An explicit opt-in to write into the repo; Dovetail
    /// writes only under this folder.
    #[serde(default)]
    pub research_dir: Option<PathBuf>,
    /// The command that serves a slice's web preview (ADR 0026); `{port}` becomes a free port on
    /// 127.0.0.1. Run in the slice's worktree when the viewer first opens on the slice.
    #[serde(default)]
    pub preview: Option<String>,
    /// How slice worktrees are prepared.
    #[serde(default)]
    pub worktree: WorktreeRecipe,
    /// Environment for setup commands and the slice session.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Per-repo overrides, by repo name (ADR 0021 point 6).
    #[serde(default)]
    pub repos: BTreeMap<String, RepoRecipe>,
}

/// A `[repos.<name>]` table: what differs for one repo's slices.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoRecipe {
    /// Replaces `[worktree] setup` when given.
    #[serde(default)]
    pub setup: Option<Vec<String>>,
    /// Replaces the top-level `preview` when given.
    #[serde(default)]
    pub preview: Option<String>,
    /// Merged over `[env]`; these keys win.
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
    /// The recipe as it applies to repo `name`'s slices: its `[repos.<name>]` table, if any,
    /// over the top-level keys. A repo with no table gets the recipe unchanged.
    pub fn for_repo(&self, name: &str) -> Recipe {
        let mut out = self.clone();
        if let Some(over) = self.repos.get(name) {
            if let Some(setup) = &over.setup {
                out.worktree.setup = setup.clone();
            }
            if let Some(preview) = &over.preview {
                out.preview = Some(preview.clone());
            }
            out.env
                .extend(over.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        out
    }

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

/// The commented-out profile line of a starter recipe: [`set_profile_text`] uncomments it.
const STARTER_PROFILE: &str = "# profile = \"";

/// The recipe `:project add` writes when the project has none (task #31): the name, and the
/// `profile`, `[worktree]` and `[env]` sections commented out, each with a one-line hint.
/// `profiles` are the names in `config.toml` `[profiles]`, listed in the profile hint.
pub fn starter(name: &str, profiles: &[&str]) -> String {
    let name = toml::Value::String(name.to_owned());
    let example = profiles.first().copied().unwrap_or("work");
    let listed = if profiles.is_empty() {
        "none yet: `dovetail login <name>` adds one".to_owned()
    } else {
        profiles.join(", ")
    };
    format!(
        "# Worktree recipe (Decision 12b), written once by `:project add`; Dovetail never \
         overwrites it.\n\
         # Every section is optional. Uncomment what you need (Dovetail's docs/self-hosting.md).\n\
         \n\
         name = {name}\n\
         \n\
         # The Claude account (ADR 0009) from config.toml [profiles] ({listed}). Left out: ~/.claude.\n\
         {STARTER_PROFILE}{example}\"\n\
         \n\
         # The command that serves a slice's web preview in the viewer (ADR 0026); {{port}} is filled in.\n\
         # A [repos.<name>] table can set its own preview line.\n\
         # preview = \"npm run dev -- --port {{port}}\"\n\
         \n\
         # How slice worktrees are prepared.\n\
         # [worktree]\n\
         # Paths relative to the repo root, symlinked (never copied) into each new worktree.\n\
         # symlink = [\".env\"]\n\
         # Commands run in order in the new worktree before the slice's first prompt; safe to re-run.\n\
         # setup = [\"cargo fetch\"]\n\
         \n\
         # Set for the setup commands and the slice's session; {{worktree}} and {{worktree_hash}} are filled in.\n\
         # [env]\n\
         # BUILD_DIR = \"{{worktree}}/target\"\n"
    )
}

/// Recipe text with the starter's commented-out profile line replaced by
/// `profile = "<profile>"`. `None` when there is no such line (the file was edited) or the
/// result would not parse to that profile.
pub fn set_profile_text(text: &str, profile: &str) -> Option<String> {
    let line = format!("profile = {}", toml::Value::String(profile.to_owned()));
    let mut found = false;
    let mut out = String::with_capacity(text.len() + line.len());
    for l in text.split_inclusive('\n') {
        if !found && l.starts_with(STARTER_PROFILE) {
            found = true;
            out.push_str(&line);
            out.push_str(if l.ends_with("\r\n") {
                "\r\n"
            } else if l.ends_with('\n') {
                "\n"
            } else {
                ""
            });
        } else {
            out.push_str(l);
        }
    }
    let parsed = Recipe::parse(&out).ok()?;
    (found && parsed.profile.as_deref() == Some(profile)).then_some(out)
}

/// FNV-1a: a stable, dependency-free hash for `{worktree_hash}`.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_repos_preview_line_replaces_the_projects() {
        let r = super::Recipe::parse(
            "preview = \"a {port}\"\n[repos.web]\npreview = \"b {port}\"\n[repos.api]\nsetup = []\n",
        )
        .unwrap();
        assert_eq!(r.for_repo("web").preview.as_deref(), Some("b {port}"));
        assert_eq!(r.for_repo("api").preview.as_deref(), Some("a {port}"));
        assert_eq!(r.for_repo("other").preview.as_deref(), Some("a {port}"));
    }

    use super::*;

    #[test]
    fn a_recipe_may_pick_a_profile() {
        let recipe = Recipe::parse("name = \"d\"\nprofile = \"maveron\"\n").unwrap();
        assert_eq!(recipe.profile.as_deref(), Some("maveron"));
        assert_eq!(Recipe::parse("").unwrap().profile, None);
    }

    #[test]
    fn a_recipe_may_name_a_research_dir() {
        let recipe = Recipe::parse("research_dir = \"docs/research\"\n").unwrap();
        assert_eq!(recipe.research_dir, Some(PathBuf::from("docs/research")));
        assert_eq!(Recipe::parse("").unwrap().research_dir, None);
    }

    #[test]
    fn the_repos_own_recipe_parses() {
        let text = include_str!("../../../../config/projects/dovetail.toml");
        let recipe = Recipe::parse(text).unwrap();
        assert_eq!(recipe.research_dir, Some(PathBuf::from("docs/research")));
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

    #[test]
    fn the_starter_recipe_parses_to_just_a_name() {
        let text = starter("my \"app\"", &["maveron", "walid"]);
        let recipe = Recipe::parse(&text).unwrap();
        assert_eq!(recipe.name.as_deref(), Some("my \"app\""));
        assert_eq!(
            recipe,
            Recipe {
                name: recipe.name.clone(),
                ..Recipe::default()
            }
        );
        assert!(text.contains("(maveron, walid)"));
        assert!(Recipe::parse(&starter("x", &[])).is_ok());
    }

    /// ADR 0021 point 6: a `[repos.<name>]` table replaces `setup` and wins over `[env]` for
    /// that repo only.
    #[test]
    fn a_repo_table_overrides_setup_and_env_for_that_repo() {
        let recipe = Recipe::parse(
            "[worktree]\nsetup = [\"cargo fetch\"]\n[env]\nA = \"1\"\nB = \"2\"\n\
             [repos.web]\nsetup = [\"npm ci\"]\n[repos.web.env]\nB = \"web\"\n\
             [repos.docs.env]\nC = \"3\"\n",
        )
        .unwrap();
        let web = recipe.for_repo("web");
        assert_eq!(web.worktree.setup, ["npm ci"]);
        assert_eq!(web.env["A"], "1");
        assert_eq!(web.env["B"], "web");
        let docs = recipe.for_repo("docs");
        assert_eq!(docs.worktree.setup, ["cargo fetch"]);
        assert_eq!(docs.env["C"], "3");
        assert_eq!(recipe.for_repo("api"), recipe);
        assert!(Recipe::parse("[repos.web]\nsymlink = []\n").is_err());
    }

    #[test]
    fn a_profile_uncomments_the_starter_line() {
        let text = set_profile_text(&starter("app", &["maveron"]), "maveron").unwrap();
        assert_eq!(
            Recipe::parse(&text).unwrap().profile.as_deref(),
            Some("maveron")
        );
        assert_eq!(set_profile_text("name = \"app\"\n", "maveron"), None);
    }
}
