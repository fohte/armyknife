use std::io;
use std::thread;
use std::time::Duration;

use crate::commands::agent::auto_pause::parse_duration;
use crate::shared::config::{self, BgTaskReminderConfig};

const CHILD_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
pub(super) struct ReminderSettings {
    timeout: Option<Duration>,
    message: String,
    excluded_commands: Vec<String>,
}

impl ReminderSettings {
    pub(super) fn load() -> Self {
        let config = config::load_config_or_default();
        Self::from_config(&config.agent.bg_task_reminder)
    }

    pub(super) fn from_config(config: &BgTaskReminderConfig) -> Self {
        let timeout = config.timeout.as_deref().and_then(|timeout| {
            parse_duration(timeout).map_or_else(
                |error| {
                    tracing::warn!(
                        event = "agent.bg_task_reminder.invalid_timeout",
                        timeout,
                        %error,
                        "Disabling background task reminders",
                    );
                    None
                },
                Some,
            )
        });

        Self {
            timeout,
            message: config.message.clone(),
            excluded_commands: config.excluded_commands.clone(),
        }
    }

    pub(super) fn timeout_for(&self, command: &str) -> Option<Duration> {
        let timeout = self.timeout?;
        let excluded = self
            .excluded_commands
            .iter()
            .filter(|fragment| !fragment.is_empty())
            .any(|fragment| command.contains(fragment));
        (!excluded).then_some(timeout)
    }

    pub(super) fn message(&self) -> &str {
        &self.message
    }
}

pub(super) fn wait_until_timeout_or_exit<T>(
    timeout: Duration,
    mut try_wait: impl FnMut() -> io::Result<Option<T>>,
    mut elapsed: impl FnMut() -> Duration,
    mut sleep: impl FnMut(Duration),
    on_timeout: impl FnOnce(),
) -> io::Result<Option<T>> {
    loop {
        if let Some(status) = try_wait()? {
            return Ok(Some(status));
        }

        let elapsed_duration = elapsed();
        if elapsed_duration >= timeout {
            if let Some(status) = try_wait()? {
                return Ok(Some(status));
            }
            on_timeout();
            return Ok(None);
        }

        sleep(CHILD_POLL_INTERVAL.min(timeout - elapsed_duration));
    }
}

pub(super) fn sleep(duration: Duration) {
    thread::sleep(duration);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    use rstest::rstest;

    use crate::shared::config::BgTaskReminderConfig;

    #[rstest]
    #[case::included_command("cargo check", Some(Duration::from_secs(420)))]
    #[case::excluded_command("sample-review --wait", None)]
    fn timeout_for_respects_configured_exclusions(
        #[case] command: &str,
        #[case] expected: Option<Duration>,
    ) {
        let settings = ReminderSettings::from_config(&BgTaskReminderConfig {
            timeout: Some("7m".to_string()),
            message: "Reminder text".to_string(),
            excluded_commands: vec!["sample-review".to_string()],
        });

        assert_eq!(settings.timeout_for(command), expected);
    }

    #[test]
    fn timeout_for_is_disabled_when_timeout_is_unset() {
        let settings = ReminderSettings::from_config(&BgTaskReminderConfig::default());

        assert_eq!(settings.timeout_for("cargo check"), None);
    }

    #[test]
    fn task_that_exits_before_timeout_does_not_trigger_reminder() {
        let reminded = Cell::new(false);
        let actual = wait_until_timeout_or_exit(
            Duration::from_secs(10),
            || Ok(Some("completed")),
            || Duration::ZERO,
            |_| panic!("a completed child must not wait"),
            || reminded.set(true),
        )
        .unwrap();

        assert_eq!((actual, reminded.get()), (Some("completed"), false));
    }

    #[test]
    fn task_running_at_timeout_triggers_one_reminder() {
        let elapsed = Cell::new(Duration::ZERO);
        let polls = Cell::new(0);
        let reminders = Cell::new(0);
        let actual = wait_until_timeout_or_exit(
            Duration::from_secs(2),
            || {
                polls.set(polls.get() + 1);
                Ok::<_, io::Error>(None::<()>)
            },
            || elapsed.get(),
            |duration| elapsed.set(elapsed.get() + duration),
            || reminders.set(reminders.get() + 1),
        )
        .unwrap();

        assert_eq!(
            (actual, polls.get(), reminders.get(), elapsed.get()),
            (None, 4, 1, Duration::from_secs(2))
        );
    }

    #[test]
    fn task_exiting_at_deadline_does_not_trigger_reminder() {
        let elapsed = Cell::new(Duration::ZERO);
        let polls = Cell::new(0);
        let reminded = Cell::new(false);
        let actual = wait_until_timeout_or_exit(
            Duration::from_secs(1),
            || {
                polls.set(polls.get() + 1);
                Ok::<_, io::Error>(match polls.get() {
                    1 | 2 => None,
                    _ => Some("completed"),
                })
            },
            || elapsed.get(),
            |duration| elapsed.set(elapsed.get() + duration),
            || reminded.set(true),
        )
        .unwrap();

        assert_eq!(
            (actual, polls.get(), reminded.get()),
            (Some("completed"), 3, false)
        );
    }
}
