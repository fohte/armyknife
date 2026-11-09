use std::collections::HashSet;

use crate::commands::gh::issue_agent::body_compare::normalize_body_for_compare;
use crate::commands::gh::issue_agent::models::{Issue, IssueMetadata};
use crate::commands::gh::issue_agent::storage::IssueContent;

#[derive(Clone, Copy)]
pub(super) struct FieldComparison {
    pub(super) local_changed: bool,
    remote_changed: bool,
    sides_equal: bool,
}

impl FieldComparison {
    fn conflicts(self) -> bool {
        self.local_changed && self.remote_changed && !self.sides_equal
    }
}

#[derive(Clone, Copy)]
pub(super) struct IssueFieldComparisons {
    pub(super) body: FieldComparison,
    pub(super) title: FieldComparison,
    pub(super) labels: FieldComparison,
    pub(super) sub_issues: FieldComparison,
    pub(super) parent_issue: FieldComparison,
}

impl IssueFieldComparisons {
    pub(super) fn all_sides_equal(self) -> bool {
        self.body.sides_equal
            && self.title.sides_equal
            && self.labels.sides_equal
            && self.sub_issues.sides_equal
            && self.parent_issue.sides_equal
    }

    pub(super) fn conflicting_fields(self) -> Vec<&'static str> {
        let mut fields = Vec::new();
        if self.body.conflicts() {
            fields.push("body");
        }
        if self.title.conflicts() {
            fields.push("title");
        }
        if self.labels.conflicts() {
            fields.push("labels");
        }
        if self.sub_issues.conflicts() {
            fields.push("sub-issues");
        }
        if self.parent_issue.conflicts() {
            fields.push("parent issue");
        }
        fields
    }
}

struct IssueFieldValues {
    body: String,
    title: String,
    labels: HashSet<String>,
    sub_issues: HashSet<String>,
    parent_issue: Option<String>,
}

impl IssueFieldValues {
    fn from_local(metadata: &IssueMetadata, body: &str) -> Self {
        Self {
            body: normalize_body_for_compare(body),
            title: metadata.title.clone(),
            labels: metadata.labels.iter().cloned().collect(),
            sub_issues: metadata.sub_issues.iter().cloned().collect(),
            parent_issue: metadata.parent_issue.clone(),
        }
    }

    fn from_remote(issue: &Issue) -> Self {
        Self {
            body: normalize_body_for_compare(issue.body.as_deref().unwrap_or("")),
            title: issue.title.clone(),
            labels: issue
                .labels
                .iter()
                .map(|label| label.name.clone())
                .collect(),
            sub_issues: issue
                .sub_issues
                .iter()
                .map(|issue| issue.to_ref_string())
                .collect(),
            parent_issue: issue
                .parent_issue
                .as_ref()
                .map(|issue| issue.to_ref_string()),
        }
    }

    fn from_base(issue: &IssueContent) -> Self {
        let fields = &issue.frontmatter.fields;
        Self {
            body: normalize_body_for_compare(&issue.body),
            title: fields.title.clone(),
            labels: fields.labels.iter().cloned().collect(),
            sub_issues: fields.sub_issues.iter().cloned().collect(),
            parent_issue: fields.parent_issue.clone(),
        }
    }
}

pub(super) fn compare_issue_fields(
    local_metadata: &IssueMetadata,
    local_body: &str,
    remote_issue: &Issue,
    base_issue: &IssueContent,
) -> IssueFieldComparisons {
    let local = IssueFieldValues::from_local(local_metadata, local_body);
    let remote = IssueFieldValues::from_remote(remote_issue);
    let base = IssueFieldValues::from_base(base_issue);

    IssueFieldComparisons {
        body: compare_field(&local.body, &remote.body, &base.body),
        title: compare_field(&local.title, &remote.title, &base.title),
        labels: compare_field(&local.labels, &remote.labels, &base.labels),
        sub_issues: compare_field(&local.sub_issues, &remote.sub_issues, &base.sub_issues),
        parent_issue: compare_field(
            &local.parent_issue,
            &remote.parent_issue,
            &base.parent_issue,
        ),
    }
}

fn compare_field<T: PartialEq>(local: &T, remote: &T, base: &T) -> FieldComparison {
    FieldComparison {
        local_changed: local != base,
        remote_changed: remote != base,
        sides_equal: local == remote,
    }
}
