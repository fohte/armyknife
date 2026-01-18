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
    let (owner, repo) = get_repo_owner_and_name(args.issue.repo.as_deref())?;
    let issue_number = args.issue.issue_number;

    let (issue, comments) = tokio::try_join!(
        client.get_issue(&owner, &repo, issue_number),
        client.get_comments(&owner, &repo, issue_number)
    )?;

    Ok(format_issue_view(&issue, issue_number, &comments))
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

    #[rstest]
    #[case::zero(0, "0 comments")]
    #[case::one(1, "1 comment\n")]
    #[case::two(2, "2 comments")]
    #[case::many(10, "10 comments")]
    fn test_format_issue_comment_count(#[case] count: usize, #[case] expected: &str) {
        let issue = create_test_issue("Title", "OPEN", Some("body"), Some("user"), vec![], vec![]);
        let output = format_issue(&issue, 1, count);
        assert!(output.contains(expected));
    }

    #[rstest]
    #[case::none(None, "No description provided.")]
    #[case::empty(Some(""), "No description provided.")]
    #[case::with_body(Some("Issue body"), "  Issue body")]
    fn test_format_issue_body(#[case] body: Option<&str>, #[case] expected: &str) {
        let issue = create_test_issue("Title", "OPEN", body, Some("user"), vec![], vec![]);
        let output = format_issue(&issue, 1, 0);
        assert!(output.contains(expected));
    }

    #[rstest]
    #[case::with_author(Some("testuser"), "testuser opened")]
    #[case::no_author(None, "unknown opened")]
    fn test_format_issue_author(#[case] author: Option<&str>, #[case] expected: &str) {
        let issue = create_test_issue("Title", "OPEN", Some("body"), author, vec![], vec![]);
        let output = format_issue(&issue, 1, 0);
        assert!(output.contains(expected));
    }

    #[rstest]
    #[case::open("OPEN", "Open")]
    #[case::closed("CLOSED", "Closed")]
    fn test_format_issue_state_display(#[case] state: &str, #[case] expected: &str) {
        let issue = create_test_issue("Title", state, Some("body"), Some("user"), vec![], vec![]);
        let output = format_issue(&issue, 1, 0);
        assert!(output.contains(expected));
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

    #[rstest]
    #[case::with_author(Some("commenter"), "commenter •")]
    #[case::no_author(None, "unknown •")]
    fn test_format_comments_author(#[case] author: Option<&str>, #[case] expected: &str) {
        let comments = vec![create_test_comment(author, "Comment body.")];
        let output = format_comments(&comments);
        assert!(output.contains(expected));
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

    mod run_with_client_tests {
        use super::*;
        use crate::gh::issue_agent::commands::IssueArgs;
        use crate::github::MockGitHubClient;
        use chrono::{TimeZone, Utc};

        fn create_mock_issue(number: i64, title: &str, body: &str) -> Issue {
            Issue {
                number,
                title: title.to_string(),
                body: Some(body.to_string()),
                state: "OPEN".to_string(),
                labels: vec![Label {
                    name: "bug".to_string(),
                }],
                assignees: vec![],
                milestone: None,
                author: Some(Author {
                    login: "testuser".to_string(),
                }),
                created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                updated_at: Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
            }
        }

        fn create_mock_comment(id: &str, database_id: i64, author: &str, body: &str) -> Comment {
            Comment {
                id: id.to_string(),
                database_id,
                author: Some(Author {
                    login: author.to_string(),
                }),
                created_at: Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
                body: body.to_string(),
            }
        }

        #[tokio::test]
        async fn test_fetches_and_displays_issue_with_comments() {
            let issue = create_mock_issue(123, "Test Issue", "Test body content");
            let comments = vec![
                create_mock_comment("IC_1", 1001, "commenter1", "First comment"),
                create_mock_comment("IC_2", 1002, "commenter2", "Second comment"),
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

            let output = run_with_client_and_output(&args, &client).await.unwrap();

            // Verify issue content
            assert!(output.contains("Test Issue #123"));
            assert!(output.contains("Test body content"));
            assert!(output.contains("Labels: bug"));

            // Verify comments
            assert!(output.contains("commenter1"));
            assert!(output.contains("First comment"));
            assert!(output.contains("commenter2"));
            assert!(output.contains("Second comment"));
        }

        #[tokio::test]
        async fn test_displays_issue_without_comments() {
            let issue = create_mock_issue(42, "No Comments Issue", "Body without comments");
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", issue)
                .with_comments("owner", "repo", 42, vec![]);

            let args = ViewArgs {
                issue: IssueArgs {
                    issue_number: 42,
                    repo: Some("owner/repo".to_string()),
                },
            };

            let output = run_with_client_and_output(&args, &client).await.unwrap();

            assert!(output.contains("No Comments Issue #42"));
            assert!(output.contains("0 comments"));
            // No comment separators
            assert!(!output.contains("──────────────────────────────────────────────────────"));
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
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Invalid repository format")
            );
        }

        #[tokio::test]
        async fn test_displays_issue_without_body() {
            let mut issue = create_mock_issue(10, "Empty Body Issue", "");
            issue.body = None;
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", issue)
                .with_comments("owner", "repo", 10, vec![]);

            let args = ViewArgs {
                issue: IssueArgs {
                    issue_number: 10,
                    repo: Some("owner/repo".to_string()),
                },
            };

            let output = run_with_client_and_output(&args, &client).await.unwrap();

            assert!(output.contains("Empty Body Issue #10"));
            assert!(output.contains("No description provided."));
        }

        #[tokio::test]
        async fn test_displays_issue_with_multiple_labels() {
            let mut issue = create_mock_issue(15, "Multi Label Issue", "Body");
            issue.labels = vec![
                Label {
                    name: "bug".to_string(),
                },
                Label {
                    name: "enhancement".to_string(),
                },
                Label {
                    name: "help wanted".to_string(),
                },
            ];
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", issue)
                .with_comments("owner", "repo", 15, vec![]);

            let args = ViewArgs {
                issue: IssueArgs {
                    issue_number: 15,
                    repo: Some("owner/repo".to_string()),
                },
            };

            let output = run_with_client_and_output(&args, &client).await.unwrap();

            assert!(output.contains("Labels: bug, enhancement, help wanted"));
        }
    }
}
