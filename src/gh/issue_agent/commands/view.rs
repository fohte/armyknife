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
    let issue = client.get_issue(&owner, &repo, issue_number).await?;
    let comments = client.get_comments(&owner, &repo, issue_number).await?;

    print_issue(&issue, issue_number, comments.len());
    print_comments(&comments);

    Ok(())
}

fn get_repo_owner_and_name(
    repo_arg: Option<&str>,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    if let Some(repo) = repo_arg {
        let parts: Vec<&str> = repo.split('/').collect();
        if parts.len() == 2 {
            return Ok((parts[0].to_string(), parts[1].to_string()));
        }
        return Err(format!("Invalid repository format: {repo}. Expected owner/repo").into());
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
    if let Some(body) = &issue.body {
        if !body.is_empty() {
            println!("{}", indent_text(body, "  "));
        } else {
            println!("  No description provided.");
        }
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
