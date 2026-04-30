//! Helpers for applying parent/sub-issue link changes via GitHub's Sub-issues API.
//!
//! Used by [`super::changeset::ChangeSet::apply`]. Both the existing-issue
//! edit path and the new-issue create path go through `ChangeSet::apply`,
//! so these helpers run for both.

use crate::commands::gh::issue_agent::models::SubIssueRef;
use crate::infra::github::GitHubClient;

/// Parse an issue reference string `owner/repo#number` into components.
pub(super) fn parse_issue_ref(ref_str: &str) -> Option<(String, String, u64)> {
    let (repo_part, number_str) = ref_str.rsplit_once('#')?;
    let (owner, repo) = repo_part.split_once('/')?;
    let number = number_str.parse::<u64>().ok()?;
    Some((owner.to_string(), repo.to_string(), number))
}

/// Add a child issue (`child_ref`, e.g. `owner/repo#10`) to a parent issue.
pub(super) async fn add_sub_issue_by_ref(
    client: &GitHubClient,
    parent_owner: &str,
    parent_repo: &str,
    parent_number: u64,
    child_ref: &str,
) -> anyhow::Result<()> {
    let (ref_owner, ref_repo, ref_number) = parse_issue_ref(child_ref).ok_or_else(|| {
        anyhow::anyhow!(
            "Invalid sub-issue reference: '{}'. Expected format: owner/repo#number",
            child_ref
        )
    })?;
    let child_id = client
        .get_issue_id(&ref_owner, &ref_repo, ref_number)
        .await?;
    client
        .add_sub_issue(parent_owner, parent_repo, parent_number, child_id)
        .await?;
    Ok(())
}

/// Remove a child issue (resolved by `child_ref`) from a parent issue.
///
/// `existing_children` is searched for a matching entry so we can use the
/// internal sub-issue ID returned by GitHub instead of paying for an extra
/// `get_issue_id` round-trip. If the ref is not present in
/// `existing_children`, the call is a no-op.
pub(super) async fn remove_sub_issue_by_ref(
    client: &GitHubClient,
    parent_owner: &str,
    parent_repo: &str,
    parent_number: u64,
    child_ref: &str,
    existing_children: &[SubIssueRef],
) -> anyhow::Result<()> {
    if let Some(child) = existing_children
        .iter()
        .find(|r| r.to_ref_string() == child_ref)
    {
        client
            .remove_sub_issue(parent_owner, parent_repo, parent_number, child.id)
            .await?;
    }
    Ok(())
}

/// Add `this_issue_id` as a sub-issue of the parent referenced by
/// `parent_ref`. Used to set the `parentIssue` field for both create and edit
/// paths.
pub(super) async fn link_to_parent(
    client: &GitHubClient,
    parent_ref: &str,
    this_issue_id: u64,
) -> anyhow::Result<()> {
    let (ref_owner, ref_repo, ref_number) = parse_issue_ref(parent_ref).ok_or_else(|| {
        anyhow::anyhow!(
            "Invalid parent issue reference: '{}'. Expected format: owner/repo#number",
            parent_ref
        )
    })?;
    client
        .add_sub_issue(&ref_owner, &ref_repo, ref_number, this_issue_id)
        .await?;
    Ok(())
}

/// Remove `this_issue_id` as a sub-issue of the parent referenced by
/// `parent_ref`.
pub(super) async fn unlink_from_parent(
    client: &GitHubClient,
    parent_ref: &str,
    this_issue_id: u64,
) -> anyhow::Result<()> {
    let (ref_owner, ref_repo, ref_number) = parse_issue_ref(parent_ref).ok_or_else(|| {
        anyhow::anyhow!(
            "Invalid parent issue reference: '{}'. Expected format: owner/repo#number",
            parent_ref
        )
    })?;
    client
        .remove_sub_issue(&ref_owner, &ref_repo, ref_number, this_issue_id)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::valid("owner/repo#123", Some(("owner".to_string(), "repo".to_string(), 123)))]
    #[case::large_number("org/project#99999", Some(("org".to_string(), "project".to_string(), 99999)))]
    #[case::missing_hash("owner/repo", None)]
    #[case::missing_slash("ownerrepo#1", None)]
    #[case::non_numeric_number("owner/repo#abc", None)]
    #[case::empty_string("", None)]
    fn test_parse_issue_ref(#[case] input: &str, #[case] expected: Option<(String, String, u64)>) {
        assert_eq!(parse_issue_ref(input), expected);
    }
}
