use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::author::{Author, WithAuthor};

/// Reference to another issue, used for sub-issue relationships.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubIssueRef {
    /// Issue internal ID (used by Sub-issues API)
    pub id: u64,
    /// Issue number
    pub number: i64,
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
}

impl SubIssueRef {
    /// Format as "owner/repo#number"
    pub fn to_ref_string(&self) -> String {
        format!("{}/{}#{}", self.owner, self.repo, self.number)
    }
}

/// Represents a GitHub Issue fetched from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub number: i64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub labels: Vec<Label>,
    pub assignees: Vec<Author>,
    pub milestone: Option<Milestone>,
    pub author: Option<Author>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Timestamp when the issue body was last edited (from GraphQL `lastEditedAt`).
    /// None if the issue has never been edited since creation.
    /// Note: GitHub API only provides a single `lastEditedAt` for body edits.
    /// Title edits are detected via `updatedAt` instead.
    #[serde(default)]
    pub last_edited_at: Option<DateTime<Utc>>,
    /// Reference to the parent issue (format: "owner/repo#number"), if this is a sub-issue.
    #[serde(default)]
    pub parent_issue: Option<SubIssueRef>,
    /// List of sub-issues (format: "owner/repo#number").
    #[serde(default)]
    pub sub_issues: Vec<SubIssueRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub title: String,
}

impl WithAuthor for Issue {
    fn author(&self) -> Option<&Author> {
        self.author.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    mod sub_issue_ref_to_ref_string {
        use super::*;

        fn sub_issue_ref(owner: &str, repo: &str, number: i64) -> SubIssueRef {
            SubIssueRef {
                id: 1,
                number,
                owner: owner.to_string(),
                repo: repo.to_string(),
            }
        }

        #[rstest]
        #[case::typical("octocat", "hello-world", 42, "octocat/hello-world#42")]
        #[case::single_digit_number("owner", "repo", 1, "owner/repo#1")]
        #[case::large_number("org", "project", 99999, "org/project#99999")]
        #[case::hyphenated_names("my-org", "my-repo", 123, "my-org/my-repo#123")]
        #[case::underscore_names("org_name", "repo_name", 7, "org_name/repo_name#7")]
        fn test_to_ref_string(
            #[case] owner: &str,
            #[case] repo: &str,
            #[case] number: i64,
            #[case] expected: &str,
        ) {
            let ref_ = sub_issue_ref(owner, repo, number);
            assert_eq!(ref_.to_ref_string(), expected);
        }
    }
}
