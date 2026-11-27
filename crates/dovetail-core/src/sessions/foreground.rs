//! Waiting for a program that has the user's terminal: the editor ([`crate::app::editor`]) and
//! native `claude` on take-over ([`crate::sessions::takeover`]).
//!
//! **Ctrl-Z.** A program that suspends itself (`kill(getpid(), SIGTSTP)`, as full-screen apps do
//! for Ctrl-Z in raw mode) stops alone. Dovetail is not a shell and never continues it, and a
//! plain `wait` does not return for a stopped child, so the terminal would hang with nothing
//! reading keys. [`wait`] sees the stop (`WUNTRACED`) and continues the program at once
//! (`SIGCONT`): Ctrl-Z does nothing there, and the program redraws.
//!
//! When the stop reaches the whole process group instead (Ctrl-Z typed while the terminal is
//! in cooked mode, or a program that signals its group, as nvim does), Dovetail stops with it
//! and the user's shell takes over as for any job; `fg` continues them all, and the `SIGCONT`
//! sent here afterwards changes nothing.

use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::{Child, ExitStatus};

use rustix::process::{Pid, Signal, WaitOptions, kill_process, waitpid};

/// Waits for `child` to exit, continuing it whenever it stops. The child is always reaped.
pub fn wait(child: Child) -> io::Result<ExitStatus> {
    let pid = i32::try_from(child.id())
        .ok()
        .and_then(Pid::from_raw)
        .ok_or_else(|| io::Error::other("the child has no usable pid"))?;
    loop {
        match waitpid(Some(pid), WaitOptions::UNTRACED) {
            Ok(Some((_, status))) if status.stopped() => {
                tracing::info!(
                    pid = pid.as_raw_nonzero().get(),
                    "a terminal program stopped itself; continuing it"
                );
                kill_process(pid, Signal::CONT)?;
            }
            Ok(Some((_, status))) => return Ok(ExitStatus::from_raw(status.as_raw())),
            // Only with `NOHANG`, which is not asked for.
            Ok(None) => {}
            Err(rustix::io::Errno::INTR) => {}
            Err(e) => return Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_child_that_stops_itself_is_continued_and_its_exit_code_kept() {
        let child = std::process::Command::new("sh")
            .args(["-c", "kill -STOP $$; exit 3"])
            .spawn()
            .expect("sh");
        let status = wait(child).expect("waited");
        assert_eq!(status.code(), Some(3));
    }

    #[test]
    fn signals_and_success_come_through() {
        let ok = std::process::Command::new("true").spawn().expect("true");
        assert!(wait(ok).expect("waited").success());
        let killed = std::process::Command::new("sh")
            .args(["-c", "kill -KILL $$"])
            .spawn()
            .expect("sh");
        assert_eq!(wait(killed).expect("waited").signal(), Some(9));
    }
}
