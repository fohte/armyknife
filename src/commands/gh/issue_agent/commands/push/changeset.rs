//! ChangeSet: Represents all detected changes between local and remote state.

use crate::commands::gh::issue_agent::models::{Comment, Issue, IssueMetadata};
use crate::commands::gh::issue_agent::storage::{IssueStorage, LocalComment};
use crate::infra::github::GitHubClient;

use crossterm::style::Color;

use super::super::common::{print_colored_line, print_diff};
use super::detect::{
    detect_body_change, detect_comment_changes, detect_label_change, detect_parent_issue_change,
    detect_sub_issue_change, detect_title_change,
};
use super::links::{
    add_sub_issue_by_ref, link_to_parent, remove_sub_issue_by_ref, unlink_from_parent,
};

/// Local state for change detection.
pub(crate) struct LocalState<'a> {
    pub(crate) metadata: &'a IssueMetadata,
    pub(crate) body: &'a str,
    pub(crate) comments: &'a [LocalComment],
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
}

/// Represents all changes detected between local and remote state.
pub(crate) struct ChangeSet<'a> {
    pub(crate) body: Option<BodyChange<'a>>,
    pub(crate) title: Option<TitleChange<'a>>,
    pub(crate) labels: Option<LabelChange>,
    pub(crate) sub_issues: Option<SubIssueChange>,
    pub(crate) parent_issue: Option<ParentIssueChange>,
    pub(crate) comments: Vec<CommentChange<'a>>,
}

pub(crate) struct BodyChange<'a> {
    pub(crate) local: &'a str,
    pub(crate) remote: &'a str,
}

pub(crate) struct TitleChange<'a> {
    pub(crate) local: &'a str,
    pub(crate) remote: &'a str,
}

pub(crate) struct LabelChange {
    pub(crate) to_add: Vec<String>,
    pub(crate) to_remove: Vec<String>,
    pub(crate) local_sorted: Vec<String>,
    pub(crate) remote_sorted: Vec<String>,
}

pub(crate) struct SubIssueChange {
    /// Sub-issues to add (ref strings like "owner/repo#number")
    pub(crate) to_add: Vec<String>,
    /// Sub-issues to remove (ref strings)
    pub(crate) to_remove: Vec<String>,
    pub(crate) local_sorted: Vec<String>,
    pub(crate) remote_sorted: Vec<String>,
}

pub(crate) struct ParentIssueChange {
    pub(crate) local: Option<String>,
    pub(crate) remote: Option<String>,
}

#[derive(Debug)]
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

impl<'a> ChangeSet<'a> {
    pub(crate) fn detect(
        local: &LocalState<'a>,
        remote: &RemoteState<'a>,
        options: &DetectOptions<'a>,
    ) -> anyhow::Result<Self> {
        let body = detect_body_change(local.body, remote.issue);
        let title = detect_title_change(local.metadata, remote.issue);
        let labels = detect_label_change(local.metadata, remote.issue);
        let sub_issues = detect_sub_issue_change(local.metadata, remote.issue);
        let parent_issue = detect_parent_issue_change(local.metadata, remote.issue);
        let comments = detect_comment_changes(
            local.comments,
            remote.comments,
            options.current_user,
            options.edit_others,
            options.allow_delete,
        )?;

        Ok(Self {
            body,
            title,
            labels,
            sub_issues,
            parent_issue,
            comments,
        })
    }

    pub(crate) fn has_changes(&self) -> bool {
        self.body.is_some()
            || self.title.is_some()
            || self.labels.is_some()
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
        owner: &str,
        repo: &str,
        issue_number: u64,
        storage: &IssueStorage,
        remote_issue: &Issue,
    ) -> anyhow::Result<()> {
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

        for change in &self.comments {
            match change {
                CommentChange::New { filename, body } => {
                    println!();
                    println!("Creating comment...");
                    client
                        .create_comment(owner, repo, issue_number, body)
                        .await?;
                    // Remove the new comment file after successful creation
                    let comment_path = storage.dir().join("comments").join(filename);
                    std::fs::remove_file(&comment_path)?;
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
            sub_issues: None,
            parent_issue: None,
            comments: vec![],
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
