use std::time::Duration;

use crate::commands::agent::review_queue::{
    self, LiveQueueRuntime, Placement, QueueRuntime, Review,
};

const SESSION_WAIT_ATTEMPTS: u32 = 50;
const SESSION_WAIT_INTERVAL: Duration = Duration::from_millis(100);

/// Polls `check` until it returns true. An error gives up at once, since
/// retrying a failing `shpool list` only delays the launch.
fn wait_until(
    attempts: u32,
    interval: Duration,
    mut check: impl FnMut() -> anyhow::Result<bool>,
) -> bool {
    for attempt in 0..attempts {
        match check() {
            Ok(true) => return true,
            Ok(false) if attempt + 1 < attempts => std::thread::sleep(interval),
            Ok(false) | Err(_) => return false,
        }
    }
    false
}

/// An editor review queued behind the agent pane that started it. Dropping it
/// removes the review from the queue and shows the next one.
pub(super) struct QueueEntry {
    runtime: LiveQueueRuntime,
    parent_pane_id: String,
    key: String,
    placement: Placement,
}

impl QueueEntry {
    /// `None` when `parent_pane_id` is not an agent pane, whose reviews are
    /// shown right away instead of being queued.
    pub(super) fn enter(parent_pane_id: &str, session: &str, title: &str) -> Option<Self> {
        let runtime = LiveQueueRuntime::for_pane(parent_pane_id)?;
        let placement = match review_queue::editor_started(
            &runtime,
            parent_pane_id,
            Review::editor(session, title),
        ) {
            Ok(placement) => placement,
            Err(error) => {
                tracing::warn!(
                    event = "hitl.review_queue.enter_failed",
                    pane = %parent_pane_id,
                    error = %error,
                );
                return None;
            }
        };
        Some(Self {
            runtime,
            parent_pane_id: parent_pane_id.to_string(),
            key: session.to_string(),
            placement,
        })
    }

    pub(super) fn is_visible(&self) -> bool {
        self.placement == Placement::Visible
    }

    /// Updates the queue labels once the session exists. A visible pane's
    /// shpool client creates the session asynchronously, and the queue drops
    /// reviews whose session is not alive, so wait for it first.
    pub(super) fn refresh(&self) {
        let alive = wait_until(SESSION_WAIT_ATTEMPTS, SESSION_WAIT_INTERVAL, || {
            self.runtime.editor_alive(&self.key)
        });
        if !alive {
            tracing::warn!(
                event = "hitl.review_queue.session_not_alive",
                pane = %self.parent_pane_id,
                session = %self.key,
            );
        }
        if let Err(error) = review_queue::refresh(&self.runtime, &self.parent_pane_id) {
            tracing::warn!(
                event = "hitl.review_queue.refresh_failed",
                pane = %self.parent_pane_id,
                error = %error,
            );
        }
    }
}

impl Drop for QueueEntry {
    fn drop(&mut self) {
        if let Err(error) =
            review_queue::review_finished(&self.runtime, &self.parent_pane_id, &self.key)
        {
            tracing::warn!(
                event = "hitl.review_queue.leave_failed",
                pane = %self.parent_pane_id,
                error = %error,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    /// Returns each result in turn and records how many times it was polled.
    fn polled(results: Vec<anyhow::Result<bool>>, attempts: u32) -> (bool, usize) {
        let mut results = results.into_iter();
        let mut calls = 0;
        let alive = wait_until(attempts, Duration::ZERO, || {
            calls += 1;
            results.next().unwrap_or(Ok(false))
        });
        (alive, calls)
    }

    #[rstest]
    #[case::alive_at_once(vec![Ok(true)], 3, (true, 1))]
    #[case::alive_after_retries(vec![Ok(false), Ok(false), Ok(true)], 3, (true, 3))]
    #[case::times_out(vec![], 3, (false, 3))]
    #[case::gives_up_on_error(vec![Ok(false), Err(anyhow::anyhow!("shpool list failed"))], 3, (false, 2))]
    fn wait_until_polls_until_alive(
        #[case] results: Vec<anyhow::Result<bool>>,
        #[case] attempts: u32,
        #[case] expected: (bool, usize),
    ) {
        assert_eq!(polled(results, attempts), expected);
    }
}
