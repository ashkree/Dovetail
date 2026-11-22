//! Staging, build and swap with a fake [`Exec`]; nothing here runs git or cargo.

use std::io::Write;
use std::sync::Mutex;

use super::*;

/// Records every call and plays git and cargo: a worktree add creates `src` with role prompts,
/// cargo writes a binary into `CARGO_TARGET_DIR`.
struct Fake {
    calls: Mutex<Vec<String>>,
    /// Fail this program's call (`cargo`, or `git fetch`) with output in the log.
    fail: Option<&'static str>,
    cancel_cargo: bool,
}

impl Fake {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            fail: None,
            cancel_cargo: false,
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

impl Exec for Fake {
    fn run(&self, call: &Call, _cancel: &AtomicBool) -> Result<String, ExecError> {
        let line = format!("{} {}", call.program, call.args.join(" "));
        self.calls.lock().unwrap().push(line.clone());
        let write_log = |text: &str| {
            if let Some(log) = &call.log {
                let mut f = fs::OpenOptions::new().append(true).open(log).unwrap();
                writeln!(f, "{text}").unwrap();
            }
        };
        let failed = || ExecError::Failed {
            program: call.program.clone(),
            code: Some(101),
        };
        if call.program == "cargo" {
            if self.cancel_cargo {
                return Err(ExecError::Cancelled);
            }
            if self.fail == Some("cargo") {
                for i in 0..30 {
                    write_log(&format!("error line {i}"));
                }
                return Err(failed());
            }
            let target = call
                .env
                .iter()
                .find(|(k, _)| k == "CARGO_TARGET_DIR")
                .map(|(_, v)| PathBuf::from(v))
                .unwrap();
            fs::create_dir_all(target.join("release")).unwrap();
            fs::write(target.join("release/dovetail"), "new binary").unwrap();
            return Ok(String::new());
        }
        if line.starts_with("git fetch") && self.fail == Some("git fetch") {
            write_log("fatal: could not read from remote");
            return Err(failed());
        }
        if line.starts_with("git worktree add") {
            let src = PathBuf::from(&call.args[3]);
            fs::create_dir_all(src.join("config/roles")).unwrap();
            fs::write(src.join("config/roles/slice.md"), "new slice prompt").unwrap();
            fs::write(src.join("config/roles/coordinator.md"), "same").unwrap();
            fs::create_dir_all(src.join("config/projects")).unwrap();
            fs::write(src.join("config/projects/dovetail.toml"), "recipe").unwrap();
        }
        if line.starts_with("git worktree remove") {
            let _ = fs::remove_dir_all(&call.args[3]);
        }
        if line.starts_with("git rev-parse") {
            return Ok("abc1234\n".into());
        }
        Ok(String::new())
    }
}

struct Scratch {
    _dir: tempfile::TempDir,
    paths: Paths,
    repo: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let paths = Paths {
            data: root.join("data"),
            config: root.join("config"),
            state: root.join("state"),
            runtime: root.join("run"),
        };
        let repo = root.join("repo");
        fs::create_dir_all(repo.join("crates/dovetail")).unwrap();
        fs::write(repo.join("crates/dovetail/Cargo.toml"), "").unwrap();
        fs::create_dir_all(paths.config.join("roles")).unwrap();
        fs::write(paths.config.join("roles/coordinator.md"), "same").unwrap();
        fs::write(paths.config.join("roles/slice.md"), "my edited prompt").unwrap();
        fs::create_dir_all(&paths.runtime).unwrap();
        Self {
            _dir: dir,
            paths,
            repo,
        }
    }

    fn request(&self, source: Source) -> BuildRequest {
        BuildRequest {
            repo: self.repo.clone(),
            source,
            config_dir: self.paths.config.clone(),
            recipe: Some(self.paths.recipe("dovetail")),
        }
    }

    fn staging(&self) -> Staging {
        Staging::new(&self.paths)
    }
}

fn no_cancel() -> AtomicBool {
    AtomicBool::new(false)
}

