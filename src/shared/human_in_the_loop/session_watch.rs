use std::time::{Duration, Instant};

use super::Result;

/// The floating pane's shpool client creates the editor session shortly after
/// the pane opens.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Tracks whether the shpool session running a review editor is still alive.
pub(super) struct SessionWatch {
    seen: bool,
    startup_deadline: Instant,
}

impl SessionWatch {
    pub(super) fn new() -> Self {
        Self {
            seen: false,
            startup_deadline: Instant::now() + STARTUP_TIMEOUT,
        }
    }

    /// A session that has never been listed counts as running until the startup
    /// deadline. Once seen, its absence means the editor ended.
    pub(super) fn is_running(
        &mut self,
        now: Instant,
        query: impl FnOnce() -> Result<bool>,
    ) -> Result<bool> {
        let running = query()?;
        self.seen |= running;
        Ok(running || (!self.seen && now < self.startup_deadline))
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::super::HumanInTheLoopError;
    use super::*;

    #[rstest]
    #[case::starting_up(false, false, false, true)]
    #[case::never_started(false, false, true, false)]
    #[case::running_after_startup(false, true, true, true)]
    #[case::ended_after_running(true, false, false, false)]
    fn is_running_waits_for_startup_only_until_first_seen(
        #[case] seen: bool,
        #[case] listed: bool,
        #[case] after_deadline: bool,
        #[case] expected: bool,
    ) {
        let base = Instant::now();
        let mut watch = SessionWatch {
            seen,
            startup_deadline: base + STARTUP_TIMEOUT,
        };
        let now = if after_deadline {
            base + STARTUP_TIMEOUT + Duration::from_secs(1)
        } else {
            base
        };

        assert_eq!(
            watch
                .is_running(now, || Ok(listed))
                .map_err(|error| error.to_string()),
            Ok(expected),
        );
    }

    #[test]
    fn is_running_propagates_query_failure() {
        let mut watch = SessionWatch::new();

        assert_eq!(
            watch
                .is_running(Instant::now(), || Err(HumanInTheLoopError::CommandFailed(
                    "shpool is unavailable".into()
                )))
                .map_err(|error| error.to_string()),
            Err("Command failed: shpool is unavailable".to_string()),
        );
    }
}
