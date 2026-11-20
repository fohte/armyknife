use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use anyhow::anyhow;
use serde_json::Value;
use tungstenite::WebSocket;

use super::super::RequestError;
use super::thread_resume_request;

pub(super) const THREAD_RESUME_RETRY_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy)]
pub(super) struct ResumeRetryTiming {
    pub(super) timeout: Duration,
    pub(super) interval: Duration,
}

pub(super) fn is_unloaded_thread_rejection(error: &RequestError, thread_id: &str) -> bool {
    let expected_message = format!("thread not found: {thread_id}");
    matches!(
        error,
        RequestError::Rejected(rejection)
            if rejection.code == Some(-32600)
                && rejection.message.as_deref() == Some(expected_message.as_str())
    )
}

pub(super) fn resume_thread_with_retry(
    socket: &mut WebSocket<UnixStream>,
    thread_id: &str,
    timeout: Duration,
    retry_interval: Duration,
) -> std::result::Result<(Value, Vec<Value>), RequestError> {
    let started_at = Instant::now();
    let Some(deadline) = started_at.checked_add(timeout) else {
        return Err(RequestError::Transport(anyhow!(
            "Codex app-server thread/resume deadline overflowed"
        )));
    };
    let request = thread_resume_request(thread_id);
    let mut rollout_rejection_count = 0;

    loop {
        match super::super::send_request_and_collect_notifications_until(socket, &request, deadline)
        {
            Ok((response, response_notifications)) => {
                if rollout_rejection_count > 0 {
                    log_retry_summary(thread_id, rollout_rejection_count, started_at, "resumed");
                }
                return Ok((response, response_notifications));
            }
            Err(error) => {
                if !is_rollout_not_found_rejection(&error, thread_id) {
                    if rollout_rejection_count > 0 {
                        log_retry_summary(
                            thread_id,
                            rollout_rejection_count,
                            started_at,
                            "stopped",
                        );
                    }
                    return Err(error);
                }

                rollout_rejection_count += 1;
                let time_left = deadline.saturating_duration_since(Instant::now());
                if time_left <= retry_interval {
                    std::thread::sleep(time_left);
                    log_retry_summary(thread_id, rollout_rejection_count, started_at, "deadline");
                    return Err(error);
                }
                std::thread::sleep(retry_interval);
            }
        }
    }
}

fn is_rollout_not_found_rejection(error: &RequestError, thread_id: &str) -> bool {
    let expected_message = format!("no rollout found for thread id {thread_id}");
    matches!(
        error,
        RequestError::Rejected(rejection)
            if rejection.code == Some(-32600)
                && rejection.message.as_deref() == Some(expected_message.as_str())
    )
}

fn log_retry_summary(
    thread_id: &str,
    rollout_rejection_count: usize,
    started_at: Instant,
    outcome: &'static str,
) {
    tracing::debug!(
        event = "agent.codex_steer.thread_resume_retry_completed",
        session = %thread_id,
        rollout_rejections = rollout_rejection_count,
        elapsed_ms = started_at.elapsed().as_millis() as u64,
        outcome,
    );
}
