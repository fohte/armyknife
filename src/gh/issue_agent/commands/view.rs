use clap::Args;

use super::common::{get_repo_from_arg_or_git, parse_repo};
use crate::gh::issue_agent::format::{format_relative_time, indent_text};
use crate::gh::issue_agent::models::{Comment, Issue};
use crate::github::{CommentClient, IssueClient, OctocrabClient};

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct ViewArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,
}

pub async fn run(args: &ViewArgs) -> Result<(), Box<dyn std::error::Error>> {
    let client = OctocrabClient::get()?;
    let output = run_with_client_and_output(args, client).await?;
    print!("{}", output);
    Ok(())
}

/// Internal implementation that returns the formatted output for testability.
#[cfg_attr(not(test), allow(dead_code))]
pub(super) async fn run_with_client_and_output<C>(
    args: &ViewArgs,
    client: &C,
) -> Result<String, Box<dyn std::error::Error>>
where
    C: IssueClient + CommentClient,
{
    run_with_client_and_output_with(args, client, format_relative_time).await
}

/// Internal implementation that accepts a custom time formatter for testability.
async fn run_with_client_and_output_with<C, F>(
    args: &ViewArgs,
    client: &C,
    time_formatter: F,
) -> Result<String, Box<dyn std::error::Error>>
where
    C: IssueClient + CommentClient,
    F: Fn(&str) -> String,
{
    let repo_str = get_repo_from_arg_or_git(&args.issue.repo)?;
    let (owner, repo) = parse_repo(&repo_str)?;
    let issue_number = args.issue.issue_number;

    let (issue, comments) = tokio::try_join!(
        client.get_issue(&owner, &repo, issue_number),
        client.get_comments(&owner, &repo, issue_number)
    )?;

    Ok(format_issue_view_with(
        &issue,
        issue_number,
        &comments,
        time_formatter,
    ))
}

/// Format the complete view output for an issue and its comments.
#[allow(dead_code)]
fn format_issue_view(issue: &Issue, issue_number: u64, comments: &[Comment]) -> String {
    format_issue_view_with(issue, issue_number, comments, format_relative_time)
}

/// Testable version that accepts a custom time formatter.
fn format_issue_view_with<F>(
    issue: &Issue,
    issue_number: u64,
    comments: &[Comment],
    time_formatter: F,
) -> String
where
    F: Fn(&str) -> String,
{
    let mut output = format_issue_with(issue, issue_number, comments.len(), &time_formatter);
    output.push_str(&format_comments_with(comments, &time_formatter));
    output
}

fn format_state(state: &str) -> &str {
    match state {
        "OPEN" => "Open",
        "CLOSED" => "Closed",
        _ => state,
    }
}

#[allow(dead_code)]
fn format_issue(issue: &Issue, issue_number: u64, comment_count: usize) -> String {
    format_issue_with(issue, issue_number, comment_count, format_relative_time)
}

