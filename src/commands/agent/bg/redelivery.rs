use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Args;

use super::super::peer::notify;
use super::undelivered;
#[cfg(not(test))]
use crate::infra::process;

const RETRY_INTERVAL: Duration = Duration::from_secs(1);
const RETRY_WINDOW: Duration = Duration::from_secs(30);

#[derive(Args, Clone, PartialEq, Eq)]
pub struct RedeliverDetachedArgs {
    /// Session whose saved background completion notifications should be sent.
    #[arg(long)]
    pub session: String,
}

#[cfg(not(test))]
pub(super) fn spawn_if_pending(session_id: &str) {
    match undelivered::has_pending(session_id) {
        Ok(false) => return,
        Ok(true) => {}
        Err(error) => {
            tracing::warn!(
                event = "agent.bg_run.redelivery_scan_failed",
                session = %session_id,
                error = %format!("{error:#}"),
            );
            return;
        }
    }

    process::spawn_self_detached(
        "agent.bg_run.redelivery.spawn",
        "agent.bg_run.redelivery.spawn_failed",
        session_id,
        &["agent", "bg", "redeliver-detached", "--session", session_id],
    );
}

pub fn run(args: &RedeliverDetachedArgs) -> Result<()> {
    let notifications = match undelivered::load_all(&args.session) {
        Ok(notifications) => notifications,
        Err(error) => {
            tracing::warn!(
                event = "agent.bg_run.redelivery_load_failed",
                session = %args.session,
                error = %format!("{error:#}"),
            );
            return Ok(());
        }
    };

    redeliver_messages(
        &args.session,
        notifications,
        RETRY_WINDOW,
        |session_id, message| notify::notify(session_id, message, None, None),
        undelivered::remove,
        Instant::now,
        std::thread::sleep,
    );
    Ok(())
}

fn redeliver_messages(
    session_id: &str,
    notifications: Vec<undelivered::UndeliveredNotification>,
    retry_window: Duration,
    mut send: impl FnMut(&str, &str) -> Result<notify::Delivery>,
    mut remove: impl FnMut(&str, &str) -> Result<()>,
    mut now: impl FnMut() -> Instant,
    mut sleep: impl FnMut(Duration),
) {
    for notification in notifications {
        let mut delivered = false;
        let mut last_error = None;
        let deadline = now() + retry_window;
        let mut attempts = 0;
        loop {
            if attempts > 0 && now() >= deadline {
                break;
            }
            attempts += 1;
            match send(session_id, &notification.message) {
                Ok(delivery) => {
                    tracing::info!(
                        event = "agent.bg_run.redelivered",
                        session = %session_id,
                        task = %notification.task_id,
                        delivery = ?delivery,
                    );
                    delivered = true;
                    break;
                }
                Err(error) => {
                    last_error = Some(error);
                    let remaining = deadline.saturating_duration_since(now());
                    if remaining.is_zero() {
                        break;
                    }
                    sleep(RETRY_INTERVAL.min(remaining));
                }
            }
        }

        if delivered {
            if let Err(error) = remove(session_id, &notification.task_id) {
                tracing::warn!(
                    event = "agent.bg_run.redelivery_remove_failed",
                    session = %session_id,
                    task = %notification.task_id,
                    error = %format!("{error:#}"),
                );
            }
        } else {
            tracing::warn!(
                event = "agent.bg_run.redelivery_failed",
                session = %session_id,
                task = %notification.task_id,
                attempts,
                error = %last_error
                    .map(|error| format!("{error:#}"))
                    .unwrap_or_else(|| "no delivery attempts configured".to_string()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SESSION_ID: &str = "fixture-session";
    const TASK_ID: &str = "a1b2c3d4-e5f6-4789-8abc-def012345678";

    fn notification() -> undelivered::UndeliveredNotification {
        undelivered::UndeliveredNotification {
            task_id: TASK_ID.to_string(),
            message: "fixture completion".to_string(),
        }
    }

    #[test]
    fn retries_until_delivery_is_accepted_then_removes_saved_message() {
        let mut attempts = 0;
        let mut sent = Vec::new();
        let mut sleeps = Vec::new();
        let mut removed = Vec::new();
        let elapsed = std::cell::Cell::new(Duration::ZERO);
        let started_at = Instant::now();

        redeliver_messages(
            SESSION_ID,
            vec![notification()],
            RETRY_WINDOW,
            |session_id, message| {
                attempts += 1;
                sent.push((session_id.to_string(), message.to_string()));
                if attempts < 3 {
                    anyhow::bail!("fixture delivery unavailable");
                }
                Ok(notify::Delivery::Queued {
                    reason: "fixture queue fallback".to_string(),
                })
            },
            |session_id, task_id| {
                removed.push((session_id.to_string(), task_id.to_string()));
                Ok(())
            },
            || started_at + elapsed.get(),
            |duration| {
                sleeps.push(duration);
                elapsed.set(elapsed.get() + duration);
            },
        );

        assert_eq!(
            (attempts, sent, sleeps, removed),
            (
                3,
                vec![
                    (SESSION_ID.to_string(), "fixture completion".to_string()),
                    (SESSION_ID.to_string(), "fixture completion".to_string()),
                    (SESSION_ID.to_string(), "fixture completion".to_string()),
                ],
                vec![RETRY_INTERVAL, RETRY_INTERVAL],
                vec![(SESSION_ID.to_string(), TASK_ID.to_string())],
            ),
        );
    }

    #[test]
    fn keeps_saved_message_when_delivery_does_not_succeed() {
        let mut attempts = 0;
        let mut sleeps = Vec::new();
        let mut removed = Vec::new();
        let elapsed = std::cell::Cell::new(Duration::ZERO);
        let started_at = Instant::now();

        redeliver_messages(
            SESSION_ID,
            vec![notification()],
            Duration::from_secs(2),
            |_, _| {
                attempts += 1;
                anyhow::bail!("fixture delivery unavailable");
            },
            |_, task_id| {
                removed.push(task_id.to_string());
                Ok(())
            },
            || started_at + elapsed.get(),
            |duration| {
                sleeps.push(duration);
                elapsed.set(elapsed.get() + duration);
            },
        );

        assert_eq!(
            (attempts, sleeps, removed),
            (2, vec![RETRY_INTERVAL, RETRY_INTERVAL], vec![]),
        );
    }
}
