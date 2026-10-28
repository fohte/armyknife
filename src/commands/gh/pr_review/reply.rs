use std::collections::HashMap;

use clap::Args;

use super::api::fetch_pr_data;
use super::changeset::ReplyChangeSet;
use super::error::PrReviewError;
use super::markdown::serializer::{SerializeOptions, ThreadsFrontmatter};
use super::markdown::{MarkdownParser, MarkdownSerializer};
use super::storage::ThreadStorage;
use crate::infra::git;
use crate::infra::github::GitHubClient;
use crate::shared::human_in_the_loop::Document;

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

    /// Expand HTML details blocks in comment bodies (default: collapse them)
    #[arg(short = 'd', long = "open-details")]
    pub open_details: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PushPlan {
    NoChanges,
    DryRun,
    Apply,
}

fn push_plan(changeset: &ReplyChangeSet, dry_run: bool) -> PushPlan {
    if !changeset.has_changes() {
        PushPlan::NoChanges
    } else if dry_run {
        PushPlan::DryRun
    } else {
        PushPlan::Apply
    }
}

fn verify_push_approval(path: &std::path::Path) -> anyhow::Result<()> {
    let document = Document::<ThreadsFrontmatter>::from_path(path.to_path_buf())?;
    document.verify_approval()?;
    Ok(())
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

    let options = SerializeOptions {
        open_details: args.open_details,
    };
    let outcome = MarkdownSerializer::serialize_with_options(
        &pr_data,
        &frontmatter,
        &existing_drafts,
        &options,
    );

    storage.write_threads(&outcome.text)?;

    println!("Saved to: {}", storage.threads_path().display());
    println!("  {} thread(s) pulled", pr_data.threads.len());
    for location in &outcome.parse_failed_threads {
        eprintln!("warning: diff hunk parse failed for {location}");
    }

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

    match push_plan(&changeset, args.dry_run) {
        PushPlan::NoChanges => {
            println!("No changes to push.");
            return Ok(());
        }
        PushPlan::DryRun => {
            changeset.display();
            println!("\n(dry-run mode: no changes applied)");
            return Ok(());
        }
        PushPlan::Apply => {
            changeset.display();
            let client = GitHubClient::get()?;
            verify_push_approval(&storage.threads_path())?;
            changeset
                .apply(client, &owner, &repo, args.pr_number)
                .await?;
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::gh::pr_review::changeset::ReplyAction;
    use indoc::indoc;
    use rstest::rstest;
    use tempfile::TempDir;

    #[rstest]
    #[case::no_changes(false, false, PushPlan::NoChanges)]
    #[case::dry_run(true, true, PushPlan::DryRun)]
    #[case::apply(true, false, PushPlan::Apply)]
    fn test_push_plan(
        #[case] has_changes: bool,
        #[case] dry_run: bool,
        #[case] expected_plan: PushPlan,
    ) {
        let changeset = ReplyChangeSet {
            actions: if has_changes {
                vec![ReplyAction::PostReply {
                    thread_id: "RT_test".to_string(),
                    in_reply_to: 1,
                    body: "Reply".to_string(),
                }]
            } else {
                vec![]
            },
            conflicts: vec![],
        };

        assert_eq!(push_plan(&changeset, dry_run), expected_plan);
    }

    #[rstest]
    #[case::approved(
        indoc! {"
            ---
            pr: 1
            repo: example/repo
            pulled_at: '2024-01-15T10:00:00Z'
            submit: true
            ---
            Draft reply"},
        true
    )]
    #[case::not_approved(
        indoc! {"
            ---
            pr: 1
            repo: example/repo
            pulled_at: '2024-01-15T10:00:00Z'
            submit: false
            ---
            Draft reply"},
        false
    )]
    fn test_verify_push_approval(#[case] content: &str, #[case] expected: bool) {
        crate::shared::testing::init_approval_dir();
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("threads.md");
        std::fs::write(&path, content).unwrap();

        if expected {
            Document::<ThreadsFrontmatter>::from_path(path.clone())
                .unwrap()
                .save_approval()
                .unwrap();
        }

        assert_eq!(verify_push_approval(&path).is_ok(), expected);
    }
}
