use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::registrations::RelayRegistration;

const FIRST_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(10 * 60);

#[derive(Default)]
pub(super) struct UnexposeBackoff {
    failures: HashMap<RelayRegistration, Failure>,
}

struct Failure {
    next_attempt_at: Instant,
    next_delay: Duration,
}

impl UnexposeBackoff {
    pub(super) fn should_attempt(&self, registration: &RelayRegistration, now: Instant) -> bool {
        self.failures
            .get(registration)
            .is_none_or(|failure| now >= failure.next_attempt_at)
    }

    pub(super) fn record_failure(&mut self, registration: RelayRegistration, now: Instant) {
        let failure = self.failures.entry(registration).or_insert(Failure {
            next_attempt_at: now,
            next_delay: FIRST_RETRY_DELAY,
        });
        failure.next_attempt_at = now + failure.next_delay;
        failure.next_delay = failure.next_delay.saturating_mul(2).min(MAX_RETRY_DELAY);
    }

    pub(super) fn remove(&mut self, registration: &RelayRegistration) {
        self.failures.remove(registration);
    }

    pub(super) fn retain_for(&mut self, registrations: &HashSet<RelayRegistration>) {
        self.failures
            .retain(|registration, _| registrations.contains(registration));
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use rstest::{fixture, rstest};

    use super::UnexposeBackoff;
    use crate::commands::agent::watch_web::web_ui_relays::registrations::RelayRegistration;

    #[fixture]
    fn registration() -> RelayRegistration {
        RelayRegistration {
            session_id: "fixture-session".to_owned(),
            name: "Preview".to_owned(),
            url: "http://127.0.0.1:49121/preview/".to_owned(),
            port: 49121,
            exposed_url: None,
        }
    }

    #[rstest]
    fn retries_wait_two_seconds_and_double_after_each_failure(registration: RelayRegistration) {
        let start = Instant::now();
        let mut backoff = UnexposeBackoff::default();

        backoff.record_failure(registration.clone(), start);
        let first_retry = start + Duration::from_secs(2);
        let first_retry_is_due = backoff.should_attempt(&registration, first_retry);
        let before_second_failure = first_retry;
        backoff.record_failure(registration.clone(), before_second_failure);

        assert_eq!(
            (
                backoff.should_attempt(&registration, start + Duration::from_secs(1)),
                first_retry_is_due,
                backoff.should_attempt(
                    &registration,
                    before_second_failure + Duration::from_secs(3),
                ),
                backoff.should_attempt(
                    &registration,
                    before_second_failure + Duration::from_secs(4),
                ),
            ),
            (false, true, false, true),
        );
    }

    #[rstest]
    fn retry_delay_does_not_exceed_ten_minutes(registration: RelayRegistration) {
        let mut now = Instant::now();
        let mut backoff = UnexposeBackoff::default();
        let mut retry_delays = Vec::new();

        for _ in 0..12 {
            backoff.record_failure(registration.clone(), now);
            let retry_at = backoff
                .failures
                .get(&registration)
                .expect("failure should be tracked")
                .next_attempt_at;
            retry_delays.push(retry_at.duration_since(now).as_secs());
            now = retry_at;
        }

        assert_eq!(
            retry_delays,
            vec![2, 4, 8, 16, 32, 64, 128, 256, 512, 600, 600, 600]
        );
    }
}
