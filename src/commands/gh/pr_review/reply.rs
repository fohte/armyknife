use std::collections::HashMap;
use std::future::Future;

use clap::Args;

use super::api::fetch_pr_data;
use super::changeset::ReplyChangeSet;
use super::error::PrReviewError;
use super::markdown::serializer::{PreservedThreadActions, SerializeOptions, ThreadsFrontmatter};
use super::markdown::{MarkdownParser, MarkdownSerializer};
use super::models::PrData;
use super::storage::ThreadStorage;
use crate::commands::agent::{ensure_no_review_access, is_vibe_enabled};
use crate::infra::git;
use crate::infra::github::GitHubClient;
use crate::shared::config::load_config;
use crate::shared::human_in_the_loop::{Document, request_review_finish_after_push};

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

    /// Skip editor approval when vibe mode and the target repository are allowed
    #[arg(long)]
    pub no_review: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PushPlan {
    NoChanges,
    DryRun,
    Apply,
}

fn push_plan(changeset: &ReplyChangeSet, dry_run: bool) -> PushPlan {
    if dry_run {
        PushPlan::DryRun
    } else if !changeset.has_changes() {
        PushPlan::NoChanges
    } else {
        PushPlan::Apply
    }
}

fn request_reply_review_finish_after_push(path: &std::path::Path) -> anyhow::Result<bool> {
    request_review_finish_after_push(path).map_err(Into::into)
}

fn verify_push_approval(path: &std::path::Path, content: &str) -> anyhow::Result<()> {
    let document = Document::<ThreadsFrontmatter>::from_content(path.to_path_buf(), content)?;
    document
        .approval_manager()
        .verify_content_matches_file(content.as_bytes())?;
    Ok(())
}

fn verify_push_approval_if_required(
    path: &std::path::Path,
    content: &str,
    no_review: bool,
) -> anyhow::Result<()> {
    if no_review {
        return verify_push_content_matches_file(path, content);
    }

    verify_push_approval(path, content)
}

fn verify_push_content_matches_file(path: &std::path::Path, content: &str) -> anyhow::Result<()> {
    if std::fs::read(path)? != content.as_bytes() {
        anyhow::bail!("threads.md changed while preparing the push; no changes were applied");
    }

    Ok(())
}

async fn run_push_after_access<Fut>(
    plan: PushPlan,
    threads_path: &std::path::Path,
    apply_and_refresh: impl FnOnce() -> Fut,
    finish_review: impl FnOnce(&std::path::Path) -> anyhow::Result<bool>,
) -> anyhow::Result<()>
where
    Fut: Future<Output = anyhow::Result<()>>,
{
    match plan {
        PushPlan::NoChanges => {}
        PushPlan::DryRun => return Ok(()),
        PushPlan::Apply => apply_and_refresh().await?,
    }

    if let Err(error) = finish_review(threads_path) {
        eprintln!("Warning: Failed to finish the open PR reply review: {error}");
    }

    Ok(())
}

async fn run_push_with_access<Fut>(
    no_review: bool,
    owner: &str,
    repo: &str,
    access_check: impl FnOnce(&str, &str) -> anyhow::Result<()>,
    push: impl FnOnce() -> Fut,
) -> anyhow::Result<()>
where
    Fut: Future<Output = anyhow::Result<()>>,
{
    if no_review {
        access_check(owner, repo)?;
    }

    push().await
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
    run_push_with_retry(args, None).await
}

pub(super) async fn run_push_with_retry(
    args: &ReplyPushArgs,
    retry_started_at: Option<chrono::DateTime<chrono::Utc>>,
) -> anyhow::Result<()> {
    let (owner, repo) = git::get_repo_owner_and_name(args.repo.as_deref())?;

    run_push_with_access(
        args.no_review,
        &owner,
        &repo,
        |owner, repo| ensure_no_review_access(is_vibe_enabled, load_config, owner, repo),
        || run_push_after_access_with_remote_state(args, &owner, &repo, retry_started_at),
    )
    .await
}

