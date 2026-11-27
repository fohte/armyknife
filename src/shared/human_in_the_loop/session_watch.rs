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

#[derive(Debug, PartialEq, Eq)]
pub(super) enum SessionResolution {
    Found(u32),
    Starting,
    Closed,
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
    pub(super) fn resolve(
        &mut self,
        now: Instant,
        query: impl FnOnce() -> Result<Option<u32>>,
    ) -> Result<SessionResolution> {
        if let Some(pid) = query()? {
            self.seen = true;
            return Ok(SessionResolution::Found(pid));
        }
        Ok(if self.seen || now >= self.startup_deadline {
            SessionResolution::Closed
        } else {
            SessionResolution::Starting
        })
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::super::HumanInTheLoopError;
    use super::*;

    #[rstest]
    #[case::starting_up(false, None, false, SessionResolutionKind::Starting)]
    #[case::never_started(false, None, true, SessionResolutionKind::Closed)]
    #[case::session_found(false, Some(42), true, SessionResolutionKind::Found(42))]
    #[case::ended_after_running(true, None, false, SessionResolutionKind::Closed)]
    fn resolve_distinguishes_startup_found_and_closed_sessions(
        #[case] seen: bool,
        #[case] pid: Option<u32>,
        #[case] after_deadline: bool,
        #[case] expected: SessionResolutionKind,
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

        let result = watch
            .resolve(now, || Ok(pid))
            .map(|resolution| match resolution {
                SessionResolution::Found(pid) => SessionResolutionKind::Found(pid),
                SessionResolution::Starting => SessionResolutionKind::Starting,
                SessionResolution::Closed => SessionResolutionKind::Closed,
            })
            .map_err(|error| error.to_string());
        assert_eq!(result, Ok(expected));
    }

    #[test]
    fn resolve_propagates_query_failure() {
        let mut watch = SessionWatch::new();

        assert_eq!(
            watch
                .resolve(Instant::now(), || Err(HumanInTheLoopError::CommandFailed(
                    "shpool is unavailable".into()
                )))
                .map_err(|error| error.to_string()),
            Err("Command failed: shpool is unavailable".to_string()),
        );
    }

    #[derive(Debug, PartialEq, Eq)]
    enum SessionResolutionKind {
        Found(u32),
        Starting,
        Closed,
    }
}
