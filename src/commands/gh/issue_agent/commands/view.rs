use chrono::{DateTime, Utc};
use clap::Args;

use super::common::{get_repo_from_arg_or_git, parse_repo};
use crate::commands::gh::issue_agent::format::{format_relative_time, indent_text};
use crate::commands::gh::issue_agent::models::{Comment, Issue, TimelineItem};
use crate::infra::github::OctocrabClient;

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct ViewArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,
}

pub async fn run(args: &ViewArgs) -> anyhow::Result<()> {
    let client = OctocrabClient::get()?;
    let output = run_with_client_and_output(args, client).await?;
    print!("{}", output);
    Ok(())
}

/// Internal implementation that returns the formatted output for testability.
pub(super) async fn run_with_client_and_output(
    args: &ViewArgs,
    client: &OctocrabClient,
) -> anyhow::Result<String> {
    run_with_client_and_output_with(args, client, format_relative_time).await
}

/// Internal implementation that accepts a custom time formatter for testability.
async fn run_with_client_and_output_with<F>(
    args: &ViewArgs,
    client: &OctocrabClient,
    time_formatter: F,
) -> anyhow::Result<String>
where
    F: Fn(&str) -> String,
{
    let repo_str = get_repo_from_arg_or_git(&args.issue.repo)?;
    let (owner, repo) = parse_repo(&repo_str)?;
    let issue_number = args.issue.issue_number;

    let (issue, comments, timeline_events) = tokio::try_join!(
        client.get_issue(&owner, &repo, issue_number),
        client.get_comments(&owner, &repo, issue_number),
        client.get_timeline_events(&owner, &repo, issue_number)
    )?;

    Ok(format_issue_view_with(
        &issue,
        issue_number,
        &comments,
        &timeline_events,
        time_formatter,
    ))
}

/// Entry that can be displayed in the timeline (comment or event).
enum DisplayEntry<'a> {
    Comment(&'a Comment),
    Event(&'a TimelineItem),
}

impl DisplayEntry<'_> {
    fn created_at(&self) -> DateTime<Utc> {
        match self {
            DisplayEntry::Comment(c) => c.created_at,
            DisplayEntry::Event(e) => e.created_at().unwrap_or_else(Utc::now),
        }
    }
}

/// Testable version that accepts a custom time formatter.
fn format_issue_view_with<F>(
    issue: &Issue,
    issue_number: u64,
    comments: &[Comment],
    timeline_events: &[TimelineItem],
    time_formatter: F,
) -> String
where
    F: Fn(&str) -> String,
{
    let mut output = format_issue_with(issue, issue_number, comments.len(), &time_formatter);

    // Merge comments and events chronologically
    let mut entries: Vec<DisplayEntry> = Vec::new();
    entries.extend(comments.iter().map(DisplayEntry::Comment));
    entries.extend(timeline_events.iter().map(DisplayEntry::Event));
    entries.sort_by_key(|e| e.created_at());

    output.push_str(&format_timeline_entries(&entries, &time_formatter));
    output
}

