use std::collections::HashMap;

use clap::Args;

use super::api::fetch_pr_data;
use super::changeset::ReplyChangeSet;
use super::error::PrReviewError;
use super::markdown::serializer::ThreadsFrontmatter;
use super::markdown::{MarkdownParser, MarkdownSerializer};
use super::storage::ThreadStorage;
use crate::infra::git;
use crate::infra::github::GitHubClient;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReplyPullArgs {
    /// PR number
    pub pr_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,

    /// Include resolved threads
    #[arg(long = "include-resolved")]
    pub include_resolved: bool,

    /// Overwrite local changes without confirmation
    #[arg(long = "force")]
    pub force: bool,
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReplyPushArgs {
    /// PR number
    pub pr_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,

    /// Preview changes without applying
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    /// Force push even with conflicts
    #[arg(long = "force")]
    pub force: bool,
}

pub async fn run_pull(args: &ReplyPullArgs) -> anyhow::Result<()> {
    let (owner, repo) = git::get_repo_owner_and_name(args.repo.as_deref())?;

    let storage = ThreadStorage::new(&owner, &repo, args.pr_number);

    // Check for local changes
    if storage.exists() && !args.force {
        let has_changes = storage.has_local_changes()?;
        if has_changes {
            return Err(PrReviewError::LocalChangesExist.into());
        }
    }

    // Fetch remote data
    let pr_data = fetch_pr_data(&owner, &repo, args.pr_number, args.include_resolved).await?;

    // Preserve existing drafts if re-pulling
    let existing_drafts = if storage.exists() {
        let content = storage.read_threads()?;
        match MarkdownParser::parse(&content) {
            Ok(parsed) => parsed
                .threads
                .into_iter()
                .filter_map(|t| t.draft_reply.map(|d| (t.thread_id, d)))
                .collect(),
            Err(_) => HashMap::new(),
        }
    } else {
        HashMap::new()
    };

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let frontmatter = ThreadsFrontmatter {
        pr: args.pr_number,
        repo: format!("{owner}/{repo}"),
        pulled_at: now,
        submit: false,
    };

    let content =
        MarkdownSerializer::serialize_with_drafts(&pr_data, &frontmatter, &existing_drafts);

    storage.write_threads(&content)?;

    println!("Saved to: {}", storage.threads_path().display());
    println!("  {} thread(s) pulled", pr_data.threads.len());

    Ok(())
}

pub async fn run_push(args: &ReplyPushArgs) -> anyhow::Result<()> {
    let (owner, repo) = git::get_repo_owner_and_name(args.repo.as_deref())?;

    let storage = ThreadStorage::new(&owner, &repo, args.pr_number);

    // Read and parse local data
    if !storage.exists() {
        return Err(PrReviewError::NoPulledData.into());
    }

    let content = storage.read_threads()?;
    let local = MarkdownParser::parse(&content)?;

    // Fetch latest remote state
    let remote = fetch_pr_data(&owner, &repo, args.pr_number, true).await?;

    // Detect changes
    let changeset = ReplyChangeSet::detect(&local, &remote, &local.frontmatter.pulled_at);

    // Check for conflicts
    if changeset.has_conflicts() && !args.force {
        return Err(PrReviewError::ConflictDetected {
            count: changeset.conflicts.len(),
            details: changeset.format_conflicts(),
        }
        .into());
    }

    if !changeset.has_changes() {
        println!("No changes to push.");
        return Ok(());
    }

    // Display changes
    changeset.display();

    if args.dry_run {
        println!("\n(dry-run mode: no changes applied)");
        return Ok(());
    }

    // Apply changes
    let client = GitHubClient::get()?;
    changeset
        .apply(client, &owner, &repo, args.pr_number)
        .await?;

    // Re-fetch and update local file after push
    let include_resolved = local.threads.iter().any(|t| t.resolve);
    let updated_remote = fetch_pr_data(&owner, &repo, args.pr_number, include_resolved).await?;

    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let frontmatter = ThreadsFrontmatter {
        pr: args.pr_number,
        repo: format!("{owner}/{repo}"),
        pulled_at: now,
        submit: false,
    };

    let updated_content = MarkdownSerializer::serialize(&updated_remote, &frontmatter);
    storage.write_threads(&updated_content)?;

    println!("\nPush complete.");

    Ok(())
}
