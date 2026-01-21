//! ChangeSet: Represents all detected changes between local and remote state.

use crate::commands::gh::issue_agent::models::{Comment, Issue, IssueMetadata};
use crate::commands::gh::issue_agent::storage::{IssueStorage, LocalComment};
use crate::infra::github::{CommentClient, IssueClient};

use super::detect::{
    detect_body_change, detect_comment_changes, detect_label_change, detect_title_change,
};
use super::diff::print_diff;

/// Local state for change detection.
pub(super) struct LocalState<'a> {
    pub(super) metadata: &'a IssueMetadata,
    pub(super) body: &'a str,
    pub(super) comments: &'a [LocalComment],
}

/// Remote state for change detection.
pub(super) struct RemoteState<'a> {
    pub(super) issue: &'a Issue,
    pub(super) comments: &'a [Comment],
}

/// Options for change detection.
pub(super) struct DetectOptions<'a> {
    pub(super) current_user: &'a str,
    pub(super) edit_others: bool,
    pub(super) allow_delete: bool,
}

/// Represents all changes detected between local and remote state.
pub(super) struct ChangeSet<'a> {
    pub(super) body: Option<BodyChange<'a>>,
    pub(super) title: Option<TitleChange<'a>>,
    pub(super) labels: Option<LabelChange>,
    pub(super) comments: Vec<CommentChange<'a>>,
}

pub(super) struct BodyChange<'a> {
    pub(super) local: &'a str,
    pub(super) remote: &'a str,
}

pub(super) struct TitleChange<'a> {
    pub(super) local: &'a str,
    pub(super) remote: &'a str,
}

pub(super) struct LabelChange {
    pub(super) to_add: Vec<String>,
    pub(super) to_remove: Vec<String>,
    pub(super) local_sorted: Vec<String>,
    pub(super) remote_sorted: Vec<String>,
}

#[derive(Debug)]
pub(super) enum CommentChange<'a> {
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
    pub(super) fn detect(
        local: &LocalState<'a>,
        remote: &RemoteState<'a>,
        options: &DetectOptions<'a>,
    ) -> anyhow::Result<Self> {
        let body = detect_body_change(local.body, remote.issue);
        let title = detect_title_change(local.metadata, remote.issue);
        let labels = detect_label_change(local.metadata, remote.issue);
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
            comments,
        })
    }

    pub(super) fn has_changes(&self) -> bool {
        self.body.is_some()
            || self.title.is_some()
            || self.labels.is_some()
            || !self.comments.is_empty()
    }

    pub(super) fn display(&self) {
        if let Some(change) = &self.body {
            println!();
            println!("=== Issue Body ===");
            print_diff(change.remote, change.local);
        }

        if let Some(change) = &self.title {
            println!();
            println!("=== Title ===");
            println!("- {}", change.remote);
            println!("+ {}", change.local);
        }

        if let Some(change) = &self.labels {
            println!();
            println!("=== Labels ===");
            println!("- {:?}", change.remote_sorted);
            println!("+ {:?}", change.local_sorted);
        }

        for change in &self.comments {
            match change {
                CommentChange::New { filename, body } => {
                    println!();
                    println!("=== New Comment: {} ===", filename);
                    println!("{}", body);
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
                    print_diff(remote_body, local_body);
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
                        println!("- {}", line);
                    }
                }
            }
        }
    }

    pub(super) async fn apply<C>(
        &self,
        client: &C,
        owner: &str,
        repo: &str,
        issue_number: u64,
        storage: &IssueStorage,
    ) -> anyhow::Result<()>
    where
        C: IssueClient + CommentClient,
    {
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

        Ok(())
    }
}
