use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use super::registrations::RelayRegistration;

const FIRST_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(10 * 60);

#[derive(Default)]
pub(super) struct RetryBackoff {
    failures: HashMap<RelayRegistration, Failure>,
    sync_failure: Option<Failure>,
}

struct Failure {
    next_attempt_at: Instant,
    next_delay: Duration,
}

impl RetryBackoff {
    pub(super) fn should_attempt(&self, registration: &RelayRegistration, now: Instant) -> bool {
        self.failures
            .get(registration)
            .is_none_or(|failure| now >= failure.next_attempt_at)
    }

    pub(super) fn record_failure(&mut self, registration: RelayRegistration, now: Instant) {
        Self::record(&mut self.failures, registration, now);
    }

    pub(super) fn record_sync_failure(&mut self, now: Instant) {
        let failure = self.sync_failure.get_or_insert(Failure {
            next_attempt_at: now,
            next_delay: FIRST_RETRY_DELAY,
        });
        failure.next_attempt_at = now + failure.next_delay;
        failure.next_delay = failure.next_delay.saturating_mul(2).min(MAX_RETRY_DELAY);
    }

    pub(super) fn clear_sync_failure(&mut self) {
        self.sync_failure = None;
    }

    pub(super) fn next_attempt_at(&self) -> Option<Instant> {
        self.failures
            .values()
            .map(|failure| failure.next_attempt_at)
            .chain(
                self.sync_failure
                    .iter()
                    .map(|failure| failure.next_attempt_at),
            )
            .min()
    }

    fn record(
        failures: &mut HashMap<RelayRegistration, Failure>,
        registration: RelayRegistration,
        now: Instant,
    ) {
        let failure = failures.entry(registration).or_insert(Failure {
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

    use super::RetryBackoff;
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
        let mut backoff = RetryBackoff::default();

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
        let mut backoff = RetryBackoff::default();
        let mut eligibility = Vec::new();

        for delay in [2, 4, 8, 16, 32, 64, 128, 256, 512, 600, 600, 600] {
            backoff.record_failure(registration.clone(), now);
            let deadline = now + Duration::from_secs(delay);
            eligibility.push((
                backoff.should_attempt(&registration, deadline - Duration::from_millis(1)),
                backoff.should_attempt(&registration, deadline),
            ));
            now = deadline;
        }

        assert_eq!(eligibility, vec![(false, true); 12]);
    }

    #[rstest]
    fn next_attempt_at_returns_the_earliest_registration_deadline(registration: RelayRegistration) {
        let start = Instant::now();
        let later = RelayRegistration {
            name: "Other preview".to_owned(),
            ..registration.clone()
        };
        let mut backoff = RetryBackoff::default();
        backoff.record_failure(registration, start);
        backoff.record_failure(later, start + Duration::from_secs(1));

        assert_eq!(
            backoff.next_attempt_at(),
            Some(start + Duration::from_secs(2))
        );
    }
}
