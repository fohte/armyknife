use std::time::Duration;

use crate::commands::agent::review_queue::{
    self, LiveQueueRuntime, Placement, QueueRuntime, Review,
};

const SESSION_WAIT_ATTEMPTS: u32 = 50;
const SESSION_WAIT_INTERVAL: Duration = Duration::from_millis(100);

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
        for _ in 0..SESSION_WAIT_ATTEMPTS {
            if self.runtime.editor_alive(&self.key).unwrap_or(false) {
                break;
            }
            std::thread::sleep(SESSION_WAIT_INTERVAL);
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
