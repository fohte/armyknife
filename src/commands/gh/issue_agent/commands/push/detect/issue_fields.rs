use std::collections::HashSet;

use crate::commands::gh::issue_agent::body_compare::bodies_equal;
use crate::commands::gh::issue_agent::models::{Issue, IssueMetadata};

use super::super::changeset::{
    BodyChange, LabelChange, ParentIssueChange, SubIssueChange, TitleChange,
};

pub(in crate::commands::gh::issue_agent::commands::push) fn detect_body_change<'a>(
    local_body: &'a str,
    remote_issue: &'a Issue,
) -> Option<BodyChange<'a>> {
    let remote_body = remote_issue.body.as_deref().unwrap_or("");
    if bodies_equal(local_body, remote_body) {
        None
    } else {
        Some(BodyChange {
            local: local_body,
            remote: remote_body,
        })
    }
}

pub(in crate::commands::gh::issue_agent::commands::push) fn detect_title_change<'a>(
    local_metadata: &'a IssueMetadata,
    remote_issue: &'a Issue,
) -> Option<TitleChange<'a>> {
    if local_metadata.title != remote_issue.title {
        Some(TitleChange {
            local: &local_metadata.title,
            remote: &remote_issue.title,
        })
    } else {
        None
    }
}

pub(in crate::commands::gh::issue_agent::commands::push) fn detect_label_change(
    local_metadata: &IssueMetadata,
    remote_issue: &Issue,
) -> Option<LabelChange> {
    let remote_labels: HashSet<&str> = remote_issue
        .labels
        .iter()
        .map(|label| label.name.as_str())
        .collect();
    let local_labels: HashSet<&str> = local_metadata.labels.iter().map(String::as_str).collect();

    if remote_labels == local_labels {
        return None;
    }

    let to_remove: Vec<String> = remote_labels
        .difference(&local_labels)
        .map(|label| (*label).to_string())
        .collect();
    let to_add: Vec<String> = local_labels
        .difference(&remote_labels)
        .map(|label| (*label).to_string())
        .collect();

    let mut remote_sorted: Vec<String> = remote_labels.into_iter().map(str::to_string).collect();
    remote_sorted.sort();
    let mut local_sorted: Vec<String> = local_labels.into_iter().map(str::to_string).collect();
    local_sorted.sort();

    Some(LabelChange {
        to_add,
        to_remove,
        local_sorted,
        remote_sorted,
    })
}

pub(in crate::commands::gh::issue_agent::commands::push) fn detect_sub_issue_change(
    local_metadata: &IssueMetadata,
    remote_issue: &Issue,
) -> Option<SubIssueChange> {
    let remote_refs: HashSet<String> = remote_issue
        .sub_issues
        .iter()
        .map(|issue| issue.to_ref_string())
        .collect();
    let local_refs: HashSet<&str> = local_metadata
        .sub_issues
        .iter()
        .map(String::as_str)
        .collect();

    if remote_refs.len() == local_refs.len()
        && remote_refs
            .iter()
            .all(|reference| local_refs.contains(reference.as_str()))
    {
        return None;
    }

    let to_remove: Vec<String> = remote_refs
        .iter()
        .filter(|reference| !local_refs.contains(reference.as_str()))
        .cloned()
        .collect();
    let to_add: Vec<String> = local_refs
        .iter()
        .filter(|reference| !remote_refs.contains(**reference))
        .map(|reference| (*reference).to_string())
        .collect();

    let mut remote_sorted: Vec<String> = remote_refs.into_iter().collect();
    remote_sorted.sort();
    let mut local_sorted: Vec<String> = local_refs.into_iter().map(str::to_string).collect();
    local_sorted.sort();

    Some(SubIssueChange {
        to_add,
        to_remove,
        local_sorted,
        remote_sorted,
    })
}

pub(in crate::commands::gh::issue_agent::commands::push) fn detect_parent_issue_change(
    local_metadata: &IssueMetadata,
    remote_issue: &Issue,
) -> Option<ParentIssueChange> {
    let remote_parent = remote_issue
        .parent_issue
        .as_ref()
        .map(|reference| reference.to_ref_string());
    let local_parent = local_metadata.parent_issue.clone();

    if local_parent == remote_parent {
        return None;
    }

    Some(ParentIssueChange {
        local: local_parent,
        remote: remote_parent,
    })
}
