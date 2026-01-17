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

    print_issue(&issue, issue_number, comments.len());
    print_comments(&comments);

    Ok(())
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

fn print_issue(issue: &Issue, issue_number: u64, comment_count: usize) {
    // Title line
    println!("{} #{}", issue.title, issue_number);
    println!();

    // Status line
    let author = issue
        .author
        .as_ref()
        .map(|a| a.login.as_str())
        .unwrap_or("unknown");
    let relative_time = format_relative_time(&issue.created_at.to_rfc3339());
    let state_display = format_state(&issue.state);
    println!(
        "{} • {} opened {} • {} comment{}",
        state_display,
        author,
        relative_time,
        comment_count,
        if comment_count == 1 { "" } else { "s" }
    );

    // Labels
    if !issue.labels.is_empty() {
        let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
        println!("Labels: {}", labels.join(", "));
    }

    // Assignees
    if !issue.assignees.is_empty() {
        let assignees: Vec<&str> = issue.assignees.iter().map(|a| a.login.as_str()).collect();
        println!("Assignees: {}", assignees.join(", "));
    }

    // Body
    println!();
    if let Some(body) = issue.body.as_deref().filter(|b| !b.is_empty()) {
        println!("{}", indent_text(body, "  "));
    } else {
        println!("  No description provided.");
    }
}

fn print_comments(comments: &[Comment]) {
    for comment in comments {
        println!();
        println!("──────────────────────────────────────────────────────");
        println!();

        let author = comment
            .author
            .as_ref()
            .map(|a| a.login.as_str())
            .unwrap_or("unknown");
        let relative_time = format_relative_time(&comment.created_at.to_rfc3339());
        println!("{} • {}", author, relative_time);

        println!();
        println!("{}", indent_text(&comment.body, "  "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

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
}
