use tokio::sync::OwnedMutexGuard;

use super::keyed_mutex_registry::KeyedMutexRegistry;

#[derive(Clone, Default)]
pub(super) struct InterruptLocks {
    locks: KeyedMutexRegistry<String>,
}

impl InterruptLocks {
    pub(super) async fn try_acquire(&self, session_id: &str) -> Option<OwnedMutexGuard<()>> {
        self.locks
            .get_or_create(session_id.to_owned())
            .await
            .try_lock_owned()
            .ok()
    }
}
