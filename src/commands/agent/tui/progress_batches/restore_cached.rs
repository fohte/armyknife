use crate::commands::agent::types::Session;

use super::super::progress::FetchedDelegate;
use super::{ImplementationItem, ImplementationPlan, UnmatchedDelegate};

impl ImplementationPlan {
    pub(in crate::commands::agent::tui) fn restore_cached_unmatched_delegates(
        &mut self,
        cached: &Self,
        cached_sessions: &[Session],
    ) -> Vec<FetchedDelegate> {
        let mut restored = Vec::new();
        for cached_delegate in &cached.unmatched_delegates {
            let Some(session) = cached_sessions
                .iter()
                .find(|session| session.session_id == cached_delegate.session_id)
            else {
                continue;
            };

            let existing = self
                .unmatched_delegates
                .iter_mut()
                .find(|delegate| delegate.session_id == cached_delegate.session_id);
            let (pull_request, lookup_unavailable) = if let Some(existing) = existing {
                if existing.pull_request.is_none() {
                    existing
                        .pull_request
                        .clone_from(&cached_delegate.pull_request);
                }
                existing.lookup_unavailable = existing.pull_request.is_none()
                    && (existing.lookup_unavailable || cached_delegate.lookup_unavailable);
                (existing.pull_request.clone(), existing.lookup_unavailable)
            } else {
                if self
                    .batch_id_for_delegate(&cached_delegate.session_id)
                    .is_some()
                    || self
                        .batches
                        .iter()
                        .flat_map(|batch| &batch.items)
                        .any(|item| item_covers_delegate(item, cached_delegate))
                {
                    continue;
                }
                self.unmatched_delegates.push(cached_delegate.clone());
                (
                    cached_delegate.pull_request.clone(),
                    cached_delegate.lookup_unavailable,
                )
            };

            restored.push(FetchedDelegate {
                session: session.clone(),
                checklist_item_id: None,
                pull_request,
                lookup_unavailable,
            });
        }
        restored
    }
}

fn item_covers_delegate(item: &ImplementationItem, delegate: &UnmatchedDelegate) -> bool {
    let Some(url) = delegate
        .pull_request
        .as_ref()
        .map(|pr| pr.pull_request.url.as_str())
    else {
        return false;
    };

    item.pull_request
        .as_ref()
        .is_some_and(|pr| pr.pull_request.url == url)
        || item
            .github_link
            .as_ref()
            .and_then(|link| link.url.as_deref())
            == Some(url)
}
