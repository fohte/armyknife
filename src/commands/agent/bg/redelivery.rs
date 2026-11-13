use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Args;

use super::super::peer::notify;
use super::undelivered;
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

    spawn(session_id);
}

pub(super) fn spawn(session_id: &str) {
    process::spawn_self_detached(
        "agent.bg_run.redelivery.spawn",
        "agent.bg_run.redelivery.spawn_failed",
        session_id,
        &["agent", "bg", "redeliver-detached", "--session", session_id],
    );
}

pub fn run(args: &RedeliverDetachedArgs) -> Result<()> {
    let _lock = match undelivered::try_lock_session(&args.session) {
        Ok(Some(lock)) => lock,
        Ok(None) => return Ok(()),
        Err(error) => {
            tracing::warn!(
                event = "agent.bg_run.redelivery_lock_failed",
                session = %args.session,
                error = %format!("{error:#}"),
            );
            return Ok(());
        }
    };
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
        |session_id, notification| {
            if let Some(peer_out_message_id) = notification.peer_out_message_id
                && let Err(error) =
                    crate::commands::agent::store::mark_watch_web_peer_out_message_delivered(
                        peer_out_message_id,
                        chrono::Utc::now(),
                    )
            {
                tracing::warn!(
                    event = "agent.peer.notify.record_failed",
                    session = %session_id,
                    task = %notification.task_id,
                    error = %format!("{error:#}"),
                );
            }
            if let Err(error) = undelivered::remove(session_id, &notification.task_id) {
                tracing::warn!(
                    event = "agent.bg_run.redelivery_remove_failed",
                    session = %session_id,
                    task = %notification.task_id,
                    error = %format!("{error:#}"),
                );
            }
        },
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
    mut on_success: impl FnMut(&str, &undelivered::UndeliveredNotification),
    mut now: impl FnMut() -> Instant,
    mut sleep: impl FnMut(Duration),
) {
    for notification in notifications {
        let deadline = now() + retry_window;
        let mut attempts = 0;
        let outcome = loop {
            attempts += 1;
            match send(session_id, &notification.message) {
                Ok(delivery) => break Ok(delivery),
                Err(error) => {
                    let remaining = deadline.saturating_duration_since(now());
                    if remaining.is_zero() {
                        break Err(error);
                    }
                    sleep(RETRY_INTERVAL.min(remaining));
                    if now() >= deadline {
                        break Err(error);
                    }
                }
            }
        };
        match outcome {
            Ok(delivery) => {
                tracing::info!(
                    event = "agent.bg_run.redelivered",
                    session = %session_id,
                    task = %notification.task_id,
                    delivery = ?delivery,
                );
                on_success(session_id, &notification);
            }
            Err(error) => tracing::warn!(
                event = "agent.bg_run.redelivery_failed",
                session = %session_id,
                task = %notification.task_id,
                attempts,
                error = %format!("{error:#}"),
            ),
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
            peer_out_message_id: Some(17),
        }
    }

    #[test]
    fn retries_until_delivery_is_accepted_then_removes_saved_message() {
        let mut attempts = 0;
        let mut sent = Vec::new();
        let mut sleeps = Vec::new();
        let mut removed = Vec::new();
        let mut delivered_records = Vec::new();
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
                Ok(notify::Delivery::Started)
            },
            |session_id, notification| {
                delivered_records.push(notification.peer_out_message_id);
                removed.push((session_id.to_string(), notification.task_id.clone()));
            },
            || started_at + elapsed.get(),
            |duration| {
                sleeps.push(duration);
                elapsed.set(elapsed.get() + duration);
            },
        );

        assert_eq!(
            (attempts, sent, sleeps, delivered_records, removed),
            (
                3,
                vec![
                    (SESSION_ID.to_string(), "fixture completion".to_string()),
                    (SESSION_ID.to_string(), "fixture completion".to_string()),
                    (SESSION_ID.to_string(), "fixture completion".to_string()),
                ],
                vec![RETRY_INTERVAL, RETRY_INTERVAL],
                vec![Some(17)],
                vec![(SESSION_ID.to_string(), TASK_ID.to_string())],
            ),
        );
    }

    #[test]
    fn keeps_saved_message_when_delivery_does_not_succeed() {
        let mut attempts = 0;
        let mut sleeps = Vec::new();
        let mut removed = Vec::new();
        let mut delivered_records = Vec::new();
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
            |_, notification| {
                delivered_records.push(notification.peer_out_message_id);
                removed.push(notification.task_id.clone());
            },
            || started_at + elapsed.get(),
            |duration| {
                sleeps.push(duration);
                elapsed.set(elapsed.get() + duration);
            },
        );

        assert_eq!(
            (attempts, sleeps, delivered_records, removed),
            (2, vec![RETRY_INTERVAL, RETRY_INTERVAL], vec![], vec![],),
        );
    }
}
