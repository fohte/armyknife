use crate::commands::gh::issue_agent::body_compare::bodies_equal;

use super::{Conflict, ConflictCheckInput, string_sets_equal};

pub(super) fn check_base_conflicts(input: &ConflictCheckInput<'_>) -> Vec<Conflict> {
    let Some(base_issue) = input.base_issue else {
        return Vec::new();
    };

    let mut conflicts = Vec::new();
    let base_fields = &base_issue.frontmatter.fields;
    let remote_body = input.remote_issue.body.as_deref().unwrap_or("");
    if !bodies_equal(input.local_body, &base_issue.body)
        && !bodies_equal(remote_body, &base_issue.body)
        && !bodies_equal(input.local_body, remote_body)
    {
        conflicts.push(Conflict::IssueField { name: "body" });
    }

    if input.local_metadata.title != base_fields.title
        && input.remote_issue.title != base_fields.title
        && input.local_metadata.title != input.remote_issue.title
    {
        conflicts.push(Conflict::IssueField { name: "title" });
    }

    let remote_labels: Vec<String> = input
        .remote_issue
        .labels
        .iter()
        .map(|label| label.name.clone())
        .collect();
    if !string_sets_equal(&input.local_metadata.labels, &base_fields.labels)
        && !string_sets_equal(&remote_labels, &base_fields.labels)
        && !string_sets_equal(&input.local_metadata.labels, &remote_labels)
    {
        conflicts.push(Conflict::IssueField { name: "labels" });
    }

    let remote_sub_issues: Vec<String> = input
        .remote_issue
        .sub_issues
        .iter()
        .map(|issue| issue.to_ref_string())
        .collect();
    if !string_sets_equal(&input.local_metadata.sub_issues, &base_fields.sub_issues)
        && !string_sets_equal(&remote_sub_issues, &base_fields.sub_issues)
        && !string_sets_equal(&input.local_metadata.sub_issues, &remote_sub_issues)
    {
        conflicts.push(Conflict::IssueField { name: "sub-issues" });
    }

    let remote_parent = input
        .remote_issue
        .parent_issue
        .as_ref()
        .map(|issue| issue.to_ref_string());
    if input.local_metadata.parent_issue != base_fields.parent_issue
        && remote_parent != base_fields.parent_issue
        && input.local_metadata.parent_issue != remote_parent
    {
        conflicts.push(Conflict::IssueField {
            name: "parent issue",
        });
    }

    conflicts
}