/// Format timeline entries (comments and events) in chronological order.
fn format_timeline_entries<F>(entries: &[DisplayEntry], time_formatter: &F) -> String
where
    F: Fn(&str) -> String,
{
    let mut output = String::new();

    for entry in entries {
        match entry {
            DisplayEntry::Comment(comment) => {
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
            DisplayEntry::Event(event) => {
                if let Some(formatted) = format_timeline_event(event, time_formatter) {
                    output.push('\n');
                    output.push_str(&format!("{}\n", formatted));
                }
            }
        }
    }

    output
}

/// Format a single timeline event for display.
/// Returns None for events that should not be displayed.
fn format_timeline_event<F>(event: &TimelineItem, time_formatter: &F) -> Option<String>
where
    F: Fn(&str) -> String,
{
    let (prefix, actor, description, created_at) = match event {
        TimelineItem::LabeledEvent(e) => (
            "+",
            e.actor.as_ref().map(|a| a.login.as_str()),
            format!("added label '{}'", e.label.name),
            e.created_at,
        ),
        TimelineItem::UnlabeledEvent(e) => (
            "-",
            e.actor.as_ref().map(|a| a.login.as_str()),
            format!("removed label '{}'", e.label.name),
            e.created_at,
        ),
        TimelineItem::AssignedEvent(e) => {
            let assignee = e
                .assignee
                .as_ref()
                .map(|a| a.login.as_str())
                .unwrap_or("someone");
            (
                "+",
                e.actor.as_ref().map(|a| a.login.as_str()),
                format!("assigned {}", assignee),
                e.created_at,
            )
        }
        TimelineItem::UnassignedEvent(e) => {
            let assignee = e
                .assignee
                .as_ref()
                .map(|a| a.login.as_str())
                .unwrap_or("someone");
            (
                "-",
                e.actor.as_ref().map(|a| a.login.as_str()),
                format!("unassigned {}", assignee),
                e.created_at,
            )
        }
        TimelineItem::ClosedEvent(e) => (
            "x",
            e.actor.as_ref().map(|a| a.login.as_str()),
            "closed this".to_string(),
            e.created_at,
        ),
        TimelineItem::ReopenedEvent(e) => (
            "o",
            e.actor.as_ref().map(|a| a.login.as_str()),
            "reopened this".to_string(),
            e.created_at,
        ),
        TimelineItem::CrossReferencedEvent(e) => {
            let source = &e.source;
            let source_type = if source.is_pull_request() {
                "PR"
            } else {
                "issue"
            };
            let will_close = if e.will_close_target {
                " (will close)"
            } else {
                ""
            };
            let description = format!(
                "referenced this from {} {}#{} \"{}\"{}",
                source_type,
                source.repository(),
                source.number(),
                source.title(),
                will_close
            );
            (
                "->",
                e.actor.as_ref().map(|a| a.login.as_str()),
                description,
                e.created_at,
            )
        }
        TimelineItem::Unknown => return None,
    };

    let actor_str = actor.unwrap_or("someone");
    let relative_time = time_formatter(&created_at.to_rfc3339());

    Some(format!(
        "  {} {} {} • {}",
        prefix, actor_str, description, relative_time
    ))
}

fn format_state(state: &str) -> &str {
    match state {
        "OPEN" => "Open",
        "CLOSED" => "Closed",
        _ => state,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::gh::issue_agent::testing::factories;
    use indoc::{formatdoc, indoc};
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
            indoc! {"
                Test Issue Title #123

                Open • testuser opened 2 hours ago • 0 comments

                  This is the issue body.
            "}
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
            indoc! {"
                Bug Report #42

                Open • reporter opened 2 hours ago • 5 comments
                Labels: bug, urgent
                Assignees: dev1, dev2

                  Found a bug.
            "}
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
            formatdoc! {"
                Test Issue #1

                {expected}
                  Test body
            "}
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
            formatdoc! {"
                Test Issue #1

                Open • testuser opened 2 hours ago • 0 comments

                {expected_body}"}
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
            formatdoc! {"
                Test Issue #1

                Open • {expected_author} opened 2 hours ago • 0 comments

                  Test body
            "}
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
            formatdoc! {"
                Test Issue #1

                {expected_state} • testuser opened 2 hours ago • 0 comments

                  Test body
            "}
        );
    }

    #[test]
    fn test_format_timeline_entries_empty() {
        let entries: Vec<DisplayEntry> = vec![];
        let output = format_timeline_entries(&entries, &fixed_time);
        assert_eq!(output, "");
    }

    #[test]
    fn test_format_timeline_entries_single_comment() {
        let comment = factories::comment_with(|c| {
            c.author = Some(factories::author("commenter"));
            c.body = "This is a comment.".to_string();
        });
        let entries = vec![DisplayEntry::Comment(&comment)];

        let output = format_timeline_entries(&entries, &fixed_time);

        assert_eq!(
            output,
            indoc! {"

                ──────────────────────────────────────────────────────

                commenter • 2 hours ago

                  This is a comment.
            "}
        );
    }

    #[test]
    fn test_format_timeline_entries_multiple_comments() {
        let comment1 = factories::comment_with(|c| {
            c.author = Some(factories::author("user1"));
            c.body = "First comment.".to_string();
        });
        let comment2 = factories::comment_with(|c| {
            c.author = Some(factories::author("user2"));
            c.body = "Second comment.".to_string();
        });
        let entries = vec![
            DisplayEntry::Comment(&comment1),
            DisplayEntry::Comment(&comment2),
        ];

        let output = format_timeline_entries(&entries, &fixed_time);

        assert_eq!(
            output,
            indoc! {"

                ──────────────────────────────────────────────────────

                user1 • 2 hours ago

                  First comment.

                ──────────────────────────────────────────────────────

                user2 • 2 hours ago

                  Second comment.
            "}
        );
    }

    #[rstest]
    #[case::with_author(Some("commenter"), "commenter")]
    #[case::no_author(None, "unknown")]
    fn test_format_timeline_entries_comment_author(
        #[case] author: Option<&str>,
        #[case] expected_author: &str,
    ) {
        let comment = factories::comment_with(|c| {
            c.author = author.map(factories::author);
            c.body = "Comment body.".to_string();
        });
        let entries = vec![DisplayEntry::Comment(&comment)];
        let output = format_timeline_entries(&entries, &fixed_time);
        assert_eq!(
            output,
            formatdoc! {"

                ──────────────────────────────────────────────────────

                {expected_author} • 2 hours ago

                  Comment body.
            "}
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

        let output = format_issue_view_with(&issue, 99, &comments, &[], fixed_time);

        assert_eq!(
            output,
            indoc! {"
                Feature Request #99

                Open • author opened 2 hours ago • 1 comment
                Labels: enhancement

                  Add new feature.

                ──────────────────────────────────────────────────────

                reviewer • 2 hours ago

                  Looks good!
            "}
        );
    }

    mod timeline_event_tests {
        use super::*;

        #[test]
        fn test_format_labeled_event() {
            let event = factories::labeled_event("admin", "bug");
            let output = format_timeline_event(&event, &fixed_time).unwrap();
            assert_eq!(output, "  + admin added label 'bug' • 2 hours ago");
        }

        #[test]
        fn test_format_unlabeled_event() {
            let event = factories::unlabeled_event("admin", "bug");
            let output = format_timeline_event(&event, &fixed_time).unwrap();
            assert_eq!(output, "  - admin removed label 'bug' • 2 hours ago");
        }

        #[test]
        fn test_format_assigned_event() {
            let event = factories::assigned_event("admin", "dev1");
            let output = format_timeline_event(&event, &fixed_time).unwrap();
            assert_eq!(output, "  + admin assigned dev1 • 2 hours ago");
        }

        #[test]
        fn test_format_unassigned_event() {
            let event = factories::unassigned_event("admin", "dev1");
            let output = format_timeline_event(&event, &fixed_time).unwrap();
            assert_eq!(output, "  - admin unassigned dev1 • 2 hours ago");
        }

        #[test]
        fn test_format_closed_event() {
            let event = factories::closed_event("admin");
            let output = format_timeline_event(&event, &fixed_time).unwrap();
            assert_eq!(output, "  x admin closed this • 2 hours ago");
        }

        #[test]
        fn test_format_reopened_event() {
            let event = factories::reopened_event("admin");
            let output = format_timeline_event(&event, &fixed_time).unwrap();
            assert_eq!(output, "  o admin reopened this • 2 hours ago");
        }

        #[test]
        fn test_format_cross_referenced_pr() {
            let event =
                factories::cross_referenced_pr("dev1", 123, "Add feature", "owner", "repo", false);
            let output = format_timeline_event(&event, &fixed_time).unwrap();
            assert_eq!(
                output,
                "  -> dev1 referenced this from PR owner/repo#123 \"Add feature\" • 2 hours ago"
            );
        }

        #[test]
        fn test_format_cross_referenced_pr_will_close() {
            let event =
                factories::cross_referenced_pr("dev1", 123, "Fix bug", "owner", "repo", true);
            let output = format_timeline_event(&event, &fixed_time).unwrap();
            assert_eq!(
                output,
                "  -> dev1 referenced this from PR owner/repo#123 \"Fix bug\" (will close) • 2 hours ago"
            );
        }

        #[test]
        fn test_format_cross_referenced_issue() {
            let event = factories::cross_referenced_issue(
                "user1",
                456,
                "Related issue",
                "other",
                "project",
            );
            let output = format_timeline_event(&event, &fixed_time).unwrap();
            assert_eq!(
                output,
                "  -> user1 referenced this from issue other/project#456 \"Related issue\" • 2 hours ago"
            );
        }

        #[test]
        fn test_format_unknown_event_returns_none() {
            let event = TimelineItem::Unknown;
            let output = format_timeline_event(&event, &fixed_time);
            assert!(output.is_none());
        }
    }

    mod run_with_client_tests {
        use super::*;
        use crate::commands::gh::issue_agent::commands::IssueArgs;
        use crate::commands::gh::issue_agent::commands::test_helpers::{
            GitHubMockServer, RemoteComment, RemoteTimelineEvent,
        };
        use indoc::indoc;

        #[tokio::test]
        async fn test_fetches_and_displays_issue_with_comments() {
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(123)
                .title("Test Issue")
                .body("Test body content")
                .get()
                .await;
            ctx.graphql_comments(&[
                RemoteComment {
                    id: "IC_1",
                    database_id: 1001,
                    author: "commenter1",
                    body: "First comment",
                },
                RemoteComment {
                    id: "IC_2",
                    database_id: 1002,
                    author: "commenter2",
                    body: "Second comment",
                },
            ])
            .await;
            ctx.graphql_timeline_events(&[]).await;

            let client = mock.client();
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
                indoc! {"
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
                "}
            );
        }

        #[tokio::test]
        async fn test_displays_issue_without_comments() {
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(42)
                .title("No Comments Issue")
                .body("Body without comments")
                .get()
                .await;
            ctx.graphql_comments(&[]).await;
            ctx.graphql_timeline_events(&[]).await;

            let client = mock.client();
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
                indoc! {"
                    No Comments Issue #42

                    Open • testuser opened 2 hours ago • 0 comments
                    Labels: bug

                      Body without comments
                "}
            );
        }

        #[tokio::test]
        async fn test_fails_when_issue_not_found() {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo").issue(999).get_not_found().await;

            let client = mock.client();
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
            let mock = GitHubMockServer::start().await;
            let client = mock.client();

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
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(10).title("Empty Body Issue").body("").get().await;
            ctx.graphql_comments(&[]).await;
            ctx.graphql_timeline_events(&[]).await;

            let client = mock.client();
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
                indoc! {"
                    Empty Body Issue #10

                    Open • testuser opened 2 hours ago • 0 comments
                    Labels: bug

                      No description provided.
                "}
            );
        }

        #[tokio::test]
        async fn test_displays_issue_with_multiple_labels() {
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(15)
                .title("Multi Label Issue")
                .body("Body")
                .labels(vec!["bug", "enhancement", "help wanted"])
                .get()
                .await;
            ctx.graphql_comments(&[]).await;
            ctx.graphql_timeline_events(&[]).await;

            let client = mock.client();
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
                indoc! {"
                    Multi Label Issue #15

                    Open • testuser opened 2 hours ago • 0 comments
                    Labels: bug, enhancement, help wanted

                      Body
                "}
            );
        }

        #[tokio::test]
        async fn test_displays_issue_with_timeline_events() {
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(123)
                .title("Issue with Events")
                .body("Test body")
                .get()
                .await;
            ctx.graphql_comments(&[RemoteComment {
                id: "IC_1",
                database_id: 1001,
                author: "commenter",
                body: "A comment",
            }])
            .await;
            ctx.graphql_timeline_events(&[
                RemoteTimelineEvent::Labeled {
                    actor: "admin",
                    created_at: "2024-01-01T10:00:00Z",
                    label: "enhancement",
                },
                RemoteTimelineEvent::CrossReferenced {
                    actor: "dev1",
                    created_at: "2024-01-01T11:00:00Z",
                    source_type: "PullRequest",
                    source_number: 456,
                    source_title: "Add feature",
                    source_repo_owner: "owner",
                    source_repo_name: "repo",
                    will_close_target: true,
                },
            ])
            .await;

            let client = mock.client();
            let args = ViewArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
            };

            let output = run_with_client_and_output_with(&args, &client, fixed_time)
                .await
                .unwrap();

            // Events are sorted by created_at, so labeled event comes first
            assert_eq!(
                output,
                indoc! {"
                    Issue with Events #123

                    Open • testuser opened 2 hours ago • 1 comment
                    Labels: bug

                      Test body

                      + admin added label 'enhancement' • 2 hours ago

                      -> dev1 referenced this from PR owner/repo#456 \"Add feature\" (will close) • 2 hours ago

                    ──────────────────────────────────────────────────────

                    commenter • 2 hours ago

                      A comment
                "}
            );
        }

        #[tokio::test]
        async fn test_displays_issue_with_all_event_types() {
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(123)
                .title("All Events Issue")
                .body("Body")
                .get()
                .await;
            ctx.graphql_comments(&[]).await;
            ctx.graphql_timeline_events(&[
                RemoteTimelineEvent::Labeled {
                    actor: "admin",
                    created_at: "2024-01-01T10:00:00Z",
                    label: "bug",
                },
                RemoteTimelineEvent::Assigned {
                    actor: "admin",
                    created_at: "2024-01-01T10:01:00Z",
                    assignee: "dev1",
                },
                RemoteTimelineEvent::Closed {
                    actor: "admin",
                    created_at: "2024-01-01T10:02:00Z",
                },
                RemoteTimelineEvent::Reopened {
                    actor: "admin",
                    created_at: "2024-01-01T10:03:00Z",
                },
                RemoteTimelineEvent::Unlabeled {
                    actor: "admin",
                    created_at: "2024-01-01T10:04:00Z",
                    label: "bug",
                },
                RemoteTimelineEvent::Unassigned {
                    actor: "admin",
                    created_at: "2024-01-01T10:05:00Z",
                    assignee: "dev1",
                },
            ])
            .await;

            let client = mock.client();
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
                indoc! {"
                    All Events Issue #123

                    Open • testuser opened 2 hours ago • 0 comments
                    Labels: bug

                      Body

                      + admin added label 'bug' • 2 hours ago

                      + admin assigned dev1 • 2 hours ago

                      x admin closed this • 2 hours ago

                      o admin reopened this • 2 hours ago

                      - admin removed label 'bug' • 2 hours ago

                      - admin unassigned dev1 • 2 hours ago
                "}
            );
        }
    }
}
