//! Profiles: a Claude Code account per project (ADR 0009, task #27).
//!
//! A profile is a name for a Claude Code config directory, listed in `config.toml`:
//!
//! ```toml
//! [profiles]
//! walid = "~/.claude"
//! maveron = "~/.local/share/dovetail/profiles/maveron"
//! ```
//!
//! A project picks one with `profile = "<name>"` in its recipe. Every session of the project then
//! starts with `CLAUDE_CONFIG_DIR=<dir>`, which moves Claude Code's credentials, global config,
//! transcripts and to-do lists there (spike `docs/spikes/profiles.md`). No profile means the
//! default home: `$CLAUDE_CONFIG_DIR` as Dovetail itself was started with, else `~/.claude`.
//! `dovetail login <name>` puts new profiles under `Paths::profiles`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The Claude Code config directory a project's sessions run under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeHome {
    /// The profile's name: the recipe's, or for the default home the profile that names it, if
    /// any. `None` when nothing names it.
    pub name: Option<String>,
    /// The config directory, absolute.
    pub dir: PathBuf,
    /// Whether sessions get `CLAUDE_CONFIG_DIR` set: only when the recipe picks a profile, so a
    /// project without one runs exactly as before.
    pub explicit: bool,
}

impl ClaudeHome {
    /// Where Claude Code keeps native to-do lists for this home.
    pub fn tasks_dir(&self) -> PathBuf {
        self.dir.join("tasks")
    }

    /// The name shown for it, `default` when nothing names it.
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or("default")
    }
}

/// Errors choosing a project's profile.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    /// The recipe names a profile `config.toml` does not list.
    #[error(
        "the project's recipe picks Claude profile {0:?}, which config.toml does not list under \
         [profiles]; add it there (or run `dovetail login {0}`), then restart"
    )]
    Unknown(String),
    /// The recipe's `[env]` sets `CLAUDE_CONFIG_DIR` itself.
    #[error(
        "the project's recipe sets CLAUDE_CONFIG_DIR in [env]; pick the account with \
         profile = \"<name>\" instead (a profile from config.toml [profiles]), then restart"
    )]
    EnvOverride,
    /// The recipe could not be read, so which profile it picks is unknown.
    #[error("{0}; fix the recipe, then restart")]
    Recipe(String),
    /// The profile's directory is not an absolute path (Claude Code would resolve a relative
    /// one against several folders).
    #[error("Claude profile {name:?} must name an absolute folder (or one under ~/), not {dir:?}")]
    Relative {
        /// The profile.
        name: String,
        /// Its directory as written.
        dir: String,
    },
}

/// The default Claude Code home: `$CLAUDE_CONFIG_DIR` if set, else `~/.claude`.
pub fn default_dir() -> PathBuf {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".claude"))
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Expands a leading `~/` (or a lone `~`) to `$HOME`.
pub fn expand(dir: &str) -> PathBuf {
    if dir == "~" {
        home()
    } else if let Some(rest) = dir.strip_prefix("~/") {
        home().join(rest)
    } else {
        PathBuf::from(dir)
    }
}

/// The directory a profile names, expanded and checked to be absolute.
pub fn profile_dir(name: &str, dir: &str) -> Result<PathBuf, ProfileError> {
    let path = expand(dir);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(ProfileError::Relative {
            name: name.to_owned(),
            dir: dir.to_owned(),
        })
    }
}

/// The home for a project whose recipe picks `profile`, out of `profiles` (`config.toml`).
pub fn resolve(
    profiles: &BTreeMap<String, String>,
    profile: Option<&str>,
) -> Result<ClaudeHome, ProfileError> {
    match profile {
        Some(name) => {
            let dir = profiles
                .get(name)
                .ok_or_else(|| ProfileError::Unknown(name.to_owned()))?;
            Ok(ClaudeHome {
                name: Some(name.to_owned()),
                dir: profile_dir(name, dir)?,
                explicit: true,
            })
        }
        None => {
            let dir = default_dir();
            let name = profiles
                .iter()
                .find(|(n, d)| profile_dir(n, d).is_ok_and(|d| same_dir(&d, &dir)))
                .map(|(n, _)| n.clone());
            Ok(ClaudeHome {
                name,
                dir,
                explicit: false,
            })
        }
    }
}

/// Whether two config directories are the same folder (a trailing slash or a symlink aside).
pub fn same_dir(a: &Path, b: &Path) -> bool {
    if a.components().eq(b.components()) {
        return true;
    }
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Whether a profile folder holds a Claude Code login: the credentials file exists. Never reads
/// it.
pub fn has_credentials(dir: &Path) -> bool {
    dir.join(".credentials.json").is_file()
}

/// Whether `name` can name a profile: letters, digits, `-` and `_` (a bare TOML key, and a
/// folder name under `Paths::profiles`).
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A path as written into `config.toml`: under the home folder as `~/...`.
pub fn display_dir(dir: &Path) -> String {
    let home = home();
    match dir.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => dir.display().to_string(),
    }
}

/// `config.toml` text with `name = "<dir>"` added to `[profiles]`, keeping every other line as
/// the user wrote it: the entry goes right under an existing `[profiles]` header, or a new table
/// is appended. `None` if the result would not parse to a config listing the profile (the table
/// written some other way, say inline): the user adds it by hand then.
pub fn add_profile_text(text: &str, name: &str, dir: &str) -> Option<String> {
    // Keep the file's line endings.
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let entry = format!("{name} = {}", toml::Value::String(dir.to_owned()));
    let header = text.lines().position(|l| {
        l.split('#')
            .next()
            .is_some_and(|l| l.trim() == "[profiles]")
    });
    let out = match header {
        Some(at) => {
            let mut lines: Vec<&str> = text.lines().collect();
            lines.insert(at + 1, &entry);
            let mut out = lines.join(nl);
            out.push_str(nl);
            out
        }
        None => {
            let mut out = text.to_owned();
            if !out.is_empty() && !out.ends_with('\n') {
                out.push_str(nl);
            }
            if !out.is_empty() {
                out.push_str(nl);
            }
            out.push_str(&format!("[profiles]{nl}{entry}{nl}"));
            out
        }
    };
    let parsed = crate::config::Config::parse(&out).ok()?;
    (parsed.profiles.get(name).map(String::as_str) == Some(dir)).then_some(out)
}

