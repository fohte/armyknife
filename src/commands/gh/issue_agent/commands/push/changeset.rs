//! ChangeSet: Represents all detected changes between local and remote state.

use chrono::{DateTime, Utc};

use crate::commands::gh::issue_agent::models::{Comment, Issue, IssueMetadata};
use crate::commands::gh::issue_agent::storage::{
    CommentSnapshot, IssueContent, IssueStorage, LocalComment,
};
use crate::infra::github::GitHubClient;

use crossterm::style::Color;

use super::super::common::{print_colored_line, print_diff};
use super::detect::{
    detect_body_change, detect_comment_changes_with_retry, detect_label_change,
    detect_parent_issue_change, detect_sub_issue_change, detect_title_change,
};
use super::field_comparison::{IssueFieldComparisons, compare_issue_fields};
use super::links::{
    add_sub_issue_by_ref, link_to_parent, remove_sub_issue_by_ref, unlink_from_parent,
};

/// Local state for change detection.
pub(crate) struct LocalState<'a> {
    pub(crate) metadata: &'a IssueMetadata,
    pub(crate) body: &'a str,
    pub(crate) comments: &'a [LocalComment],
    pub(crate) base_comments: Option<&'a [CommentSnapshot]>,
}

/// Remote state for change detection.
pub(crate) struct RemoteState<'a> {
    pub(crate) issue: &'a Issue,
    pub(crate) comments: &'a [Comment],
}

/// Options for change detection.
pub(crate) struct DetectOptions<'a> {
    pub(crate) current_user: &'a str,
    pub(crate) edit_others: bool,
    pub(crate) allow_delete: bool,
    pub(crate) retry_started_at: Option<DateTime<Utc>>,
}

/// Represents all changes detected between local and remote state.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ChangeSet<'a> {
    pub(crate) body: Option<BodyChange<'a>>,
    pub(crate) title: Option<TitleChange<'a>>,
    pub(crate) labels: Option<LabelChange>,
    pub(crate) assignees: Option<AssigneeChange>,
    pub(crate) milestone: Option<MilestoneChange>,
    pub(crate) sub_issues: Option<SubIssueChange>,
    pub(crate) parent_issue: Option<ParentIssueChange>,
    pub(crate) comments: Vec<CommentChange<'a>>,
}

