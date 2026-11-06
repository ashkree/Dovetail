//! Ending the app: sessions are asked to stop, killed if they don't, and always reaped before
//! the core thread ends (B23: sessions end with the app).

use std::time::Duration;

use crate::service::Core;

/// How long sessions get to exit on quit before they are killed.
pub(crate) const QUIT_GRACE: Duration = Duration::from_secs(3);

/// Waits for the core thread, so sessions are shut down and reaped before the app exits.
#[derive(Debug)]
pub struct CoreHandle {
    pub(crate) thread: std::thread::JoinHandle<()>,
}

impl CoreHandle {
    /// Blocks until the core has stopped or `timeout` passed. `true` if it stopped.
    pub fn wait(self, timeout: Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while !self.thread.is_finished() {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Finished, so join returns at once; a panic was already logged by the hook.
        let _ = self.thread.join();
        true
    }
}

impl Core {
    /// Asks every session to stop; the loop ends once all have exited.
    pub(crate) fn begin_quit(&mut self) {
        if self.quitting.is_some() {
            return;
        }
        for live in self.live.values_mut() {
            live.stopping = true;
            live.session.shutdown(QUIT_GRACE);
        }
        self.quitting = Some(tokio::time::Instant::now() + QUIT_GRACE + Duration::from_secs(1));
    }

    /// The quit deadline passed: kill what is left, then give the supervisors a moment to reap.
    /// `true` when the loop should stop waiting.
    pub(crate) fn quit_deadline(&mut self) -> bool {
        if self.killed {
            tracing::warn!("{} sessions still not reaped; giving up", self.live.len());
            return true;
        }
        tracing::warn!("sessions did not exit in time; killing them");
        for live in self.live.values() {
            live.session.kill();
        }
        self.killed = true;
        self.quitting = Some(tokio::time::Instant::now() + Duration::from_secs(2));
        false
    }
}
