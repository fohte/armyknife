//! ChangeSet: Represents all detected changes between local and remote state.

use crate::gh::issue_agent::models::{Comment, Issue, IssueMetadata};
use crate::gh::issue_agent::storage::{IssueStorage, LocalComment};
use crate::github::{CommentClient, IssueClient};

use super::detect::{
    detect_body_change, detect_comment_changes, detect_label_change, detect_title_change,
};
use super::diff::print_diff;

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
}

impl<'a> ChangeSet<'a> {
    pub(super) fn detect(
        local_metadata: &'a IssueMetadata,
        local_body: &'a str,
        local_comments: &'a [LocalComment],
        remote_issue: &'a Issue,
        remote_comments: &'a [Comment],
        current_user: &'a str,
        edit_others: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let body = detect_body_change(local_body, remote_issue);
        let title = detect_title_change(local_metadata, remote_issue);
        let labels = detect_label_change(local_metadata, remote_issue);
        let comments =
            detect_comment_changes(local_comments, remote_comments, current_user, edit_others)?;

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
    ) -> Result<(), Box<dyn std::error::Error>>
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
            }
        }

        Ok(())
    }
}