#[test]
fn building_origin_main_stages_the_binary_and_notes_what_differs() {
    let s = Scratch::new();
    let fake = Fake::new();
    let ready = build(
        &fake,
        &s.staging(),
        &s.request(Source::OriginMain),
        &no_cancel(),
        42,
    )
    .expect("build");

    assert_eq!(ready.source, "abc1234");
    assert_eq!(ready.built_at_ms, 42);
    // The edited slice prompt and the missing recipe differ; the unchanged coordinator does not.
    assert_eq!(
        ready.differing,
        ["projects/dovetail.toml", "roles/slice.md"]
    );
    assert_eq!(
        fs::read_to_string(s.staging().binary()).unwrap(),
        "new binary"
    );
    assert_eq!(s.staging().ready(), Some(ready));

    let calls = fake.calls();
    let position = |prefix: &str| calls.iter().position(|c| c.starts_with(prefix)).unwrap();
    assert!(position("git fetch --quiet origin main") < position("git worktree add --detach"));
    assert!(position("git worktree add --detach") < position("cargo build --release --locked"));
    assert!(calls.iter().any(|c| c.ends_with("origin/main")));
    // The throwaway worktree is gone and the lock released.
    assert!(!s.staging().src().exists());
    assert!(!s.staging().lock().exists());
}

#[test]
fn a_local_build_uses_the_checkout_and_says_so() {
    let s = Scratch::new();
    let fake = Fake::new();
    let ready = build(
        &fake,
        &s.staging(),
        &s.request(Source::Local),
        &no_cancel(),
        1,
    )
    .unwrap();

    assert_eq!(ready.source, "abc1234 (local checkout)");
    assert!(fake.calls().iter().all(|c| !c.starts_with("git fetch")));
    assert!(fake.calls().iter().all(|c| !c.starts_with("git worktree")));
}

#[test]
fn a_failed_build_reports_the_tail_and_stages_nothing() {
    let s = Scratch::new();
    let mut fake = Fake::new();
    fake.fail = Some("cargo");
    let staging = s.staging();
    // An earlier staged build is forgotten when a new build starts.
    fs::create_dir_all(staging.staged_dir()).unwrap();
    fs::write(staging.binary(), "old").unwrap();

    let err = build(
        &fake,
        &staging,
        &s.request(Source::OriginMain),
        &no_cancel(),
        1,
    )
    .expect_err("cargo failed");
    let text = err.to_string();
    assert!(
        text.starts_with("cargo build failed (cargo exited with status 101)"),
        "{text}"
    );
    assert!(text.contains("error line 29"));
    assert!(
        !text.contains("error line 9\n"),
        "only the last {BUILD_TAIL_LINES} lines"
    );
    assert_eq!(staging.ready(), None);
    assert!(
        !staging.src().exists(),
        "the worktree is removed after a failure too"
    );
}

#[test]
fn a_fetch_failure_names_the_step() {
    let s = Scratch::new();
    let mut fake = Fake::new();
    fake.fail = Some("git fetch");
    let err = build(
        &fake,
        &s.staging(),
        &s.request(Source::OriginMain),
        &no_cancel(),
        1,
    )
    .expect_err("fetch failed");
    assert!(err.to_string().starts_with("fetching origin/main failed"));
    assert!(err.to_string().contains("could not read from remote"));
}

#[test]
fn a_cancelled_build_is_not_an_error_item() {
    let s = Scratch::new();
    let mut fake = Fake::new();
    fake.cancel_cargo = true;
    let err = build(
        &fake,
        &s.staging(),
        &s.request(Source::OriginMain),
        &no_cancel(),
        1,
    )
    .expect_err("cancelled");
    assert!(matches!(err, BuildError::Cancelled));
    assert!(!s.staging().src().exists());
}

#[test]
fn a_folder_that_is_not_dovetail_is_refused_before_anything_runs() {
    let s = Scratch::new();
    fs::remove_file(s.repo.join("crates/dovetail/Cargo.toml")).unwrap();
    let fake = Fake::new();
    let err = build(
        &fake,
        &s.staging(),
        &s.request(Source::OriginMain),
        &no_cancel(),
        1,
    )
    .expect_err("not dovetail");
    assert!(matches!(
        err,
        BuildError::Upgrade(UpgradeError::NotDovetail(_))
    ));
    assert!(fake.calls().is_empty());
}

#[test]
fn two_builds_do_not_share_the_staging_folder() {
    let s = Scratch::new();
    let staging = s.staging();
    fs::create_dir_all(&staging.root).unwrap();
    // This very process holds the lock: it is alive.
    fs::write(staging.lock(), std::process::id().to_string()).unwrap();
    let err = build(
        &Fake::new(),
        &staging,
        &s.request(Source::OriginMain),
        &no_cancel(),
        1,
    )
    .expect_err("busy");
    assert!(matches!(err, BuildError::Upgrade(UpgradeError::Busy(_))));
    // A lock whose process is gone is taken over.
    fs::write(staging.lock(), "999999999").unwrap();
    build(
        &Fake::new(),
        &staging,
        &s.request(Source::OriginMain),
        &no_cancel(),
        1,
    )
    .unwrap();
}

