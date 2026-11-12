use std::collections::HashMap;
use std::sync::{Arc, Weak};

use tokio::sync::Mutex;

type TodoLock = Mutex<()>;
type TodoLockMap = HashMap<(String, i64), Weak<TodoLock>>;

#[derive(Clone, Default)]
pub(super) struct TodoCompletionLocks {
    locks: Arc<Mutex<TodoLockMap>>,
}

impl TodoCompletionLocks {
    pub(super) async fn get_or_create(&self, session_id: &str, seq: i64) -> Arc<Mutex<()>> {
        let key = (session_id.to_owned(), seq);
        let mut locks = self.locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }

        let lock = Arc::new(TodoLock::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::TodoCompletionLocks;

    #[tokio::test]
    async fn reuses_locks_for_the_same_todo() {
        let locks = TodoCompletionLocks::default();
        let first = locks.get_or_create("session-a", 1).await;
        let same_todo = locks.get_or_create("session-a", 1).await;
        let other_session = locks.get_or_create("session-b", 1).await;
        let other_todo = locks.get_or_create("session-a", 2).await;
        let _first_guard = first.lock().await;

        assert_eq!(
            (
                Arc::ptr_eq(&first, &same_todo),
                Arc::ptr_eq(&first, &other_session),
                Arc::ptr_eq(&first, &other_todo),
                same_todo.try_lock().is_err(),
                other_session.try_lock().is_ok(),
                other_todo.try_lock().is_ok(),
            ),
            (true, false, false, true, true, true),
        );
    }
}