fn format_issue_with<F>(
    issue: &Issue,
    issue_number: u64,
    comment_count: usize,
    time_formatter: F,
) -> String
where
    F: Fn(&str) -> String,
{
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
    let relative_time = time_formatter(&issue.created_at.to_rfc3339());
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

#[allow(dead_code)]
fn format_comments(comments: &[Comment]) -> String {
    format_comments_with(comments, format_relative_time)
}

fn format_comments_with<F>(comments: &[Comment], time_formatter: F) -> String
where
    F: Fn(&str) -> String,
{
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
        let relative_time = time_formatter(&comment.created_at.to_rfc3339());
        output.push_str(&format!("{} • {}\n", author, relative_time));

        output.push('\n');
        output.push_str(&format!("{}\n", indent_text(&comment.body, "  ")));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::factories;
    use rstest::rstest;

    /// Fixed time formatter for deterministic test output.
    fn fixed_time(_: &str) -> String {
        "2 hours ago".to_string()
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
        let issue = factories::issue_with(|i| {
            i.title = "Test Issue Title".to_string();
            i.body = Some("This is the issue body.".to_string());
        });

        let output = format_issue_with(&issue, 123, 0, fixed_time);

        assert_eq!(
            output,
            "\
Test Issue Title #123

Open • testuser opened 2 hours ago • 0 comments

  This is the issue body.
"
        );
    }

    #[test]
    fn test_format_issue_with_labels_and_assignees() {
        let issue = factories::issue_with(|i| {
            i.title = "Bug Report".to_string();
            i.body = Some("Found a bug.".to_string());
            i.author = Some(factories::author("reporter"));
            i.labels = factories::labels(&["bug", "urgent"]);
            i.assignees = factories::assignees(&["dev1", "dev2"]);
        });

        let output = format_issue_with(&issue, 42, 5, fixed_time);

        assert_eq!(
            output,
            "\
Bug Report #42

Open • reporter opened 2 hours ago • 5 comments
Labels: bug, urgent
Assignees: dev1, dev2

  Found a bug.
"
        );
    }

    #[rstest]
    #[case::zero(0, "Open • testuser opened 2 hours ago • 0 comments\n")]
    #[case::one(1, "Open • testuser opened 2 hours ago • 1 comment\n")]
    #[case::two(2, "Open • testuser opened 2 hours ago • 2 comments\n")]
    #[case::many(10, "Open • testuser opened 2 hours ago • 10 comments\n")]
    fn test_format_issue_comment_count(#[case] count: usize, #[case] expected: &str) {
        let issue = factories::issue();
        let output = format_issue_with(&issue, 1, count, fixed_time);
        assert_eq!(
            output,
            format!(
                "\
Test Issue #1

{expected}
  Test body
"
            )
        );
    }

    #[rstest]
    #[case::none(None, "  No description provided.\n")]
    #[case::empty(Some(""), "  No description provided.\n")]
    #[case::with_body(Some("Issue body"), "  Issue body\n")]
    fn test_format_issue_body(#[case] body: Option<&str>, #[case] expected_body: &str) {
        let issue = factories::issue_with(|i| i.body = body.map(|s| s.to_string()));
        let output = format_issue_with(&issue, 1, 0, fixed_time);
        assert_eq!(
            output,
            format!(
                "\
Test Issue #1

Open • testuser opened 2 hours ago • 0 comments

{expected_body}"
            )
        );
    }

    #[rstest]
    #[case::with_author(Some("testuser"), "testuser")]
    #[case::no_author(None, "unknown")]
    fn test_format_issue_author(#[case] author: Option<&str>, #[case] expected_author: &str) {
        let issue = factories::issue_with(|i| i.author = author.map(factories::author));
        let output = format_issue_with(&issue, 1, 0, fixed_time);
        assert_eq!(
            output,
            format!(
                "\
Test Issue #1

Open • {expected_author} opened 2 hours ago • 0 comments

  Test body
"
            )
        );
    }

    #[rstest]
    #[case::open("OPEN", "Open")]
    #[case::closed("CLOSED", "Closed")]
    fn test_format_issue_state_display(#[case] state: &str, #[case] expected_state: &str) {
        let issue = factories::issue_with(|i| i.state = state.to_string());
        let output = format_issue_with(&issue, 1, 0, fixed_time);
        assert_eq!(
            output,
            format!(
                "\
Test Issue #1

{expected_state} • testuser opened 2 hours ago • 0 comments

  Test body
"
            )
        );
    }

    #[test]
    fn test_format_comments_empty() {
        let output = format_comments_with(&[], fixed_time);
        assert_eq!(output, "");
    }

    #[test]
    fn test_format_comments_single() {
        let comments = vec![factories::comment_with(|c| {
            c.author = Some(factories::author("commenter"));
            c.body = "This is a comment.".to_string();
        })];

        let output = format_comments_with(&comments, fixed_time);

        assert_eq!(
            output,
            "
──────────────────────────────────────────────────────

commenter • 2 hours ago

  This is a comment.
"
        );
    }

    #[test]
    fn test_format_comments_multiple() {
        let comments = vec![
            factories::comment_with(|c| {
                c.author = Some(factories::author("user1"));
                c.body = "First comment.".to_string();
            }),
            factories::comment_with(|c| {
                c.author = Some(factories::author("user2"));
                c.body = "Second comment.".to_string();
            }),
        ];

        let output = format_comments_with(&comments, fixed_time);

        assert_eq!(
            output,
            "
──────────────────────────────────────────────────────

user1 • 2 hours ago

  First comment.

──────────────────────────────────────────────────────

user2 • 2 hours ago

  Second comment.
"
        );
    }

    #[rstest]
    #[case::with_author(Some("commenter"), "commenter")]
    #[case::no_author(None, "unknown")]
    fn test_format_comments_author(#[case] author: Option<&str>, #[case] expected_author: &str) {
        let comments = vec![factories::comment_with(|c| {
            c.author = author.map(factories::author);
            c.body = "Comment body.".to_string();
        })];
        let output = format_comments_with(&comments, fixed_time);
        assert_eq!(
            output,
            format!(
                "
──────────────────────────────────────────────────────

{expected_author} • 2 hours ago

  Comment body.
"
            )
        );
    }

    #[test]
    fn test_format_issue_view_complete() {
        let issue = factories::issue_with(|i| {
            i.title = "Feature Request".to_string();
            i.body = Some("Add new feature.".to_string());
            i.author = Some(factories::author("author"));
            i.labels = factories::labels(&["enhancement"]);
        });
        let comments = vec![factories::comment_with(|c| {
            c.author = Some(factories::author("reviewer"));
            c.body = "Looks good!".to_string();
        })];

        let output = format_issue_view_with(&issue, 99, &comments, fixed_time);

        assert_eq!(
            output,
            "\
Feature Request #99

Open • author opened 2 hours ago • 1 comment
Labels: enhancement

  Add new feature.

──────────────────────────────────────────────────────

reviewer • 2 hours ago

  Looks good!
"
        );
    }

    mod run_with_client_tests {
        use super::*;
        use crate::gh::issue_agent::commands::IssueArgs;
        use crate::github::MockGitHubClient;
        use chrono::{TimeZone, Utc};

        fn mock_issue(number: i64, title: &str, body: &str) -> Issue {
            factories::issue_with(|i| {
                i.number = number;
                i.title = title.to_string();
                i.body = Some(body.to_string());
                i.labels = factories::labels(&["bug"]);
                i.created_at = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
                i.updated_at = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
            })
        }

        fn mock_comment(id: &str, database_id: i64, author: &str, body: &str) -> Comment {
            factories::comment_with(|c| {
                c.id = id.to_string();
                c.database_id = database_id;
                c.author = Some(factories::author(author));
                c.created_at = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
                c.body = body.to_string();
            })
        }

        #[tokio::test]
        async fn test_fetches_and_displays_issue_with_comments() {
            let issue = mock_issue(123, "Test Issue", "Test body content");
            let comments = vec![
                mock_comment("IC_1", 1001, "commenter1", "First comment"),
                mock_comment("IC_2", 1002, "commenter2", "Second comment"),
            ];
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", issue)
                .with_comments("owner", "repo", 123, comments);

            let args = ViewArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
            };

            let output = run_with_client_and_output_with(&args, &client, fixed_time)
                .await
                .unwrap();

            assert_eq!(
                output,
                "\
Test Issue #123

Open • testuser opened 2 hours ago • 2 comments
Labels: bug

  Test body content

──────────────────────────────────────────────────────

commenter1 • 2 hours ago

  First comment

──────────────────────────────────────────────────────

commenter2 • 2 hours ago

  Second comment
"
            );
        }

        #[tokio::test]
        async fn test_displays_issue_without_comments() {
            let issue = mock_issue(42, "No Comments Issue", "Body without comments");
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", issue)
                .with_comments("owner", "repo", 42, vec![]);

            let args = ViewArgs {
                issue: IssueArgs {
                    issue_number: 42,
                    repo: Some("owner/repo".to_string()),
                },
            };

            let output = run_with_client_and_output_with(&args, &client, fixed_time)
                .await
                .unwrap();

            assert_eq!(
                output,
                "\
No Comments Issue #42

Open • testuser opened 2 hours ago • 0 comments
Labels: bug

  Body without comments
"
            );
        }

        #[tokio::test]
        async fn test_fails_when_issue_not_found() {
            let client = MockGitHubClient::new(); // No issues configured

            let args = ViewArgs {
                issue: IssueArgs {
                    issue_number: 999,
                    repo: Some("owner/repo".to_string()),
                },
            };

            let result = run_with_client_and_output(&args, &client).await;
            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_fails_with_invalid_repo_format() {
            let client = MockGitHubClient::new();

            let args = ViewArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("invalid-repo-format".to_string()),
                },
            };

            let result = run_with_client_and_output(&args, &client).await;
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "Invalid input: Invalid repository format: invalid-repo-format. Expected owner/repo"
            );
        }

        #[tokio::test]
        async fn test_displays_issue_without_body() {
            let issue = factories::issue_with(|i| {
                i.number = 10;
                i.title = "Empty Body Issue".to_string();
                i.body = None;
                i.labels = factories::labels(&["bug"]);
            });
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", issue)
                .with_comments("owner", "repo", 10, vec![]);

            let args = ViewArgs {
                issue: IssueArgs {
                    issue_number: 10,
                    repo: Some("owner/repo".to_string()),
                },
            };

            let output = run_with_client_and_output_with(&args, &client, fixed_time)
                .await
                .unwrap();

            assert_eq!(
                output,
                "\
Empty Body Issue #10

Open • testuser opened 2 hours ago • 0 comments
Labels: bug

  No description provided.
"
            );
        }

        #[tokio::test]
        async fn test_displays_issue_with_multiple_labels() {
            let issue = factories::issue_with(|i| {
                i.number = 15;
                i.title = "Multi Label Issue".to_string();
                i.body = Some("Body".to_string());
                i.labels = factories::labels(&["bug", "enhancement", "help wanted"]);
            });
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", issue)
                .with_comments("owner", "repo", 15, vec![]);

            let args = ViewArgs {
                issue: IssueArgs {
                    issue_number: 15,
                    repo: Some("owner/repo".to_string()),
                },
            };

            let output = run_with_client_and_output_with(&args, &client, fixed_time)
                .await
                .unwrap();

            assert_eq!(
                output,
                "\
Multi Label Issue #15

Open • testuser opened 2 hours ago • 0 comments
Labels: bug, enhancement, help wanted

  Body
"
            );
        }
    }
}