async fn run_push_after_access_with_remote_state(
    args: &ReplyPushArgs,
    owner: &str,
    repo: &str,
    retry_started_at: Option<chrono::DateTime<chrono::Utc>>,
) -> anyhow::Result<()> {
    let storage = ThreadStorage::new(owner, repo, args.pr_number);

    // Read and parse local data
    if !storage.exists() {
        return Err(PrReviewError::NoPulledData.into());
    }

    let content = storage.read_threads()?;
    let local = MarkdownParser::parse(&content)?;

    // Fetch latest remote state
    let remote = fetch_pr_data(owner, repo, args.pr_number, true).await?;

    let current_user = if local
        .threads
        .iter()
        .any(|thread| thread.draft_reply.is_some())
    {
        Some(GitHubClient::get()?.get_current_user().await?)
    } else {
        None
    };

    // Detect changes
    let changeset = ReplyChangeSet::detect_with_retry(
        &local,
        &remote,
        &local.frontmatter.pulled_at,
        current_user.as_deref().unwrap_or_default(),
        retry_started_at,
    );

    // Check for conflicts
    if changeset.has_conflicts() && !args.force {
        return Err(PrReviewError::ConflictDetected {
            count: changeset.conflicts.len(),
            details: changeset.format_conflicts(),
        }
        .into());
    }

    let plan = push_plan(&changeset, args.dry_run);
    if plan == PushPlan::NoChanges {
        println!("No changes to push.");
    } else if plan == PushPlan::DryRun {
        changeset.display();
        println!("\n(dry-run mode: no changes applied)");
    }

    let threads_path = storage.threads_path();
    run_push_after_access(
        plan,
        &threads_path,
        || async {
            changeset.display();
            let client = GitHubClient::get()?;
            verify_push_approval_if_required(&threads_path, &content, args.no_review)?;
            changeset.apply(client, owner, repo, args.pr_number).await?;

            // Re-fetch and update local file after push
            let include_resolved = local.threads.iter().any(|t| t.resolve);
            let updated_remote =
                fetch_pr_data(owner, repo, args.pr_number, include_resolved).await?;

            let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
            let frontmatter = ThreadsFrontmatter {
                pr: args.pr_number,
                repo: format!("{owner}/{repo}"),
                pulled_at: now,
                submit: false,
            };

            let updated_content = MarkdownSerializer::serialize(&updated_remote, &frontmatter);
            storage.write_threads(&updated_content)?;
            Ok(())
        },
        request_reply_review_finish_after_push,
    )
    .await?;

    if plan == PushPlan::Apply {
        println!("\nPush complete.");
    }

    Ok(())
}

