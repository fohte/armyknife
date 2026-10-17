use crate::commands::gh::issue_agent::storage::IssueContent;

use super::{Conflict, ConflictCheckInput};
use crate::commands::gh::issue_agent::commands::push::field_comparison::compare_issue_fields;

pub(super) fn check_base_conflicts(
    input: &ConflictCheckInput<'_>,
    base_issue: &IssueContent,
) -> Vec<Conflict> {
    compare_issue_fields(
        input.local_metadata,
        input.local_body,
        input.remote_issue,
        base_issue,
    )
    .conflicting_fields()
    .into_iter()
    .map(|name| Conflict::IssueField { name })
    .collect()
}
