//! XDG paths for Dovetail's state, config, logs and sockets. Nothing is written into user repos.

use std::path::PathBuf;

use dovetail_api::tools::env;

/// Base directories, resolved once from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// `$XDG_DATA_HOME/dovetail`: databases and per-project data.
    pub data: PathBuf,
    /// `$XDG_CONFIG_HOME/dovetail`: `config.toml`, `roles/`, `projects/<id>.toml`.
    pub config: PathBuf,
    /// `$XDG_STATE_HOME/dovetail`: logs and worktree setup state.
    pub state: PathBuf,
    /// `$XDG_RUNTIME_DIR/dovetail`: sockets.
    pub runtime: PathBuf,
}

impl Paths {
    /// Resolve from `XDG_*` and `HOME`, with the XDG defaults. Each `DOVETAIL_*_DIR`
    /// (`dovetail_api::tools::env`) replaces one whole directory, for dev builds in a slice.
    pub fn from_env() -> Self {
        let mut paths = Self::from_xdg();
        let over = |var: &str, dir: &mut PathBuf| {
            if let Some(v) = std::env::var_os(var).filter(|v| !v.is_empty()) {
                *dir = PathBuf::from(v);
            }
        };
        over(env::DATA_DIR, &mut paths.data);
        over(env::CONFIG_DIR, &mut paths.config);
        over(env::STATE_DIR, &mut paths.state);
        over(env::RUNTIME_DIR, &mut paths.runtime);
        paths
    }

    /// The four directories as `DOVETAIL_*_DIR` variables, to pin a child process (the worktree
    /// hook, the MCP bridge) to this app's state whatever its session's environment says.
    pub fn pinned_env(&self) -> Vec<(&'static str, PathBuf)> {
        vec![
            (env::DATA_DIR, self.data.clone()),
            (env::CONFIG_DIR, self.config.clone()),
            (env::STATE_DIR, self.state.clone()),
            (env::RUNTIME_DIR, self.runtime.clone()),
        ]
    }

    fn from_xdg() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let xdg = |var: &str, fallback: &str| {
            std::env::var_os(var)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(fallback))
                .join("dovetail")
        };
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("dovetail");
        Self {
            data: xdg("XDG_DATA_HOME", ".local/share"),
            config: xdg("XDG_CONFIG_HOME", ".config"),
            state: xdg("XDG_STATE_HOME", ".local/state"),
            runtime,
        }
    }

    /// `registry.db`, listing every registered project.
    pub fn registry_db(&self) -> PathBuf {
        self.data.join("registry.db")
    }

    /// A project's `state.db`.
    pub fn state_db(&self, project: &str) -> PathBuf {
        self.data.join("projects").join(project).join("state.db")
    }

    /// A project's worktree recipe.
    pub fn recipe(&self, project: &str) -> PathBuf {
        self.config.join("projects").join(format!("{project}.toml"))
    }

    /// An installed role prompt, e.g. `coordinator` or `slice`.
    pub fn role_prompt(&self, role: &str) -> PathBuf {
        self.config.join("roles").join(format!("{role}.md"))
    }

    /// The app's socket for a project (mode 0600).
    pub fn socket(&self, project: &str) -> PathBuf {
        self.runtime.join(format!("{project}.sock"))
    }

    /// Root for slice worktrees, outside every repo.
    pub fn worktrees(&self, project: &str) -> PathBuf {
        self.data.join("projects").join(project).join("worktrees")
    }

    /// Generated launch files for one session (`settings.json`, `mcp.json`), never in a repo.
    /// `project` is the project id, or the app's key for a folder not registered yet.
    pub fn run_dir(&self, project: &str, agent: &str) -> PathBuf {
        self.state.join("run").join(project).join(agent)
    }

    /// Setup state for one worktree, so a repeat hook call returns at once (B5).
    pub fn setup_state(&self, project: &str, worktree: &str) -> PathBuf {
        self.state.join("setup").join(project).join(worktree)
    }
}
