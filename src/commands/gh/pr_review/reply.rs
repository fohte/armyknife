use std::collections::HashMap;

use clap::Args;

use super::api::fetch_pr_data;
use super::changeset::ReplyChangeSet;
use super::error::PrReviewError;
use super::markdown::serializer::{PreservedThreadActions, SerializeOptions, ThreadsFrontmatter};
use super::markdown::{MarkdownParser, MarkdownSerializer};
use super::models::PrData;
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

fn verify_push_approval(path: &std::path::Path, content: &str) -> anyhow::Result<()> {
    let document = Document::<ThreadsFrontmatter>::from_content(path.to_path_buf(), content)?;
    document
        .approval_manager()
        .verify_content_matches_file(content.as_bytes())?;
    Ok(())
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
    let mut pr_data = fetch_pr_data(&owner, &repo, args.pr_number, true).await?;
    let remote_thread_states = prepare_pull_data(&mut pr_data, args.include_resolved);
    let existing_actions = prepare_existing_thread_actions(
        existing_content.as_deref(),
        &remote_thread_states,
        args.include_resolved,
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

fn prepare_pull_data(pr_data: &mut PrData, include_resolved: bool) -> HashMap<String, bool> {
    let remote_thread_states = pr_data
        .threads
        .iter()
        .filter_map(|thread| {
            thread
                .id
                .as_ref()
                .map(|id| (id.clone(), thread.is_resolved))
        })
        .collect();

    if !include_resolved {
        pr_data.threads.retain(|thread| !thread.is_resolved);
    }

    remote_thread_states
}

fn prepare_existing_thread_actions(
    existing_content: Option<&str>,
    remote_thread_states: &HashMap<String, bool>,
    include_resolved: bool,
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
        let remote_is_resolved = remote_thread_states.get(&thread.thread_id).copied();
        let will_be_serialized =
            remote_is_resolved.is_some_and(|is_resolved| include_resolved || !is_resolved);
        let mut lost_actions = Vec::new();
        if actions.draft_reply.is_some() && !will_be_serialized {
            lost_actions.push("draft reply");
        }
        if actions.resolve && remote_is_resolved.is_none() {
            lost_actions.push("resolve tick");
        }

        if !lost_actions.is_empty() && !force {
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

        if will_be_serialized {
            let actions = PreservedThreadActions {
                draft_reply: actions.draft_reply,
                resolve: actions.resolve && remote_is_resolved == Some(false),
            };
            if actions.draft_reply.is_some() || actions.resolve {
                preserved_actions.insert(thread.thread_id, actions);
            }
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
            verify_push_approval(&storage.threads_path(), &content)?;
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
mod pull_tests {
    use super::super::models::ReviewThread;
    use super::super::models::thread::CommentsNode;
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    const THREAD_WITH_DRAFT_AND_RESOLVE: &str = indoc! {r#"
        ---
        pr: 42
        repo: "example/project"
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

    const THREAD_WITHOUT_LOCAL_ACTIONS_TEMPLATE: &str = indoc! {r#"
        ---
        pr: 42
        repo: "example/project"
        pulled_at: "2024-01-15T10:00:00Z"
        submit: {submit}
        ---

        <!-- thread: RT_example path: src/example.rs:9 author: @reviewer -->
        - [ ] resolve
        <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
        Remote comment
        <!-- /comment -->
    "#};

    #[rstest]
    #[case::unparseable_file(
        "not threads markdown",
        None,
        "Re-pull would lose local data: threads.md could not be parsed, so draft replies and resolve ticks cannot be preserved: Invalid frontmatter: missing opening ---. Use --force to overwrite"
    )]
    #[case::missing_local_actions(
        THREAD_WITH_DRAFT_AND_RESOLVE,
        None,
        "Re-pull would lose local data: thread RT_example at src/example.rs:9: draft reply and resolve tick. Use --force to overwrite"
    )]
    #[case::draft_on_omitted_resolved_thread(
        THREAD_WITH_DRAFT_AND_RESOLVE,
        Some(true),
        "Re-pull would lose local data: thread RT_example at src/example.rs:9: draft reply. Use --force to overwrite"
    )]
    fn test_prepare_existing_thread_actions_blocks_when_local_data_would_be_lost(
        #[case] content: &str,
        #[case] remote_is_resolved: Option<bool>,
        #[case] expected: &str,
    ) {
        let remote_thread_states = remote_is_resolved
            .map(|is_resolved| HashMap::from([("RT_example".to_string(), is_resolved)]))
            .unwrap_or_default();

        let result =
            prepare_existing_thread_actions(Some(content), &remote_thread_states, false, false);

        assert_eq!(result.unwrap_err().to_string(), expected);
    }

    #[rstest]
    fn test_prepare_existing_thread_actions_preserves_local_actions() {
        let remote_thread_states = HashMap::from([("RT_example".to_string(), false)]);
        let expected = HashMap::from([(
            "RT_example".to_string(),
            PreservedThreadActions {
                draft_reply: Some("Draft reply".to_string()),
                resolve: true,
            },
        )]);

        assert_eq!(
            prepare_existing_thread_actions(
                Some(THREAD_WITH_DRAFT_AND_RESOLVE),
                &remote_thread_states,
                false,
                false,
            )
            .unwrap(),
            expected
        );
    }

    #[rstest]
    #[case::missing_thread_without_local_actions(false, false)]
    #[case::submit_approval_flag(true, false)]
    #[case::whitespace_only_edits(false, true)]
    fn test_prepare_existing_thread_actions_allows_changes_without_local_actions(
        #[case] submit: bool,
        #[case] whitespace_only_edit: bool,
    ) {
        let submit = if submit { "true" } else { "false" };
        let mut content = THREAD_WITHOUT_LOCAL_ACTIONS_TEMPLATE.replace("{submit}", submit);
        if whitespace_only_edit {
            content.push(' ');
        }

        assert_eq!(
            prepare_existing_thread_actions(Some(&content), &HashMap::new(), false, false,)
                .unwrap(),
            HashMap::new()
        );
    }

    #[rstest]
    fn test_prepare_existing_thread_actions_does_not_preserve_remote_resolve_state() {
        let content = THREAD_WITH_DRAFT_AND_RESOLVE.replace("Draft reply", "");
        let remote_thread_states = HashMap::from([("RT_example".to_string(), true)]);

        assert_eq!(
            prepare_existing_thread_actions(Some(&content), &remote_thread_states, false, false,)
                .unwrap(),
            HashMap::new()
        );
    }

    #[rstest]
    fn test_prepare_pull_data_tracks_resolved_threads_before_filtering() {
        let mut pr_data = PrData {
            reviews: vec![],
            threads: vec![
                ReviewThread {
                    id: Some("RT_resolved_example".to_string()),
                    is_resolved: true,
                    comments: CommentsNode { nodes: vec![] },
                },
                ReviewThread {
                    id: Some("RT_active_example".to_string()),
                    is_resolved: false,
                    comments: CommentsNode { nodes: vec![] },
                },
            ],
        };
        let thread_states = prepare_pull_data(&mut pr_data, false);
        let remaining_threads = pr_data
            .threads
            .iter()
            .map(|thread| (thread.id.clone(), thread.is_resolved))
            .collect::<Vec<_>>();
        assert_eq!(
            (thread_states, remaining_threads),
            (
                HashMap::from([
                    ("RT_resolved_example".to_string(), true),
                    ("RT_active_example".to_string(), false),
                ]),
                vec![(Some("RT_active_example".to_string()), false)],
            )
        );
    }

    #[rstest]
    #[case::unparseable_file("not threads markdown")]
    #[case::missing_thread_with_actions(THREAD_WITH_DRAFT_AND_RESOLVE)]
    fn test_force_allows_overwriting_unpreservable_data(#[case] content: &str) {
        assert_eq!(
            prepare_existing_thread_actions(Some(content), &HashMap::new(), false, true).unwrap(),
            HashMap::new()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::gh::pr_review::changeset::ReplyAction;
    use crate::shared::human_in_the_loop::HumanInTheLoopError;
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

        assert_eq!(verify_push_approval(&path, content).is_ok(), expected);
    }

    #[rstest]
    fn test_verify_push_approval_rejects_file_changed_after_parse() {
        crate::shared::testing::init_approval_dir();
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("threads.md");
        let parsed_content = indoc! {"
            ---
            pr: 1
            repo: example/repo
            pulled_at: '2024-01-15T10:00:00Z'
            submit: true
            ---
            Approved reply"};
        std::fs::write(&path, parsed_content).unwrap();
        let parsed_content = std::fs::read_to_string(&path).unwrap();
        Document::<ThreadsFrontmatter>::from_path(path.clone())
            .unwrap()
            .save_approval()
            .unwrap();
        std::fs::write(
            &path,
            parsed_content.replace("Approved reply", "Unapproved reply"),
        )
        .unwrap();

        let error = verify_push_approval(&path, &parsed_content).unwrap_err();

        assert!(matches!(
            error.downcast_ref::<HumanInTheLoopError>(),
            Some(HumanInTheLoopError::ModifiedAfterApproval)
        ));
    }
}
