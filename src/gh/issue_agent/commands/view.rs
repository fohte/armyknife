use clap::Args;

use crate::gh::issue_agent::format::{format_relative_time, indent_text};
use crate::gh::issue_agent::models::{Comment, Issue};
use crate::git;
use crate::github::{CommentClient, IssueClient, OctocrabClient};

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct ViewArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,
}

pub async fn run(args: &ViewArgs) -> Result<(), Box<dyn std::error::Error>> {
    let (owner, repo) = get_repo_owner_and_name(args.issue.repo.as_deref())?;
    let issue_number = args.issue.issue_number;

    let client = OctocrabClient::get()?;
    let (issue, comments) = tokio::try_join!(
        client.get_issue(&owner, &repo, issue_number),
        client.get_comments(&owner, &repo, issue_number)
    )?;

    print!("{}", format_issue_view(&issue, issue_number, &comments));

    Ok(())
}

/// Format the complete view output for an issue and its comments.
fn format_issue_view(issue: &Issue, issue_number: u64, comments: &[Comment]) -> String {
    let mut output = format_issue(issue, issue_number, comments.len());
    output.push_str(&format_comments(comments));
    output
}

fn get_repo_owner_and_name(
    repo_arg: Option<&str>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    if let Some(repo) = repo_arg {
        return repo
            .split_once('/')
            .map(|(owner, repo_name)| (owner.to_string(), repo_name.to_string()))
            .ok_or_else(|| {
                format!("Invalid repository format: {repo}. Expected owner/repo").into()
            });
    }

    // Get from current repo using git2
    let repo = git::open_repo()?;
    let (owner, name) = git::github_owner_and_repo(&repo)?;
    Ok((owner, name))
}

fn format_state(state: &str) -> &str {
    match state {
        "OPEN" => "Open",
        "CLOSED" => "Closed",
        _ => state,
    }
}

fn format_issue(issue: &Issue, issue_number: u64, comment_count: usize) -> String {
    let mut output = String::new();

    // Title line
    output.push_str(&format!("{} #{}\n", issue.title, issue_number));
    output.push('\n');

    // Status line
    let author = issue
        .author
        .as_ref()
        .map(|a| a.login.as_str())
        .unwrap_or("unknown");
    let relative_time = format_relative_time(&issue.created_at.to_rfc3339());
    let state_display = format_state(&issue.state);
    output.push_str(&format!(
        "{} • {} opened {} • {} comment{}\n",
        state_display,
        author,
        relative_time,
        comment_count,
        if comment_count == 1 { "" } else { "s" }
    ));

    // Labels
    if !issue.labels.is_empty() {
        let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
        output.push_str(&format!("Labels: {}\n", labels.join(", ")));
    }

    // Assignees
    if !issue.assignees.is_empty() {
        let assignees: Vec<&str> = issue.assignees.iter().map(|a| a.login.as_str()).collect();
        output.push_str(&format!("Assignees: {}\n", assignees.join(", ")));
    }

    // Body
    output.push('\n');
    if let Some(body) = issue.body.as_deref().filter(|b| !b.is_empty()) {
        output.push_str(&format!("{}\n", indent_text(body, "  ")));
    } else {
        output.push_str("  No description provided.\n");
    }

    output
}

