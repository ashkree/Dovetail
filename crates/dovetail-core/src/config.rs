//! Global settings from `~/.config/dovetail/config.toml` (plan B10, `max_slices` from ADR 0004).
//!
//! ```toml
//! reading_width = 120        # columns of the conversation reading column
//! claude = "/usr/bin/claude" # default: `claude` on PATH
//! default_model = "opus"     # default: whatever Claude Code picks
//! max_slices = 3             # running slices at once
//! [memory]
//! coordinator_gb = 4         # cgroup cap for the coordinator session
//! slice_gb = 3               # cgroup cap for each slice session
//! [profiles]                 # Claude Code accounts, by name (ADR 0009, `profile`)
//! walid = "~/.claude"
//! maveron = "~/.local/share/dovetail/profiles/maveron"
//! ```
//!
//! Every field is optional.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    /// Most slice sessions running at once.
    pub max_slices: u32,
    /// Memory caps.
    pub memory: MemoryCaps,
    /// Claude Code config directories by profile name; a project picks one in its recipe.
    pub profiles: BTreeMap<String, String>,
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
            max_slices: 3,
            memory: MemoryCaps::default(),
            profiles: BTreeMap::new(),
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
}
