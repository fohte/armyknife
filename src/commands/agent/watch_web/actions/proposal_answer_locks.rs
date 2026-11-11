use std::collections::HashMap;
use std::sync::{Arc, Weak};

use tokio::sync::Mutex;

type ProposalLock = Mutex<()>;
type ProposalLockMap = HashMap<(String, i64), Weak<ProposalLock>>;

#[derive(Clone, Default)]
pub(super) struct ProposalAnswerLocks {
    locks: Arc<Mutex<ProposalLockMap>>,
}

impl ProposalAnswerLocks {
    pub(super) async fn get_or_create(&self, session_id: &str, seq: i64) -> Arc<Mutex<()>> {
        let key = (session_id.to_owned(), seq);
        let mut locks = self.locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }

        let lock = Arc::new(ProposalLock::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::ProposalAnswerLocks;

    #[tokio::test]
    async fn reuses_locks_only_for_the_same_proposal() {
        let locks = ProposalAnswerLocks::default();
        let first = locks.get_or_create("sample-session", 1).await;
        let same_proposal = locks.get_or_create("sample-session", 1).await;
        let other_session = locks.get_or_create("other-session", 1).await;
        let other_proposal = locks.get_or_create("sample-session", 2).await;
        let _first_guard = first.lock().await;

        assert_eq!(
            (
                Arc::ptr_eq(&first, &same_proposal),
                Arc::ptr_eq(&first, &other_session),
                Arc::ptr_eq(&first, &other_proposal),
                same_proposal.try_lock().is_err(),
                other_session.try_lock().is_ok(),
                other_proposal.try_lock().is_ok(),
            ),
            (true, false, false, true, true, true),
        );
    }
}