fn format_comments(comments: &[Comment]) -> String {
    let mut output = String::new();

    for comment in comments {
        output.push('\n');
        output.push_str("──────────────────────────────────────────────────────\n");
        output.push('\n');

        let author = comment
            .author
            .as_ref()
            .map(|a| a.login.as_str())
            .unwrap_or("unknown");
        let relative_time = format_relative_time(&comment.created_at.to_rfc3339());
        output.push_str(&format!("{} • {}\n", author, relative_time));

        output.push('\n');
        output.push_str(&format!("{}\n", indent_text(&comment.body, "  ")));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::issue_agent::models::{Author, Label};
    use chrono::{Duration, Utc};
    use rstest::rstest;

    fn create_test_issue(
        title: &str,
        state: &str,
        body: Option<&str>,
        author: Option<&str>,
        labels: Vec<&str>,
        assignees: Vec<&str>,
    ) -> Issue {
        Issue {
            number: 1,
            title: title.to_string(),
            body: body.map(|s| s.to_string()),
            state: state.to_string(),
            labels: labels
                .into_iter()
                .map(|name| Label {
                    name: name.to_string(),
                })
                .collect(),
            assignees: assignees
                .into_iter()
                .map(|login| Author {
                    login: login.to_string(),
                })
                .collect(),
            milestone: None,
            author: author.map(|login| Author {
                login: login.to_string(),
            }),
            created_at: Utc::now() - Duration::hours(2),
            updated_at: Utc::now(),
        }
    }

    fn create_test_comment(author: Option<&str>, body: &str) -> Comment {
        Comment {
            id: "IC_123".to_string(),
            database_id: 123,
            author: author.map(|login| Author {
                login: login.to_string(),
            }),
            created_at: Utc::now() - Duration::hours(1),
            body: body.to_string(),
        }
    }

    #[rstest]
    #[case::valid("owner/repo", "owner", "repo")]
    #[case::with_dashes("my-org/my-repo", "my-org", "my-repo")]
    #[case::with_numbers("user123/project456", "user123", "project456")]
    #[case::with_dots("owner/repo.name", "owner", "repo.name")]
    fn test_get_repo_owner_and_name_with_arg(
        #[case] input: &str,
        #[case] expected_owner: &str,
        #[case] expected_repo: &str,
    ) {
        let (owner, repo) = get_repo_owner_and_name(Some(input)).unwrap();
        assert_eq!(owner, expected_owner);
        assert_eq!(repo, expected_repo);
    }

    #[rstest]
    #[case::no_slash("invalid")]
    #[case::empty("")]
    fn test_get_repo_owner_and_name_invalid(#[case] input: &str) {
        let result = get_repo_owner_and_name(Some(input));
        assert!(result.is_err());
    }

    #[rstest]
    #[case::open("OPEN", "Open")]
    #[case::closed("CLOSED", "Closed")]
    #[case::unknown("UNKNOWN", "UNKNOWN")]
    #[case::other("other", "other")]
    fn test_format_state(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(format_state(input), expected);
    }

    #[test]
    fn test_format_issue_basic() {
        let issue = create_test_issue(
            "Test Issue Title",
            "OPEN",
            Some("This is the issue body."),
            Some("testuser"),
            vec![],
            vec![],
        );

        let output = format_issue(&issue, 123, 0);

        assert!(output.contains("Test Issue Title #123"));
        assert!(output.contains("Open"));
        assert!(output.contains("testuser"));
        assert!(output.contains("0 comments"));
        assert!(output.contains("  This is the issue body."));
    }

    #[test]
    fn test_format_issue_with_labels_and_assignees() {
        let issue = create_test_issue(
            "Bug Report",
            "OPEN",
            Some("Found a bug."),
            Some("reporter"),
            vec!["bug", "urgent"],
            vec!["dev1", "dev2"],
        );

        let output = format_issue(&issue, 42, 5);

        assert!(output.contains("Bug Report #42"));
        assert!(output.contains("Labels: bug, urgent"));
        assert!(output.contains("Assignees: dev1, dev2"));
        assert!(output.contains("5 comments"));
    }

    #[test]
    fn test_format_issue_single_comment() {
        let issue = create_test_issue("Title", "CLOSED", None, None, vec![], vec![]);

        let output = format_issue(&issue, 1, 1);

        assert!(output.contains("1 comment\n")); // singular
        assert!(!output.contains("1 comments"));
    }

    #[test]
    fn test_format_issue_no_body() {
        let issue = create_test_issue("Title", "OPEN", None, Some("user"), vec![], vec![]);

        let output = format_issue(&issue, 1, 0);

        assert!(output.contains("No description provided."));
    }

    #[test]
    fn test_format_issue_empty_body() {
        let issue = create_test_issue("Title", "OPEN", Some(""), Some("user"), vec![], vec![]);

        let output = format_issue(&issue, 1, 0);

        assert!(output.contains("No description provided."));
    }

    #[test]
    fn test_format_issue_unknown_author() {
        let issue = create_test_issue("Title", "OPEN", Some("body"), None, vec![], vec![]);

        let output = format_issue(&issue, 1, 0);

        assert!(output.contains("unknown opened"));
    }

    #[test]
    fn test_format_issue_closed_state() {
        let issue = create_test_issue(
            "Title",
            "CLOSED",
            Some("body"),
            Some("user"),
            vec![],
            vec![],
        );

        let output = format_issue(&issue, 1, 0);

        assert!(output.contains("Closed"));
    }

    #[test]
    fn test_format_comments_empty() {
        let output = format_comments(&[]);
        assert!(output.is_empty());
    }

    #[test]
    fn test_format_comments_single() {
        let comments = vec![create_test_comment(Some("commenter"), "This is a comment.")];

        let output = format_comments(&comments);

        assert!(output.contains("──────────────────────────────────────────────────────"));
        assert!(output.contains("commenter"));
        assert!(output.contains("  This is a comment."));
    }

    #[test]
    fn test_format_comments_multiple() {
        let comments = vec![
            create_test_comment(Some("user1"), "First comment."),
            create_test_comment(Some("user2"), "Second comment."),
        ];

        let output = format_comments(&comments);

        assert!(output.contains("user1"));
        assert!(output.contains("First comment."));
        assert!(output.contains("user2"));
        assert!(output.contains("Second comment."));
        // Should have 2 separators
        assert_eq!(
            output
                .matches("──────────────────────────────────────────────────────")
                .count(),
            2
        );
    }

    #[test]
    fn test_format_comments_unknown_author() {
        let comments = vec![create_test_comment(None, "Anonymous comment.")];

        let output = format_comments(&comments);

        assert!(output.contains("unknown •"));
    }

    #[test]
    fn test_format_issue_view_complete() {
        let issue = create_test_issue(
            "Feature Request",
            "OPEN",
            Some("Add new feature."),
            Some("author"),
            vec!["enhancement"],
            vec![],
        );
        let comments = vec![create_test_comment(Some("reviewer"), "Looks good!")];

        let output = format_issue_view(&issue, 99, &comments);

        // Issue part
        assert!(output.contains("Feature Request #99"));
        assert!(output.contains("Labels: enhancement"));
        assert!(output.contains("Add new feature."));

        // Comment part
        assert!(output.contains("──────────────────────────────────────────────────────"));
        assert!(output.contains("reviewer"));
        assert!(output.contains("Looks good!"));
    }
}