struct Swapping {
    s: Scratch,
    install: PathBuf,
    proc_root: PathBuf,
}

impl Swapping {
    fn new() -> Self {
        let s = Scratch::new();
        let install = s.paths.data.join("home/.local/bin/dovetail");
        fs::create_dir_all(install.parent().unwrap()).unwrap();
        fs::write(&install, "old binary").unwrap();
        let proc_root = s.paths.data.join("proc");
        fs::create_dir_all(&proc_root).unwrap();
        build(
            &Fake::new(),
            &s.staging(),
            &s.request(Source::OriginMain),
            &no_cancel(),
            1,
        )
        .unwrap();
        Self {
            s,
            install,
            proc_root,
        }
    }

    fn guard(&self) -> SwapGuard {
        SwapGuard {
            runtime: self.s.paths.runtime.clone(),
            proc_root: self.proc_root.clone(),
            own_pid: 100,
            wait: Duration::ZERO,
        }
    }

    /// A fake `/proc/<pid>/exe` pointing at `exe`.
    fn process(&self, pid: u32, exe: &str) {
        let dir = self.proc_root.join(pid.to_string());
        fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(exe, dir.join("exe")).unwrap();
    }
}

#[test]
fn the_staged_binary_replaces_the_installed_one_and_is_forgotten() {
    let w = Swapping::new();
    // Our own process runs the installed binary too; it does not count.
    w.process(100, &w.install.canonicalize().unwrap().to_string_lossy());
    // Another binary does not count either.
    w.process(200, "/usr/bin/other");

    let swap = install_staged(&w.s.staging(), &w.install, &w.guard()).expect("install");

    assert!(matches!(swap, Swap::Installed(r) if r.source == "abc1234"));
    assert_eq!(fs::read_to_string(&w.install).unwrap(), "new binary");
    assert_eq!(w.s.staging().ready(), None);
    // No temp file is left next to it.
    let left: Vec<_> = fs::read_dir(w.install.parent().unwrap()).unwrap().collect();
    assert_eq!(left.len(), 1);
}

#[test]
fn a_running_helper_blocks_the_swap_and_keeps_the_build_staged() {
    let w = Swapping::new();
    let real = w.install.canonicalize().unwrap();
    w.process(300, &real.to_string_lossy());
    w.process(301, &format!("{} (deleted)", real.display()));

    let err = install_staged(&w.s.staging(), &w.install, &w.guard()).expect_err("blocked");

    assert!(
        matches!(&err, SwapError::Blocked(b) if b == &["pid 300", "pid 301"]),
        "{err}"
    );
    assert_eq!(fs::read_to_string(&w.install).unwrap(), "old binary");
    assert!(w.s.staging().ready().is_some());
}

#[test]
fn a_listening_app_socket_blocks_the_swap_but_a_stale_one_does_not() {
    let w = Swapping::new();
    let stale = w.s.paths.runtime.join("old.sock");
    drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
    install_staged(&w.s.staging(), &w.install, &w.guard()).expect("stale socket is no app");

    let w = Swapping::new();
    let live = w.s.paths.runtime.join("app.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&live).unwrap();
    let err = install_staged(&w.s.staging(), &w.install, &w.guard()).expect_err("blocked");
    assert!(matches!(&err, SwapError::Blocked(b) if b.len() == 1 && b[0].ends_with("app.sock")));
}

#[test]
fn nothing_staged_means_nothing_to_do() {
    let w = Swapping::new();
    w.s.staging().clear();
    assert_eq!(
        install_staged(&w.s.staging(), &w.install, &w.guard()).unwrap(),
        Swap::NothingStaged
    );
    assert_eq!(fs::read_to_string(&w.install).unwrap(), "old binary");
}

#[test]
fn a_first_install_creates_the_bin_folder() {
    let w = Swapping::new();
    let fresh = w.s.paths.data.join("fresh/.local/bin/dovetail");
    install_staged(&w.s.staging(), &fresh, &w.guard()).unwrap();
    assert_eq!(fs::read_to_string(fresh).unwrap(), "new binary");
}

#[test]
fn only_the_installed_binary_counts_as_installed() {
    let w = Swapping::new();
    let other = w.s.paths.data.join("target-debug-dovetail");
    fs::write(&other, "dev").unwrap();
    assert!(is_installed_binary(&w.install, &w.install));
    assert!(!is_installed_binary(&other, &w.install));
    assert!(!is_installed_binary(
        &w.install,
        &w.s.paths.data.join("missing")
    ));
}
