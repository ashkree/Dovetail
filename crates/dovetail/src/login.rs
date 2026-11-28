//! `dovetail login <profile>` and `dovetail login --list` (ADR 0009): set up the Claude Code
//! account a project's sessions run under. Runs outside the TUI.
//!
//! `login <profile>` makes the profile's folder if needed (a new profile goes under
//! `<data>/profiles/<name>`), adds it to `config.toml` if missing (keeping the user's lines), and
//! runs `claude` interactively with `CLAUDE_CONFIG_DIR` set to it, so the user can `/login`.
//! `--list` shows each profile, its folder and whether it holds a login; it checks that the
//! credentials file exists and never reads it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use dovetail_core::settings::config::Config;
use dovetail_core::settings::paths::Paths;
use dovetail_core::settings::profile;

#[derive(Debug, clap::Args)]
#[command(group(
    clap::ArgGroup::new("action")
        .required(true)
        .args(["profile", "list"])
))]
pub struct Args {
    /// Claude profile name to sign in (created if needed).
    pub profile: Option<String>,
    /// List profiles and whether each is logged in.
    #[arg(long)]
    pub list: bool,
}

/// Runs `dovetail login`.
pub fn run(args: Args) -> Result<()> {
    let paths = Paths::from_env();
    if args.list {
        print!("{}", list(&paths)?);
        return Ok(());
    }
    if let Some(name) = args.profile.as_deref() {
        return login(&paths, name);
    }
    unreachable!("clap ArgGroup requires either profile or --list")
}

/// The `--list` text.
fn list(paths: &Paths) -> Result<String> {
    let config = Config::load(paths).context("reading config.toml")?;
    let mut out = String::new();
    if config.profiles.is_empty() {
        out.push_str(&format!(
            "No profiles in {}. Projects use the default Claude home, {} ({}).\n\
             Add one with `dovetail login <name>`.\n",
            paths.config.join("config.toml").display(),
            profile::default_dir().display(),
            logged_in_word(profile::has_credentials(&profile::default_dir())),
        ));
        return Ok(out);
    }
    let width = config.profiles.keys().map(String::len).max().unwrap_or(0);
    for status in profile::statuses(&config.profiles) {
        let line = match &status.dir {
            Ok(dir) => format!(
                "{:width$}  {:13}  {}",
                status.name,
                logged_in_word(status.logged_in),
                dir.display()
            ),
            Err(e) => format!("{:width$}  {:13}  {e}", status.name, "unusable"),
        };
        out.push_str(line.trim_end());
        out.push('\n');
    }
    Ok(out)
}

fn logged_in_word(logged_in: bool) -> &'static str {
    if logged_in {
        "logged in"
    } else {
        "not logged in"
    }
}

/// Makes sure profile `name` exists (folder and config entry), then runs `claude` in it.
fn login(paths: &Paths, name: &str) -> Result<()> {
    let config = Config::load(paths).context("reading config.toml")?;
    let dir = ensure_profile(paths, &config, name)?;
    println!(
        "Opening Claude Code under profile {name} ({}).\n\
         Type /login and sign in with the account for this profile, then /exit.",
        dir.display()
    );
    let program = config
        .claude
        .clone()
        .unwrap_or_else(|| PathBuf::from("claude"));
    let status = std::process::Command::new(&program)
        .env("CLAUDE_CONFIG_DIR", &dir)
        .current_dir(&dir)
        .status()
        .with_context(|| format!("running {}", program.display()))?;
    if profile::has_credentials(&dir) {
        println!(
            "Profile {name} is logged in. Pick it in a project's recipe: profile = \"{name}\""
        );
    } else {
        println!(
            "Profile {name} has no login yet (claude exited with {status}). \
             Run `dovetail login {name}` again to sign in."
        );
    }
    Ok(())
}

