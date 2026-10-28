use std::collections::{HashMap, HashSet};

use clap::Args;

use super::api::fetch_pr_data;
use super::changeset::ReplyChangeSet;
use super::error::PrReviewError;
use super::markdown::serializer::{PreservedThreadActions, SerializeOptions, ThreadsFrontmatter};
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

    /// Overwrite local data even if it cannot be preserved
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

pub async fn run_pull(args: &ReplyPullArgs) -> anyhow::Result<()> {
    let (owner, repo) = git::get_repo_owner_and_name(args.repo.as_deref())?;

    let storage = ThreadStorage::new(&owner, &repo, args.pr_number);

    let existing_content = if storage.exists() {
        match storage.read_threads() {
            Ok(content) => Some(content),
            Err(_) if args.force => None,
            Err(error) => {
                return Err(PrReviewError::PullWouldLoseLocalData {
                    details: format!(
                        "threads.md could not be read, so draft replies and resolve ticks cannot be preserved: {error}"
                    ),
                }
                .into());
            }
        }
    } else {
        None
    };

    // Fetch remote data
    let pr_data = fetch_pr_data(&owner, &repo, args.pr_number, args.include_resolved).await?;

    let remote_thread_ids = pr_data
        .threads
        .iter()
        .filter_map(|thread| thread.id.clone())
        .collect();
    let existing_actions = prepare_existing_thread_actions(
        existing_content.as_deref(),
        &remote_thread_ids,
        args.force,
    )?;

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
    let outcome = MarkdownSerializer::serialize_with_actions(
        &pr_data,
        &frontmatter,
        &existing_actions,
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

fn prepare_existing_thread_actions(
    existing_content: Option<&str>,
    fresh_thread_ids: &HashSet<String>,
    force: bool,
) -> Result<HashMap<String, PreservedThreadActions>, PrReviewError> {
    let Some(content) = existing_content else {
        return Ok(HashMap::new());
    };

    let parsed = match MarkdownParser::parse(content) {
        Ok(parsed) => parsed,
        Err(_) if force => return Ok(HashMap::new()),
        Err(error) => {
            return Err(PrReviewError::PullWouldLoseLocalData {
                details: format!(
                    "threads.md could not be parsed, so draft replies and resolve ticks cannot be preserved: {error}"
                ),
            });
        }
    };

    let mut preserved_actions = HashMap::new();
    let mut losses = Vec::new();

    for thread in parsed.threads {
        let actions = PreservedThreadActions {
            draft_reply: thread.draft_reply,
            resolve: thread.resolve,
        };
        let mut lost_actions = Vec::new();
        if actions.draft_reply.is_some() {
            lost_actions.push("draft reply");
        }
        if actions.resolve {
            lost_actions.push("resolve tick");
        }
        if lost_actions.is_empty() {
            continue;
        }

        if fresh_thread_ids.contains(&thread.thread_id) {
            preserved_actions.insert(thread.thread_id, actions);
        } else {
            let location = thread
                .line
                .map(|line| format!("{}:{line}", thread.path))
                .unwrap_or(thread.path);
            losses.push(format!(
                "thread {} at {location}: {}",
                thread.thread_id,
                lost_actions.join(" and ")
            ));
        }
    }

    if !force && !losses.is_empty() {
        return Err(PrReviewError::PullWouldLoseLocalData {
            details: losses.join("; "),
        });
    }

    Ok(preserved_actions)
}

pub async fn run_push(args: &ReplyPushArgs) -> anyhow::Result<()> {
    let (owner, repo) = git::get_repo_owner_and_name(args.repo.as_deref())?;

    let storage = ThreadStorage::new(&owner, &repo, args.pr_number);

    // Read and parse local data
    if !storage.exists() {
        return Err(PrReviewError::NoPulledData.into());
    }

    // Verify approval before pushing
    let document = Document::<ThreadsFrontmatter>::from_path(storage.threads_path())?;
    document.verify_approval()?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::{fixture, rstest};

    const THREAD_WITH_DRAFT_AND_RESOLVE: &str = indoc! {r#"
        ---
        pr: 42
        repo: "fohte/armyknife"
        pulled_at: "2024-01-15T10:00:00Z"
        submit: true
        ---

        <!-- thread: RT_example path: src/example.rs:9 author: @reviewer -->
        - [x] resolve
        <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
        Remote comment
        <!-- /comment -->
        Draft reply
    "#};

    #[fixture]
    fn local_thread_with_draft_and_resolve() -> String {
        THREAD_WITH_DRAFT_AND_RESOLVE.to_string()
    }

    #[rstest]
    fn test_prepare_existing_thread_actions_blocks_unparseable_file() {
        let expected = "Re-pull would lose local data: threads.md could not be parsed, so draft replies and resolve ticks cannot be preserved: Invalid frontmatter: missing opening ---. Use --force to overwrite";

        let result =
            prepare_existing_thread_actions(Some("not threads markdown"), &HashSet::new(), false);

        assert_eq!(result.unwrap_err().to_string(), expected);
    }

    #[rstest]
    fn test_prepare_existing_thread_actions_blocks_missing_local_actions(
        local_thread_with_draft_and_resolve: String,
    ) {
        let expected = "Re-pull would lose local data: thread RT_example at src/example.rs:9: draft reply and resolve tick. Use --force to overwrite";

        let result = prepare_existing_thread_actions(
            Some(&local_thread_with_draft_and_resolve),
            &HashSet::new(),
            false,
        );

        assert_eq!(result.unwrap_err().to_string(), expected);
    }

    #[rstest]
    fn test_prepare_existing_thread_actions_preserves_local_actions(
        mut local_thread_with_draft_and_resolve: String,
    ) {
        local_thread_with_draft_and_resolve.push(' ');
        let fresh_thread_ids = HashSet::from(["RT_example".to_string()]);
        let expected = HashMap::from([(
            "RT_example".to_string(),
            PreservedThreadActions {
                draft_reply: Some("Draft reply".to_string()),
                resolve: true,
            },
        )]);

        assert_eq!(
            prepare_existing_thread_actions(
                Some(&local_thread_with_draft_and_resolve),
                &fresh_thread_ids,
                false,
            )
            .unwrap(),
            expected
        );
    }

    #[rstest]
    fn test_prepare_existing_thread_actions_allows_missing_thread_without_local_actions() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
            ---

            <!-- thread: RT_example path: src/example.rs:9 author: @reviewer -->
            - [ ] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Remote comment
            <!-- /comment -->
        "#};

        assert_eq!(
            prepare_existing_thread_actions(Some(content), &HashSet::new(), false).unwrap(),
            HashMap::new()
        );
    }

    #[rstest]
    #[case::unparseable_file("not threads markdown")]
    #[case::missing_thread_with_actions(THREAD_WITH_DRAFT_AND_RESOLVE)]
    fn test_force_allows_overwriting_unpreservable_data(#[case] content: &str) {
        assert_eq!(
            prepare_existing_thread_actions(Some(content), &HashSet::new(), true).unwrap(),
            HashMap::new()
        );
    }
}
