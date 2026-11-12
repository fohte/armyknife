use std::sync::Arc;

use tokio::sync::Mutex;

use super::keyed_mutex_registry::KeyedMutexRegistry;

#[derive(Clone, Default)]
pub(super) struct ProposalAnswerLocks {
    locks: KeyedMutexRegistry<(String, i64)>,
}

impl ProposalAnswerLocks {
    pub(super) async fn get_or_create(&self, session_id: &str, seq: i64) -> Arc<Mutex<()>> {
        self.locks.get_or_create((session_id.to_owned(), seq)).await
    }
}
