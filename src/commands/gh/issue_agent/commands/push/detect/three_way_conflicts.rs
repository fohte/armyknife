use super::super::field_comparison::IssueFieldComparisons;
use super::Conflict;

pub(super) fn check_base_conflicts(field_comparisons: IssueFieldComparisons) -> Vec<Conflict> {
    field_comparisons
        .conflicting_fields()
        .into_iter()
        .map(|name| Conflict::IssueField { name })
        .collect()
}