#[cfg(test)]
mod pull_tests {
    use super::super::models::ReviewThread;
    use super::super::models::thread::CommentsNode;
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    #[cfg(unix)]
    #[tokio::test]
    async fn successful_no_change_push_finishes_active_reply_review() {
        use std::ffi::CString;
        use std::fs;
        use std::io::{Read, Write};
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::OpenOptionsExt;
        use std::sync::mpsc;
        use std::thread;
        use std::time::{Duration, Instant};

        use crate::shared::human_in_the_loop::{
            LockGuard, finish_ack_fifo_path, finish_request_fifo_path,
        };

        let temp_dir = tempfile::tempdir().expect("temporary review directory");
        let threads_path = temp_dir.path().join("threads.md");
        fs::write(&threads_path, "review").expect("write review document");
        let _review_lock = LockGuard::acquire(&threads_path).expect("active review lock");
        let finish_fifo_path = finish_request_fifo_path(&threads_path);
        let ack_fifo_path = finish_ack_fifo_path(&threads_path);
        for fifo_path in [&finish_fifo_path, &ack_fifo_path] {
            let c_path = CString::new(fifo_path.as_os_str().as_bytes()).expect("valid FIFO path");
            // SAFETY: `c_path` is a valid NUL-terminated path owned by this scope.
            assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        }

        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let review_actor = thread::spawn(move || -> std::io::Result<u8> {
            let mut finish_reader = fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&finish_fifo_path)?;
            ready_sender
                .send(())
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let mut signal = [0_u8; 1];
                match finish_reader.read(&mut signal) {
                    Ok(1) => {
                        fs::OpenOptions::new()
                            .write(true)
                            .open(&ack_fifo_path)?
                            .write_all(b"0")?;
                        return Ok(signal[0]);
                    }
                    Ok(0) => {}
                    Ok(_) => unreachable!(),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error),
                }
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "finish request was not received",
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        ready_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("review actor ready");

        let push_result = run_push_after_access(
            PushPlan::NoChanges,
            &threads_path,
            || async { Ok(()) },
            request_reply_review_finish_after_push,
        )
        .await
        .map_err(|error| error.to_string());
        let finish_reason = review_actor
            .join()
            .expect("review actor thread")
            .map_err(|error| error.to_string());

        assert_eq!((push_result, finish_reason), (Ok(()), Ok(b'p')));
    }

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
                    diff_side: None,
                    start_diff_side: None,
                    comments: CommentsNode { nodes: vec![] },
                },
                ReviewThread {
                    id: Some("RT_active_example".to_string()),
                    is_resolved: false,
                    diff_side: None,
                    start_diff_side: None,
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
    use crate::commands::gh::GhCommands;
    use crate::commands::gh::pr_review::changeset::ReplyAction;
    use crate::commands::gh::pr_review::{PrReviewCommands, ReplyCommands};
    use crate::shared::config::Config;
    use crate::shared::human_in_the_loop::HumanInTheLoopError;
    use clap::Parser;
    use indoc::indoc;
    use rstest::rstest;
    use tempfile::TempDir;

    #[rstest]
    #[case::no_changes(false, false, PushPlan::NoChanges)]
    #[case::dry_run_no_changes(false, true, PushPlan::DryRun)]
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
    #[case::no_changes_finishes_review(
        PushPlan::NoChanges,
        false,
        false,
        Ok(()),
        vec!["finish"],
        vec![std::path::PathBuf::from("reply-review.md")],
    )]
    #[case::dry_run_keeps_review_open(
        PushPlan::DryRun,
        false,
        false,
        Ok(()),
        vec![],
        vec![],
    )]
    #[case::applied_push_finishes_review(
        PushPlan::Apply,
        false,
        false,
        Ok(()),
        vec!["apply", "finish"],
        vec![std::path::PathBuf::from("reply-review.md")],
    )]
    #[case::failed_push_keeps_review_open(
        PushPlan::Apply,
        true,
        false,
        Err("push failed".to_string()),
        vec!["apply"],
        vec![],
    )]
    #[case::finish_failure_does_not_fail_push(
        PushPlan::NoChanges,
        false,
        true,
        Ok(()),
        vec!["finish"],
        vec![std::path::PathBuf::from("reply-review.md")],
    )]
    #[tokio::test]
    async fn run_push_after_access_routes_finish_requests_after_successful_pushes(
        #[case] plan: PushPlan,
        #[case] apply_fails: bool,
        #[case] finish_fails: bool,
        #[case] expected_result: Result<(), String>,
        #[case] expected_steps: Vec<&'static str>,
        #[case] expected_finish_paths: Vec<std::path::PathBuf>,
    ) {
        let steps = std::cell::RefCell::new(Vec::new());
        let finish_paths = std::cell::RefCell::new(Vec::new());
        let threads_path = std::path::PathBuf::from("reply-review.md");
        let result = run_push_after_access(
            plan,
            &threads_path,
            || {
                steps.borrow_mut().push("apply");
                async move {
                    if apply_fails {
                        anyhow::bail!("push failed");
                    }
                    Ok(())
                }
            },
            |path| {
                steps.borrow_mut().push("finish");
                finish_paths.borrow_mut().push(path.to_path_buf());
                if finish_fails {
                    anyhow::bail!("finish failed");
                }
                Ok(true)
            },
        )
        .await
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, steps.into_inner(), finish_paths.into_inner()),
            (expected_result, expected_steps, expected_finish_paths)
        );
    }

    #[test]
    fn push_parses_no_review_flag() {
        let parsed = crate::cli::Cli::try_parse_from([
            "a",
            "gh",
            "pr-review",
            "reply",
            "push",
            "42",
            "--no-review",
        ])
        .expect("--no-review should be a valid push option");

        assert!(
            parsed.command
                == crate::cli::Commands::Gh(GhCommands::PrReview {
                    command: PrReviewCommands::Reply {
                        command: ReplyCommands::Push(ReplyPushArgs {
                            pr_number: 42,
                            repo: None,
                            dry_run: false,
                            force: false,
                            no_review: true,
                        }),
                    },
                }),
        );
    }

    #[rstest]
    #[case::vibe_and_repo_allowed(
        true,
        indoc! {"
            orgs:
              example-org:
                vibe: true
        "},
        Ok(())
    )]
    #[case::vibe_disabled(
        false,
        indoc! {"
            orgs:
              example-org:
                vibe: true
        "},
        Err("The --no-review option is only available when vibe mode is enabled.".to_string())
    )]
    #[case::repo_access_denied(
        true,
        "{}",
        Err("Vibe mode is disabled by configuration for repository example-org/example-repo.".to_string())
    )]
    fn no_review_requires_vibe_and_repo_access(
        #[case] vibe_enabled: bool,
        #[case] config_yaml: &str,
        #[case] expected: Result<(), String>,
    ) {
        let config: Config = serde_yaml::from_str(config_yaml).unwrap();

        let result = ensure_no_review_access(
            || Ok(vibe_enabled),
            || Ok(config),
            "example-org",
            "example-repo",
        )
        .map_err(|error| error.to_string());

        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn no_review_access_denial_stops_push_before_work() {
        let push_started = std::cell::Cell::new(false);

        let result = run_push_with_access(
            true,
            "example-org",
            "example-repo",
            |_, _| anyhow::bail!("vibe access denied"),
            || async {
                push_started.set(true);
                Ok(())
            },
        )
        .await
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, push_started.get()),
            (Err("vibe access denied".to_string()), false)
        );
    }

    #[rstest]
    #[case::approval_still_required(false, false)]
    #[case::approval_skipped_for_no_review(true, true)]
    fn push_approval_is_skipped_only_for_no_review(
        #[case] no_review: bool,
        #[case] expected_success: bool,
    ) {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("threads.md");
        let content = indoc! {"
            ---
            pr: 1
            repo: example/repo
            pulled_at: '2024-01-15T10:00:00Z'
            submit: false
            ---
            Draft reply"};
        std::fs::write(&path, content).unwrap();

        assert_eq!(
            verify_push_approval_if_required(&path, content, no_review).is_ok(),
            expected_success,
        );
    }

    #[test]
    fn no_review_still_rejects_a_threads_file_changed_after_read() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("threads.md");
        let content = indoc! {"
            ---
            pr: 1
            repo: example/repo
            pulled_at: '2024-01-15T10:00:00Z'
            submit: false
            ---
            Original reply"};
        std::fs::write(&path, content.replace("Original reply", "New reply")).unwrap();

        assert_eq!(
            verify_push_approval_if_required(&path, content, true)
                .map_err(|error| error.to_string()),
            Err("threads.md changed while preparing the push; no changes were applied".to_string()),
        );
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
