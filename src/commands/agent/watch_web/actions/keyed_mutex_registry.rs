use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Weak};

use tokio::sync::Mutex;

#[derive(Clone, Default)]
pub(super) struct KeyedMutexRegistry<K> {
    locks: Arc<Mutex<HashMap<K, Weak<Mutex<()>>>>>,
}

impl<K: Eq + Hash> KeyedMutexRegistry<K> {
    pub(super) async fn get_or_create(&self, key: K) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().await;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }

        let lock = Arc::new(Mutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::KeyedMutexRegistry;

    #[tokio::test]
    async fn reuses_locks_for_identical_keys_and_separates_independent_registries() {
        let locks = KeyedMutexRegistry::<(String, i64)>::default();
        let other_registry = KeyedMutexRegistry::<(String, i64)>::default();
        let first = locks.get_or_create(("sample-session".to_string(), 1)).await;
        let same_key = locks.get_or_create(("sample-session".to_string(), 1)).await;
        let other_session = locks.get_or_create(("other-session".to_string(), 1)).await;
        let other_item = locks.get_or_create(("sample-session".to_string(), 2)).await;
        let independent_registry = other_registry
            .get_or_create(("sample-session".to_string(), 1))
            .await;
        let _first_guard = first.lock().await;

        assert_eq!(
            (
                Arc::ptr_eq(&first, &same_key),
                Arc::ptr_eq(&first, &other_session),
                Arc::ptr_eq(&first, &other_item),
                Arc::ptr_eq(&first, &independent_registry),
                same_key.try_lock().is_err(),
                other_session.try_lock().is_ok(),
                other_item.try_lock().is_ok(),
                independent_registry.try_lock().is_ok(),
            ),
            (true, false, false, false, true, true, true, true),
        );
    }
}