/// The folder of profile `name`, created if needed; a profile `config.toml` does not list yet is
/// added under `<data>/profiles/<name>`. Returns the folder.
fn ensure_profile(paths: &Paths, config: &Config, name: &str) -> Result<PathBuf> {
    let dir = match config.profiles.get(name) {
        Some(dir) => profile::profile_dir(name, dir)?,
        None => {
            if !profile::valid_name(name) {
                bail!("profile names use letters, digits, `-` and `_`: {name:?}");
            }
            let dir = paths.profiles().join(name);
            add_to_config(&paths.config.join("config.toml"), name, &dir)?;
            println!(
                "Added profile {name} to {}",
                paths.config.join("config.toml").display()
            );
            dir
        }
    };
    create_private_dir(&dir)?;
    Ok(dir)
}

/// Adds `name = "<dir>"` to `[profiles]` in the config file, keeping the user's lines.
fn add_to_config(file: &Path, name: &str, dir: &Path) -> Result<()> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
    };
    let written = profile::display_dir(dir);
    let Some(out) = profile::add_profile_text(&text, name, &written) else {
        bail!(
            "could not add the profile to {} by itself; add this under [profiles] and run \
             again:\n  {name} = \"{written}\"",
            file.display()
        );
    };
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    write_atomic(file, &out)
}

/// Writes through a temp file in the same folder, then renames it over `file`, so a crash
/// never leaves the user's config half written. Keeps the file's permissions.
fn write_atomic(file: &Path, text: &str) -> Result<()> {
    let tmp = file.with_file_name(format!(
        ".{}.dovetail-{}.tmp",
        file.file_name()
            .map_or_else(|| "config".into(), |n| n.to_string_lossy()),
        std::process::id()
    ));
    let result = (|| {
        std::fs::write(&tmp, text)?;
        if let Ok(meta) = std::fs::metadata(file) {
            std::fs::set_permissions(&tmp, meta.permissions())?;
        }
        std::fs::rename(&tmp, file)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.with_context(|| format!("writing {}", file.display()))
}

/// Creates a folder readable by its owner only (it will hold credentials).
fn create_private_dir(dir: &Path) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths_in(root: &Path) -> Paths {
        Paths {
            data: root.join("data"),
            config: root.join("config"),
            state: root.join("state"),
            runtime: root.join("run"),
        }
    }

    fn temp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("dovetail-login-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn list_shows_each_profile_and_its_login_state() {
        let root = temp_root("list");
        let paths = paths_in(&root);
        let logged = root.join("logged");
        std::fs::create_dir_all(&logged).unwrap();
        std::fs::write(logged.join(".credentials.json"), "secret").unwrap();
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(
            paths.config.join("config.toml"),
            format!(
                "[profiles]\nwork = {:?}\nmine = {:?}\n",
                logged.display().to_string(),
                root.join("fresh").display().to_string()
            ),
        )
        .unwrap();
        let text = list(&paths).unwrap();
        assert!(text.contains(&format!("work  logged in      {}", logged.display())));
        assert!(text.contains("mine  not logged in"));
        assert!(!text.contains("secret"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_new_profile_goes_under_the_data_dir_and_into_config() {
        let root = temp_root("new");
        let paths = paths_in(&root);
        std::fs::create_dir_all(&paths.config).unwrap();
        std::fs::write(paths.config.join("config.toml"), "max_slices = 2\n").unwrap();
        let config = Config::load(&paths).unwrap();
        let dir = ensure_profile(&paths, &config, "maveron").unwrap();
        assert_eq!(dir, paths.profiles().join("maveron"));
        assert!(dir.is_dir());
        let config = Config::load(&paths).unwrap();
        assert_eq!(config.max_slices, 2);
        assert_eq!(
            config.claude_home(Some("maveron")).unwrap().dir,
            paths.profiles().join("maveron")
        );
        // A second run finds it listed and changes nothing.
        let before = std::fs::read_to_string(paths.config.join("config.toml")).unwrap();
        ensure_profile(&paths, &config, "maveron").unwrap();
        let after = std::fs::read_to_string(paths.config.join("config.toml")).unwrap();
        assert_eq!(before, after);
        assert!(ensure_profile(&paths, &config, "bad name").is_err());
        // Written atomically: no temp file left beside it.
        let left: Vec<_> = std::fs::read_dir(&paths.config)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("config.toml")]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
