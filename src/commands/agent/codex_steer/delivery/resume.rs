use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use serde_json::Value;
use tungstenite::WebSocket;

use super::super::RequestError;
use super::thread_resume_request;

pub(super) const THREAD_RESUME_RETRY_INTERVAL: Duration = Duration::from_millis(250);

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
    let deadline = Instant::now() + timeout;
    let request = thread_resume_request(thread_id);
    let mut notifications = Vec::new();
    let mut last_rollout_rejection = None;

    loop {
        match super::super::send_request_and_collect_notifications_until(socket, &request, deadline)
        {
            Ok((response, response_notifications)) => {
                notifications.extend(response_notifications);
                return Ok((response, notifications));
            }
            Err(error) if is_rollout_not_found_rejection(&error, thread_id) => {
                let time_left = deadline.saturating_duration_since(Instant::now());
                if time_left.is_zero() {
                    return Err(error);
                }
                last_rollout_rejection = Some(error);
                std::thread::sleep(retry_interval.min(time_left));
                if Instant::now() >= deadline
                    && let Some(error) = last_rollout_rejection.take()
                {
                    return Err(error);
                }
            }
            Err(error) if Instant::now() >= deadline => {
                return Err(last_rollout_rejection.take().unwrap_or(error));
            }
            Err(error) => return Err(error),
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

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use super::super::super::{RequestError, RpcRejection};
    use super::is_rollout_not_found_rejection;

    const THREAD_ID: &str = "thread-a";

    #[rstest]
    #[case::exact_rejection(-32600, "no rollout found for thread id thread-a", true)]
    #[case::wrong_code(-32601, "no rollout found for thread id thread-a", false)]
    #[case::wrong_thread(-32600, "no rollout found for thread id thread-b", false)]
    #[case::extra_message_text(-32600, "no rollout found for thread id thread-a; retry", false)]
    fn recognizes_only_exact_missing_rollout_rejection(
        #[case] code: i64,
        #[case] message: &str,
        #[case] expected: bool,
    ) {
        let error = RequestError::Rejected(RpcRejection::new(
            "thread/resume",
            &json!({"code": code, "message": message}),
        ));

        assert_eq!(is_rollout_not_found_rejection(&error, THREAD_ID), expected);
    }
}