pub(crate) struct IssueAddress<'a> {
    pub(crate) owner: &'a str,
    pub(crate) repo: &'a str,
    pub(crate) number: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BodyChange<'a> {
    pub(crate) local: &'a str,
    pub(crate) remote: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TitleChange<'a> {
    pub(crate) local: &'a str,
    pub(crate) remote: &'a str,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LabelChange {
    pub(crate) to_add: Vec<String>,
    pub(crate) to_remove: Vec<String>,
    pub(crate) local_sorted: Vec<String>,
    pub(crate) remote_sorted: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AssigneeChange {
    pub(crate) local: Vec<String>,
    pub(crate) remote_sorted: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct MilestoneChange {
    pub(crate) local: Option<String>,
    pub(crate) remote: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct SubIssueChange {
    /// Sub-issues to add (ref strings like "owner/repo#number")
    pub(crate) to_add: Vec<String>,
    /// Sub-issues to remove (ref strings)
    pub(crate) to_remove: Vec<String>,
    pub(crate) local_sorted: Vec<String>,
    pub(crate) remote_sorted: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParentIssueChange {
    pub(crate) local: Option<String>,
    pub(crate) remote: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CommentChange<'a> {
    New {
        filename: &'a str,
        body: &'a str,
    },
    Updated {
        filename: &'a str,
        local_body: &'a str,
        remote_body: &'a str,
        database_id: i64,
        author: &'a str,
        current_user: &'a str,
    },
    Deleted {
        database_id: i64,
        body: &'a str,
        author: &'a str,
    },
}

fn detect_assignee_change(local: &IssueMetadata, remote: &Issue) -> Option<AssigneeChange> {
    let mut local = local.assignees.clone();
    let mut remote_sorted = remote
        .assignees
        .iter()
        .map(|assignee| assignee.login.clone())
        .collect::<Vec<_>>();
    local.sort();
    remote_sorted.sort();
    (local != remote_sorted).then_some(AssigneeChange {
        local,
        remote_sorted,
    })
}

fn detect_milestone_change(local: &IssueMetadata, remote: &Issue) -> Option<MilestoneChange> {
    let remote = remote
        .milestone
        .as_ref()
        .map(|milestone| milestone.title.clone());
    (local.milestone != remote).then_some(MilestoneChange {
        local: local.milestone.clone(),
        remote,
    })
}

impl<'a> ChangeSet<'a> {
    pub(crate) fn detect(
        local: &LocalState<'a>,
        remote: &RemoteState<'a>,
        options: &DetectOptions<'a>,
    ) -> anyhow::Result<Self> {
        Self::detect_with_base(local, remote, options, None)
    }

    pub(crate) fn detect_with_base(
        local: &LocalState<'a>,
        remote: &RemoteState<'a>,
        options: &DetectOptions<'a>,
        base_issue: Option<&IssueContent>,
    ) -> anyhow::Result<Self> {
        let field_comparisons = base_issue
            .map(|base| compare_issue_fields(local.metadata, local.body, remote.issue, base));
        Self::detect_with_comparisons(local, remote, options, field_comparisons)
    }

    pub(super) fn detect_with_comparisons(
        local: &LocalState<'a>,
        remote: &RemoteState<'a>,
        options: &DetectOptions<'a>,
        field_comparisons: Option<IssueFieldComparisons>,
    ) -> anyhow::Result<Self> {
        let body = if field_comparisons.is_some_and(|fields| !fields.body.local_changed) {
            None
        } else {
            detect_body_change(local.body, remote.issue)
        };
        let title = if field_comparisons.is_some_and(|fields| !fields.title.local_changed) {
            None
        } else {
            detect_title_change(local.metadata, remote.issue)
        };
        let labels = if field_comparisons.is_some_and(|fields| !fields.labels.local_changed) {
            None
        } else {
            detect_label_change(local.metadata, remote.issue)
        };
        let assignees = if field_comparisons.is_some_and(|fields| !fields.assignees.local_changed) {
            None
        } else {
            detect_assignee_change(local.metadata, remote.issue)
        };
        let milestone = if field_comparisons.is_some_and(|fields| !fields.milestone.local_changed) {
            None
        } else {
            detect_milestone_change(local.metadata, remote.issue)
        };
        let sub_issues = if field_comparisons.is_some_and(|fields| !fields.sub_issues.local_changed)
        {
            None
        } else {
            detect_sub_issue_change(local.metadata, remote.issue)
        };
        let parent_issue =
            if field_comparisons.is_some_and(|fields| !fields.parent_issue.local_changed) {
                None
            } else {
                detect_parent_issue_change(local.metadata, remote.issue)
            };
        let comments = detect_comment_changes_with_retry(
            local.comments,
            remote.comments,
            local.base_comments,
            options.current_user,
            options.edit_others,
            options.allow_delete,
            options.retry_started_at,
        )?;

        Ok(Self {
            body,
            title,
            labels,
            assignees,
            milestone,
            sub_issues,
            parent_issue,
            comments,
        })
    }

    pub(crate) fn has_changes(&self) -> bool {
        self.body.is_some()
            || self.title.is_some()
            || self.labels.is_some()
            || self.assignees.is_some()
            || self.milestone.is_some()
            || self.sub_issues.is_some()
            || self.parent_issue.is_some()
            || !self.comments.is_empty()
    }

    pub(crate) fn display(&self) -> anyhow::Result<()> {
        if let Some(change) = &self.body {
            println!();
            println!("=== Issue Body ===");
            print_diff(change.remote, change.local)?;
        }

        if let Some(change) = &self.title {
            println!();
            println!("=== Title ===");
            print_colored_line("- ", change.remote, Color::Red);
            print_colored_line("+ ", change.local, Color::Green);
        }

        if let Some(change) = &self.labels {
            println!();
            println!("=== Labels ===");
            print_colored_line("- ", &format!("{:?}", change.remote_sorted), Color::Red);
            print_colored_line("+ ", &format!("{:?}", change.local_sorted), Color::Green);
        }

        if let Some(change) = &self.assignees {
            println!();
            println!("=== Assignees ===");
            print_colored_line("- ", &format!("{:?}", change.remote_sorted), Color::Red);
            print_colored_line("+ ", &format!("{:?}", change.local), Color::Green);
        }

        if let Some(change) = &self.milestone {
            println!();
            println!("=== Milestone ===");
            print_colored_line(
                "- ",
                change.remote.as_deref().unwrap_or("(none)"),
                Color::Red,
            );
            print_colored_line(
                "+ ",
                change.local.as_deref().unwrap_or("(none)"),
                Color::Green,
            );
        }

        if let Some(change) = &self.sub_issues {
            println!();
            println!("=== Sub-issues ===");
            print_colored_line("- ", &format!("{:?}", change.remote_sorted), Color::Red);
            print_colored_line("+ ", &format!("{:?}", change.local_sorted), Color::Green);
        }

        if let Some(change) = &self.parent_issue {
            println!();
            println!("=== Parent Issue ===");
            print_colored_line(
                "- ",
                change.remote.as_deref().unwrap_or("(none)"),
                Color::Red,
            );
            print_colored_line(
                "+ ",
                change.local.as_deref().unwrap_or("(none)"),
                Color::Green,
            );
        }

        for change in &self.comments {
            match change {
                CommentChange::New { filename, body } => {
                    println!();
                    println!("=== New Comment: {} ===", filename);
                    for line in body.lines() {
                        print_colored_line("+ ", line, Color::Green);
                    }
                }
                CommentChange::Updated {
                    filename,
                    local_body,
                    remote_body,
                    author,
                    current_user,
                    ..
                } => {
                    println!();
                    if author != current_user {
                        println!("=== Comment: {} (author: {}) ===", filename, author);
                    } else {
                        println!("=== Comment: {} ===", filename);
                    }
                    print_diff(remote_body, local_body)?;
                }
                CommentChange::Deleted {
                    database_id,
                    body,
                    author,
                } => {
                    println!();
                    println!(
                        "=== Delete Comment: database_id={} (author: {}) ===",
                        database_id, author
                    );
                    // Show the content that will be deleted (prefixed with -)
                    for line in body.lines() {
                        print_colored_line("- ", line, Color::Red);
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) async fn apply(
        &self,
        client: &GitHubClient,
        address: IssueAddress<'_>,
        storage: &IssueStorage,
        remote_issue: &Issue,
        remove_new_comment_files: bool,
    ) -> anyhow::Result<()> {
        let IssueAddress {
            owner,
            repo,
            number: issue_number,
        } = address;
        if let Some(change) = &self.body {
            println!();
            println!("Updating issue body...");
            client
                .update_issue_body(owner, repo, issue_number, change.local)
                .await?;
        }

        if let Some(change) = &self.title {
            println!();
            println!("Updating title...");
            client
                .update_issue_title(owner, repo, issue_number, change.local)
                .await?;
        }

        if let Some(change) = &self.labels {
            println!();
            println!("Updating labels...");
            for label in &change.to_remove {
                client
                    .remove_label(owner, repo, issue_number, label)
                    .await?;
            }
            if !change.to_add.is_empty() {
                client
                    .add_labels(owner, repo, issue_number, &change.to_add)
                    .await?;
            }
        }

        if let Some(change) = &self.assignees {
            println!();
            println!("Updating assignees...");
            client
                .update_issue_assignees(owner, repo, issue_number, &change.local)
                .await?;
        }

        if let Some(change) = &self.milestone {
            println!();
            println!("Updating milestone...");
            client
                .update_issue_milestone(owner, repo, issue_number, change.local.as_deref())
                .await?;
        }

        for change in &self.comments {
            match change {
                CommentChange::New { filename, body } => {
                    println!();
                    println!("Creating comment...");
                    client
                        .create_comment(owner, repo, issue_number, body)
                        .await?;
                    if remove_new_comment_files {
                        let comment_path = storage.dir().join("comments").join(filename);
                        std::fs::remove_file(&comment_path)?;
                    }
                }
                CommentChange::Updated {
                    database_id,
                    local_body,
                    ..
                } => {
                    println!();
                    println!("Updating comment...");
                    client
                        .update_comment(owner, repo, *database_id as u64, local_body)
                        .await?;
                }
                CommentChange::Deleted { database_id, .. } => {
                    println!();
                    println!("Deleting comment...");
                    client
                        .delete_comment(owner, repo, *database_id as u64)
                        .await?;
                }
            }
        }

        if let Some(change) = &self.sub_issues {
            println!();
            println!("Updating sub-issues...");
            for ref_str in &change.to_remove {
                remove_sub_issue_by_ref(
                    client,
                    owner,
                    repo,
                    issue_number,
                    ref_str,
                    &remote_issue.sub_issues,
                )
                .await?;
            }
            for ref_str in &change.to_add {
                add_sub_issue_by_ref(client, owner, repo, issue_number, ref_str).await?;
            }
        }

        if let Some(change) = &self.parent_issue {
            println!();
            println!("Updating parent issue...");
            let this_issue_id = client.get_issue_id(owner, repo, issue_number).await?;
            if let Some(old_parent_ref) = &change.remote {
                unlink_from_parent(client, old_parent_ref, this_issue_id).await?;
            }
            if let Some(new_parent_ref) = &change.local {
                link_to_parent(client, new_parent_ref, this_issue_id).await?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    /// Build an empty ChangeSet (no changes).
    fn empty_changeset<'a>() -> ChangeSet<'a> {
        ChangeSet {
            body: None,
            title: None,
            labels: None,
            assignees: None,
            milestone: None,
            sub_issues: None,
            parent_issue: None,
            comments: vec![],
        }
    }

    mod detect_with_base_tests {
        use super::*;
        use crate::commands::gh::issue_agent::models::IssueMetadata;
        use crate::commands::gh::issue_agent::testing::factories;
        use rstest::rstest;

        #[rstest]
        fn test_remote_only_issue_field_changes_are_omitted() {
            let base_issue = factories::base_issue_content();
            let metadata = IssueMetadata::from(base_issue.frontmatter.clone());
            let local_comments = [];
            let local = LocalState {
                metadata: &metadata,
                body: &base_issue.body,
                comments: &local_comments,
                base_comments: None,
            };
            let remote_issue = factories::issue_with_fields(
                "Remote title",
                "Remote body",
                &["remote-label"],
                Some(30),
                &[31],
            );
            let remote_comments = [];
            let remote = RemoteState {
                issue: &remote_issue,
                comments: &remote_comments,
            };
            let options = DetectOptions {
                current_user: "testuser",
                edit_others: false,
                allow_delete: false,
                retry_started_at: None,
            };

            let changeset =
                ChangeSet::detect_with_base(&local, &remote, &options, Some(&base_issue)).unwrap();

            assert_eq!(changeset, empty_changeset());
        }

        #[rstest]
        fn test_local_label_change_is_retained_against_unchanged_base() {
            let base_issue = factories::base_issue_content();
            let mut metadata = IssueMetadata::from(base_issue.frontmatter.clone());
            metadata.labels = vec!["local-label".to_string()];
            let local_comments = [];
            let local = LocalState {
                metadata: &metadata,
                body: &base_issue.body,
                comments: &local_comments,
                base_comments: None,
            };
            let remote_issue = factories::issue_with_fields(
                "Base title",
                "Base body",
                &["base-label"],
                Some(10),
                &[11],
            );
            let remote_comments = [];
            let remote = RemoteState {
                issue: &remote_issue,
                comments: &remote_comments,
            };
            let options = DetectOptions {
                current_user: "testuser",
                edit_others: false,
                allow_delete: false,
                retry_started_at: None,
            };

            let changeset =
                ChangeSet::detect_with_base(&local, &remote, &options, Some(&base_issue)).unwrap();

            assert_eq!(
                changeset,
                ChangeSet {
                    body: None,
                    title: None,
                    labels: Some(LabelChange {
                        to_add: vec!["local-label".to_string()],
                        to_remove: vec!["base-label".to_string()],
                        local_sorted: vec!["local-label".to_string()],
                        remote_sorted: vec!["base-label".to_string()],
                    }),
                    assignees: None,
                    milestone: None,
                    sub_issues: None,
                    parent_issue: None,
                    comments: vec![],
                },
            );
        }

        #[rstest]
        fn test_local_assignee_and_milestone_changes_are_retained_against_unchanged_base() {
            let base_issue = factories::base_issue_content();
            let mut metadata = IssueMetadata::from(base_issue.frontmatter.clone());
            metadata.assignees = vec!["sample-assignee".to_string()];
            metadata.milestone = Some("Sample milestone".to_string());
            let local_comments = [];
            let local = LocalState {
                metadata: &metadata,
                body: &base_issue.body,
                comments: &local_comments,
                base_comments: None,
            };
            let remote_issue = factories::issue_with_fields(
                "Base title",
                "Base body",
                &["base-label"],
                Some(10),
                &[11],
            );
            let remote_comments = [];
            let remote = RemoteState {
                issue: &remote_issue,
                comments: &remote_comments,
            };
            let options = DetectOptions {
                current_user: "testuser",
                edit_others: false,
                allow_delete: false,
                retry_started_at: None,
            };

            let changeset =
                ChangeSet::detect_with_base(&local, &remote, &options, Some(&base_issue)).unwrap();

            assert_eq!(
                changeset,
                ChangeSet {
                    body: None,
                    title: None,
                    labels: None,
                    assignees: Some(AssigneeChange {
                        local: vec!["sample-assignee".to_string()],
                        remote_sorted: vec![],
                    }),
                    milestone: Some(MilestoneChange {
                        local: Some("Sample milestone".to_string()),
                        remote: None,
                    }),
                    sub_issues: None,
                    parent_issue: None,
                    comments: vec![],
                },
            );
        }
    }

    mod apply_issue_metadata_tests {
        use super::*;
        use crate::commands::gh::issue_agent::storage::IssueStorage;
        use crate::commands::gh::issue_agent::testing::factories;
        use crate::infra::github::mock::GitHubMockServer;
        use serde_json::json;

        #[rstest]
        #[tokio::test]
        async fn apply_updates_assignees_and_milestone_by_repository_values() {
            let mock = GitHubMockServer::start().await;
            let repository = mock.repo("owner", "repo");
            repository.milestones(&[("Sample milestone", 17)]).await;
            repository
                .issue(123)
                .update_with_body(json!({ "assignees": ["sample-assignee"] }))
                .await;
            repository
                .issue(123)
                .update_with_body(json!({ "milestone": 17 }))
                .await;
            let storage_dir = tempfile::tempdir().unwrap();
            let storage = IssueStorage::from_dir(storage_dir.path());
            let changeset = ChangeSet {
                body: None,
                title: None,
                labels: None,
                assignees: Some(AssigneeChange {
                    local: vec!["sample-assignee".to_string()],
                    remote_sorted: vec![],
                }),
                milestone: Some(MilestoneChange {
                    local: Some("Sample milestone".to_string()),
                    remote: None,
                }),
                sub_issues: None,
                parent_issue: None,
                comments: vec![],
            };

            changeset
                .apply(
                    &mock.client(),
                    IssueAddress {
                        owner: "owner",
                        repo: "repo",
                        number: 123,
                    },
                    &storage,
                    &factories::issue(),
                    true,
                )
                .await
                .unwrap();

            let mut requests = mock
                .received_requests()
                .await
                .into_iter()
                .map(|request| {
                    let body = serde_json::from_slice::<serde_json::Value>(&request.body)
                        .unwrap_or(serde_json::Value::Null);
                    let mut query = request
                        .url
                        .query_pairs()
                        .map(|(key, value)| format!("{key}={value}"))
                        .collect::<Vec<_>>();
                    query.sort();
                    format!(
                        "{} {} {} {}",
                        request.method,
                        request.url.path(),
                        query.join("&"),
                        body,
                    )
                })
                .collect::<Vec<_>>();
            requests.sort();

            let mut expected = vec![
                "GET /repos/owner/repo/milestones page=1&per_page=100&state=all null".to_string(),
                "PATCH /repos/owner/repo/issues/123  {\"assignees\":[\"sample-assignee\"]}"
                    .to_string(),
                "PATCH /repos/owner/repo/issues/123  {\"milestone\":17}".to_string(),
            ];
            expected.sort();

            assert_eq!(requests, expected);
        }
    }

    mod display_tests {
        use super::*;

        #[rstest]
        fn test_display_empty_changeset() {
            let cs = empty_changeset();
            assert!(!cs.has_changes());
            // display() should succeed even with no changes
            cs.display()
                .expect("display() should not error on empty changeset");
        }

        #[rstest]
        #[case::add_sub_issues(
            vec!["owner/repo#10".to_string(), "owner/repo#20".to_string()],
            vec![],
            vec!["owner/repo#10".to_string(), "owner/repo#20".to_string()],
            vec![]
        )]
        #[case::remove_sub_issues(
            vec![],
            vec!["org/proj#5".to_string()],
            vec![],
            vec!["org/proj#5".to_string()]
        )]
        #[case::add_and_remove_sub_issues(
            vec!["a/b#1".to_string()],
            vec!["c/d#2".to_string()],
            vec!["a/b#1".to_string()],
            vec!["c/d#2".to_string()]
        )]
        fn test_display_with_sub_issues(
            #[case] local_sorted: Vec<String>,
            #[case] remote_sorted: Vec<String>,
            #[case] to_add: Vec<String>,
            #[case] to_remove: Vec<String>,
        ) {
            let mut cs = empty_changeset();
            cs.sub_issues = Some(SubIssueChange {
                to_add,
                to_remove,
                local_sorted,
                remote_sorted,
            });

            assert!(cs.has_changes());
            cs.display()
                .expect("display() should not error with sub_issues change");
        }

        #[rstest]
        #[case::set_parent(Some("owner/repo#1".to_string()), None)]
        #[case::remove_parent(None, Some("owner/repo#1".to_string()))]
        #[case::change_parent(
            Some("new-owner/new-repo#2".to_string()),
            Some("old-owner/old-repo#1".to_string())
        )]
        fn test_display_with_parent_issue(
            #[case] local: Option<String>,
            #[case] remote: Option<String>,
        ) {
            let mut cs = empty_changeset();
            cs.parent_issue = Some(ParentIssueChange { local, remote });

            assert!(cs.has_changes());
            cs.display()
                .expect("display() should not error with parent_issue change");
        }

        #[rstest]
        fn test_display_with_all_change_types() {
            let body_local = "new body";
            let body_remote = "old body";
            let title_local = "new title";
            let title_remote = "old title";

            let cs = ChangeSet {
                body: Some(BodyChange {
                    local: body_local,
                    remote: body_remote,
                }),
                title: Some(TitleChange {
                    local: title_local,
                    remote: title_remote,
                }),
                labels: Some(LabelChange {
                    to_add: vec!["new-label".to_string()],
                    to_remove: vec!["old-label".to_string()],
                    local_sorted: vec!["new-label".to_string()],
                    remote_sorted: vec!["old-label".to_string()],
                }),
                assignees: None,
                milestone: None,
                sub_issues: Some(SubIssueChange {
                    to_add: vec!["owner/repo#10".to_string()],
                    to_remove: vec!["owner/repo#5".to_string()],
                    local_sorted: vec!["owner/repo#10".to_string()],
                    remote_sorted: vec!["owner/repo#5".to_string()],
                }),
                parent_issue: Some(ParentIssueChange {
                    local: Some("new-owner/new-repo#2".to_string()),
                    remote: Some("old-owner/old-repo#1".to_string()),
                }),
                comments: vec![],
            };

            assert!(cs.has_changes());
            cs.display()
                .expect("display() should not error with all change types");
        }
    }

    mod has_changes_tests {
        use super::*;

        #[rstest]
        fn test_no_changes() {
            let cs = empty_changeset();
            assert!(!cs.has_changes());
        }

        #[rstest]
        fn test_sub_issues_counts_as_change() {
            let mut cs = empty_changeset();
            cs.sub_issues = Some(SubIssueChange {
                to_add: vec!["a/b#1".to_string()],
                to_remove: vec![],
                local_sorted: vec!["a/b#1".to_string()],
                remote_sorted: vec![],
            });
            assert!(cs.has_changes());
        }

        #[rstest]
        fn test_parent_issue_counts_as_change() {
            let mut cs = empty_changeset();
            cs.parent_issue = Some(ParentIssueChange {
                local: Some("a/b#1".to_string()),
                remote: None,
            });
            assert!(cs.has_changes());
        }
    }
}
