use std::collections::HashMap;

use super::super::progress::ProgressFetchScope;
use super::super::progress_batches::ImplementationPlan;

pub(super) fn same_implementation_scope(
    cached: Option<&ProgressFetchScope>,
    current: Option<&ProgressFetchScope>,
) -> bool {
    match (cached, current) {
        (Some(cached), Some(current)) if cached.task_id.is_some() && current.task_id.is_some() => {
            cached.task_id == current.task_id
        }
        _ => cached == current,
    }
}

pub(super) fn expand_initial_target_batch(
    plan: &ImplementationPlan,
    target: Option<&str>,
    expanded_batches: &mut HashMap<String, bool>,
) {
    if let Some(batch_id) = target.and_then(|delegate_id| plan.batch_id_for_delegate(delegate_id)) {
        expanded_batches.insert(batch_id.to_string(), true);
    }
}