/// One line of `dovetail login --list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileStatus {
    /// The profile's name.
    pub name: String,
    /// Its folder, expanded; `Err` holds the problem when unusable.
    pub dir: Result<PathBuf, ProfileError>,
    /// Whether the folder holds a login (the credentials file exists; never read).
    pub logged_in: bool,
}

/// Every profile in `profiles` with its folder and login state.
pub fn statuses(profiles: &BTreeMap<String, String>) -> Vec<ProfileStatus> {
    profiles
        .iter()
        .map(|(name, dir)| {
            let dir = profile_dir(name, dir);
            let logged_in = dir.as_deref().is_ok_and(has_credentials);
            ProfileStatus {
                name: name.clone(),
                dir,
                logged_in,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_a_profile_keeps_the_users_lines() {
        let text = "# mine\nmax_slices = 2\n\n[profiles]\nwalid = \"~/.claude\" # work\n";
        let out = add_profile_text(text, "maveron", "/p/maveron").unwrap();
        assert!(
            out.starts_with("# mine\nmax_slices = 2\n\n[profiles]\nmaveron = \"/p/maveron\"\n")
        );
        assert!(out.contains("walid = \"~/.claude\" # work"));

        let out = add_profile_text("max_slices = 2", "maveron", "/p/m").unwrap();
        assert_eq!(out, "max_slices = 2\n\n[profiles]\nmaveron = \"/p/m\"\n");
        assert_eq!(
            add_profile_text("", "a", "/x").as_deref(),
            Some("[profiles]\na = \"/x\"\n")
        );
        // CRLF files stay CRLF.
        assert_eq!(
            add_profile_text("a = 1\r\n[profiles]\r\nb = \"/b\"\r\n", "c", "/c").as_deref(),
            Some("a = 1\r\n[profiles]\r\nc = \"/c\"\r\nb = \"/b\"\r\n")
        );
        assert_eq!(
            add_profile_text("a = 1\r\n", "c", "/c").as_deref(),
            Some("a = 1\r\n\r\n[profiles]\r\nc = \"/c\"\r\n")
        );
        // Written inline: not touched.
        assert_eq!(
            add_profile_text("profiles = { a = \"/x\" }\n", "b", "/y"),
            None
        );
    }

    #[test]
    fn names_are_bare_keys() {
        assert!(valid_name("maveron"));
        assert!(valid_name("work_2"));
        assert!(!valid_name(""));
        assert!(!valid_name("a b"));
        assert!(!valid_name("../x"));
        assert!(!valid_name("-x"));
    }

    #[test]
    fn statuses_check_the_credentials_file_only() {
        let tmp = tempfile::tempdir().unwrap();
        let logged = tmp.path().join("logged");
        std::fs::create_dir_all(&logged).unwrap();
        std::fs::write(logged.join(".credentials.json"), "{}").unwrap();
        let profiles = BTreeMap::from([
            ("a".to_owned(), logged.to_string_lossy().into_owned()),
            (
                "b".to_owned(),
                tmp.path().join("empty").to_string_lossy().into_owned(),
            ),
            ("c".to_owned(), "relative".to_owned()),
        ]);
        let s = statuses(&profiles);
        assert!(s[0].logged_in);
        assert_eq!(s[0].dir.as_deref().ok(), Some(logged.as_path()));
        assert!(!s[1].logged_in);
        assert!(s[2].dir.is_err() && !s[2].logged_in);
    }

    fn profiles() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("walid".to_owned(), "/home/u/.claude".to_owned()),
            ("maveron".to_owned(), "/data/profiles/maveron".to_owned()),
            ("bad".to_owned(), "relative/dir".to_owned()),
        ])
    }

    #[test]
    fn a_named_profile_resolves_to_its_dir_and_sets_the_env() {
        let home = resolve(&profiles(), Some("maveron")).unwrap();
        assert_eq!(home.name.as_deref(), Some("maveron"));
        assert_eq!(home.dir, PathBuf::from("/data/profiles/maveron"));
        assert!(home.explicit);
        assert_eq!(
            home.tasks_dir(),
            PathBuf::from("/data/profiles/maveron/tasks")
        );
    }

    #[test]
    fn an_unknown_or_relative_profile_is_an_error() {
        assert_eq!(
            resolve(&profiles(), Some("nobody")),
            Err(ProfileError::Unknown("nobody".into()))
        );
        assert!(matches!(
            resolve(&profiles(), Some("bad")),
            Err(ProfileError::Relative { .. })
        ));
    }

    #[test]
    fn no_profile_is_the_default_home_without_env() {
        let home = resolve(&BTreeMap::new(), None).unwrap();
        assert_eq!(home.dir, default_dir());
        assert!(!home.explicit);
        assert_eq!(home.label(), "default");
    }

    #[test]
    fn tilde_expands_to_home() {
        let expanded = expand("~/x/y");
        assert!(expanded.ends_with("x/y"));
        assert!(expanded.is_absolute() || std::env::var_os("HOME").is_none());
    }
}
