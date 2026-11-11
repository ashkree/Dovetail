//! Global settings from `~/.config/dovetail/config.toml` (plan B10, `max_slices` from ADR 0004).
//!
//! ```toml
//! reading_width = 120        # columns of the conversation reading column
//! claude = "/usr/bin/claude" # default: `claude` on PATH
//! default_model = "opus"     # default: whatever Claude Code picks
//! side_chat_model = "sonnet" # model of side chats (ADR 0011); default: sonnet
//! max_slices = 3             # running slices at once
//! [memory]
//! coordinator_gb = 4         # cgroup cap for the coordinator session
//! slice_gb = 3               # cgroup cap for each slice session
//! [profiles]                 # Claude Code accounts, by name (ADR 0009, `profile`)
//! walid = "~/.claude"
//! maveron = "~/.local/share/dovetail/profiles/maveron"
//! [suites]                   # tool suites per role (ADR 0016); a role left out keeps its default
//! coordinator = ["core", "tasks", "project", "coordination", "research", "maps"]
//! slice = ["core", "coordination", "maps"]
//! side_chat = ["core"]
//! ```
//!
//! Every field is optional.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dovetail_api::tools::{Role, Suite};
use serde::Deserialize;

use crate::paths::Paths;

/// Dovetail's global settings, with defaults for anything left out.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Width of the reading column, in terminal columns.
    pub reading_width: u16,
    /// Path of the `claude` binary; `None` means find `claude` on `PATH`.
    pub claude: Option<PathBuf>,
    /// Model passed to new sessions; `None` leaves the choice to Claude Code.
    pub default_model: Option<String>,
    /// Model passed to side chats, faster than the coordinator's (ADR 0011).
    pub side_chat_model: Option<String>,
    /// Most slice sessions running at once.
    pub max_slices: u32,
    /// Memory caps.
    pub memory: MemoryCaps,
    /// Claude Code config directories by profile name; a project picks one in its recipe.
    pub profiles: BTreeMap<String, String>,
    /// The tool suites each role gets.
    pub suites: RoleSuites,
}

/// The `[suites]` table: the tool suites each role gets (ADR 0016). `--allowedTools`, the
/// bridge's tool list and the core's check on each call all follow it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct RoleSuites {
    /// The coordinator's suites.
    pub coordinator: Vec<Suite>,
    /// Each slice's suites.
    pub slice: Vec<Suite>,
    /// Each side chat's suites.
    pub side_chat: Vec<Suite>,
}

impl Default for RoleSuites {
    fn default() -> Self {
        Self {
            coordinator: Role::Coordinator.default_suites().to_vec(),
            slice: Role::Slice.default_suites().to_vec(),
            side_chat: Role::SideChat.default_suites().to_vec(),
        }
    }
}

impl RoleSuites {
    /// The suites of a session with `role`.
    pub fn for_role(&self, role: Role) -> &[Suite] {
        match role {
            Role::Coordinator => &self.coordinator,
            Role::Slice => &self.slice,
            Role::SideChat => &self.side_chat,
        }
    }
}

/// The `[memory]` table: cgroup caps for sessions, in gigabytes.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct MemoryCaps {
    /// Cap for the coordinator session.
    pub coordinator_gb: u32,
    /// Cap for each slice session.
    pub slice_gb: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            reading_width: 120,
            claude: None,
            default_model: None,
            side_chat_model: Some("sonnet".into()),
            max_slices: 3,
            memory: MemoryCaps::default(),
            profiles: BTreeMap::new(),
            suites: RoleSuites::default(),
        }
    }
}

impl Default for MemoryCaps {
    fn default() -> Self {
        Self {
            coordinator_gb: 4,
            slice_gb: 3,
        }
    }
}

/// Errors reading `config.toml`.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file exists but could not be read.
    #[error("reading {path}: {source}")]
    Io {
        /// The config file.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The file is not valid config.
    #[error("parsing {path}: {source}")]
    Parse {
        /// The config file.
        path: PathBuf,
        /// The cause.
        source: toml::de::Error,
    },
}

impl Config {
    /// Parses config text.
    pub fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// Reads `config.toml` under the config directory; a missing file gives the defaults.
    pub fn load(paths: &Paths) -> Result<Self, ConfigError> {
        Self::load_from(&paths.config.join("config.toml"))
    }

    /// The Claude home for a project whose recipe picks `profile` (`None`: the default home).
    pub fn claude_home(
        &self,
        profile: Option<&str>,
    ) -> Result<crate::profile::ClaudeHome, crate::profile::ProfileError> {
        crate::profile::resolve(&self.profiles, profile)
    }

    /// Reads a config file; a missing file gives the defaults.
    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|source| ConfigError::Parse {
                path: path.to_owned(),
                source,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(ConfigError::Io {
                path: path.to_owned(),
                source,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_parse_by_name() {
        let config = Config::parse(
            "max_slices = 2\n[profiles]\nwalid = \"~/.claude\"\nmaveron = \"/p/maveron\"\n",
        )
        .unwrap();
        assert_eq!(config.profiles["maveron"], "/p/maveron");
        assert_eq!(config.profiles["walid"], "~/.claude");
        let home = config.claude_home(Some("maveron")).unwrap();
        assert_eq!(home.dir, PathBuf::from("/p/maveron"));
        assert!(config.claude_home(Some("missing")).is_err());
        assert!(Config::parse("").unwrap().profiles.is_empty());
    }

    #[test]
    fn suites_default_per_role_and_parse_per_key() {
        let defaults = Config::parse("").unwrap().suites;
        assert_eq!(defaults.for_role(Role::Coordinator), Suite::ALL);
        assert_eq!(
            defaults.for_role(Role::Slice),
            [Suite::Core, Suite::Coordination, Suite::Maps]
        );
        assert_eq!(defaults.for_role(Role::SideChat), [Suite::Core]);
        let partial = Config::parse("[suites]\nslice = [\"core\"]\n")
            .unwrap()
            .suites;
        assert_eq!(partial.for_role(Role::Slice), [Suite::Core]);
        assert_eq!(partial.for_role(Role::Coordinator), Suite::ALL);
        assert!(Config::parse("[suites]\nslice = [\"everything\"]\n").is_err());
    }

    #[test]
    fn memory_caps_default_to_b8_and_parse_per_key() {
        let defaults = Config::parse("").unwrap().memory;
        assert_eq!((defaults.coordinator_gb, defaults.slice_gb), (4, 3));
        let partial = Config::parse("[memory]\nslice_gb = 2\n").unwrap().memory;
        assert_eq!((partial.coordinator_gb, partial.slice_gb), (4, 2));
        let both = Config::parse("[memory]\ncoordinator_gb = 6\nslice_gb = 5\n")
            .unwrap()
            .memory;
        assert_eq!((both.coordinator_gb, both.slice_gb), (6, 5));
        assert!(Config::parse("[memory]\nslice_gb = \"3G\"\n").is_err());
    }
}
