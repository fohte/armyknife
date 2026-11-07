use std::{future::Future, io::Write, path::PathBuf, process::Output};

use anyhow::{Context, bail};
use clap::Args;

use crate::commands::agent::{close_worktree_path, ensure_repo_vibe_access, is_vibe_enabled};
use crate::infra::{
    external_tool::ExternalTool,
    git::{self, GitRepo, github_owner_and_repo},
    github::{
        AsyncMergeResult, GitHubClient, MergeResult, PrStackClient, PrState, PullRequestStack,
        PullRequestStackEntry,
    },
};
use crate::shared::{
    config::{Config, load_config},
    env_var::EnvVars,
    hooks,
    repos_root::{discover_repos_with_worktrees, resolve_repos_root},
    worktree::{LinkedWorktree, list_linked_worktrees},
};

mod checks;
mod merge_flow;

use merge_flow::run_merge;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct PrMergeArgs {
    /// PR number
    pub pr_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,
}

pub async fn run(args: &PrMergeArgs) -> anyhow::Result<()> {
    let in_agent_session = EnvVars::load().own_session_id().is_some();
    let vibe_enabled = in_agent_session && is_vibe_enabled()?;
    ensure_vibe_access(in_agent_session, vibe_enabled)?;

    let (owner, repo) = git::get_repo_owner_and_name(args.repo.as_deref())?;
    ensure_repo_vibe_access_for_session(in_agent_session, load_config, &owner, &repo)?;

    let client = GhCli;
    run_merge_and_close(
        &client,
        &mut |env| hooks::run_hook("pre-pr-merge", env),
        &owner,
        &repo,
        args.pr_number,
        |merged_pr| async move { close_merged_head_worktree(&merged_pr).await },
    )
    .await?;
    Ok(())
}

fn ensure_vibe_access(in_agent_session: bool, vibe_enabled: bool) -> anyhow::Result<()> {
    if in_agent_session && !vibe_enabled {
        bail!("PR merge is only allowed in vibe mode inside an agent session.");
    }
    Ok(())
}

fn ensure_repo_vibe_access_for_session(
    in_agent_session: bool,
    load_config: impl FnOnce() -> anyhow::Result<Config>,
    owner: &str,
    repo: &str,
) -> anyhow::Result<()> {
    if in_agent_session {
        ensure_repo_vibe_access(&load_config()?, owner, repo)?;
    }
    Ok(())
}

trait PrMergeClient {
    fn view_pr(&self, owner: &str, repo: &str, pr_number: u64) -> anyhow::Result<PrSnapshot>;
    async fn get_stack_for_pull_request(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> anyhow::Result<Option<PullRequestStack>>;
    async fn get_pull_requests_in_stack(
        &self,
        owner: &str,
        repo: &str,
        stack_number: u64,
    ) -> anyhow::Result<Vec<PullRequestStackEntry>>;
    fn wait_for_required_checks(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> anyhow::Result<()>;
    async fn merge_async(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        head_sha: &str,
    ) -> anyhow::Result<AsyncMergeResult>;
    async fn merge_sync(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        head_sha: &str,
    ) -> anyhow::Result<MergeResult>;
    async fn branch_requires_merge_queue(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> anyhow::Result<bool>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrSnapshot {
    state: PrState,
    head_sha: String,
    head_ref_name: String,
    base_ref_name: String,
    head_repository: Option<String>,
    merge_state_status: String,
    url: String,
}

#[derive(Debug, PartialEq, Eq)]
enum MergeOutcome {
    Merged(PrSnapshot),
    AlreadyMerged(PrSnapshot),
    Enqueued(String),
}

fn unmerged_lower_pr(
    stack_pull_requests: &[PullRequestStackEntry],
    pull_request: u64,
) -> anyhow::Result<Option<u64>> {
    let current_index = stack_pull_requests
        .iter()
        .position(|stack_pull_request| stack_pull_request.number == pull_request)
        .with_context(|| format!("PR #{pull_request} is missing from its stack response"))?;

    Ok(stack_pull_requests[..current_index]
        .iter()
        .find(|stack_pull_request| stack_pull_request.merged_at.is_none())
        .map(|stack_pull_request| stack_pull_request.number))
}

async fn run_merge_and_close<C, F>(
    client: &impl PrMergeClient,
    hook_runner: &mut impl FnMut(&[(&str, &str)]) -> anyhow::Result<()>,
    owner: &str,
    repo: &str,
    pr_number: u64,
    close_worktree: C,
) -> anyhow::Result<()>
where
    C: FnOnce(PrSnapshot) -> F,
    F: Future<Output = anyhow::Result<()>>,
{
    match run_merge(client, hook_runner, owner, repo, pr_number).await? {
        MergeOutcome::Merged(merged) => {
            println!("PR #{pr_number} merged.");
            let _ = std::io::stdout().flush();
            if let Err(error) = close_worktree(merged.clone()).await {
                eprintln!(
                    "Could not close worktree for merged PR #{pr_number} (branch `{}`): {error:#}",
                    merged.head_ref_name,
                );
            }
        }
        MergeOutcome::AlreadyMerged(_) => {
            println!("PR #{pr_number} was already merged.");
        }
        MergeOutcome::Enqueued(message) => {
            println!("PR #{pr_number} was added to the merge queue and has not merged: {message}");
        }
    }
    Ok(())
}

async fn close_merged_head_worktree(merged_pr: &PrSnapshot) -> anyhow::Result<()> {
    let Some((owner, repo_name)) = merged_pr
        .head_repository
        .as_deref()
        .and_then(|repository| git::parse_repo(repository).ok())
    else {
        return Ok(());
    };
    let config = load_config()?;
    match find_local_head_worktree(&owner, &repo_name, &merged_pr.head_ref_name, &config) {
        HeadWorktreeSelection::NotFound => Ok(()),
        HeadWorktreeSelection::Unique(worktree_path) => close_worktree_path(&worktree_path).await,
        HeadWorktreeSelection::Ambiguous(count) => {
            eprintln!(
                "Multiple local worktrees match merged PR head branch `{}` ({count} matches); skipping close.",
                merged_pr.head_ref_name,
            );
            Ok(())
        }
    }
}

fn find_local_head_worktree(
    owner: &str,
    repo_name: &str,
    head_branch: &str,
    config: &Config,
) -> HeadWorktreeSelection {
    let mut repositories = Vec::new();
    let mut visited_repos = Vec::<PathBuf>::new();

    if let Ok(current_repo) = GitRepo::open_from_env() {
        collect_repository_worktrees(&current_repo, &mut visited_repos, &mut repositories);
    }

    let Some(repos_root) = resolve_repos_root(config.agent.worktree.repos_root.as_deref()).ok()
    else {
        return select_head_worktree(repositories, owner, repo_name, head_branch);
    };
    for repo_path in discover_repos_with_worktrees(&repos_root, &config.agent.worktree.dir) {
        let Ok(repo) = GitRepo::open_at(&repo_path) else {
            continue;
        };
        collect_repository_worktrees(&repo, &mut visited_repos, &mut repositories);
    }

    select_head_worktree(repositories, owner, repo_name, head_branch)
}

#[derive(Debug, Clone)]
struct LocalRepositoryWorktrees {
    repository: Option<(String, String)>,
    worktrees: Vec<LinkedWorktree>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HeadWorktreeSelection {
    NotFound,
    Unique(PathBuf),
    Ambiguous(usize),
}

fn collect_repository_worktrees(
    repo: &GitRepo,
    visited_repos: &mut Vec<PathBuf>,
    repositories: &mut Vec<LocalRepositoryWorktrees>,
) {
    let Ok(main_repo) = repo.main_repo() else {
        return;
    };
    if visited_repos.iter().any(|path| path == main_repo.workdir()) {
        return;
    }
    visited_repos.push(main_repo.workdir().to_path_buf());

    let repository = github_owner_and_repo(&main_repo).ok();
    let worktrees = if repository.is_some() {
        list_linked_worktrees(&main_repo)
            .map(existing_worktrees)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    repositories.push(LocalRepositoryWorktrees {
        repository,
        worktrees,
    });
}

fn select_head_worktree(
    repositories: Vec<LocalRepositoryWorktrees>,
    owner: &str,
    repo_name: &str,
    head_branch: &str,
) -> HeadWorktreeSelection {
    let worktrees = repositories
        .into_iter()
        .filter(|repository| {
            repository.repository.as_ref().is_some_and(|id| {
                id.0.eq_ignore_ascii_case(owner) && id.1.eq_ignore_ascii_case(repo_name)
            })
        })
        .flat_map(|repository| repository.worktrees)
        .filter(|worktree| worktree.branch == head_branch)
        .map(|worktree| worktree.path)
        .collect();
    unique_worktree_path(worktrees)
}

fn existing_worktrees(worktrees: Vec<LinkedWorktree>) -> Vec<LinkedWorktree> {
    worktrees
        .into_iter()
        .filter(|worktree| worktree.path.is_dir())
        .collect()
}

fn unique_worktree_path(mut worktrees: Vec<PathBuf>) -> HeadWorktreeSelection {
    worktrees.sort();
    worktrees.dedup();
    match worktrees.len() {
        0 => HeadWorktreeSelection::NotFound,
        1 => HeadWorktreeSelection::Unique(worktrees.remove(0)),
        count => HeadWorktreeSelection::Ambiguous(count),
    }
}

fn ensure_pr_is_open_and_safe(
    pr: &PrSnapshot,
    pr_number: u64,
    expected_head_sha: Option<&str>,
) -> anyhow::Result<()> {
    if pr.state != PrState::Open {
        bail!("PR #{pr_number} is not open (state: {:?}).", pr.state);
    }
    if pr.head_sha.is_empty() {
        bail!("PR #{pr_number} did not report a head SHA.");
    }
    if expected_head_sha.is_some_and(|expected| expected != pr.head_sha) {
        bail!("PR #{pr_number} head changed before merge.");
    }
    if pr.merge_state_status == "DIRTY" {
        bail!("PR #{pr_number} has merge conflicts.");
    }
    Ok(())
}

struct GhCli;

impl PrMergeClient for GhCli {
    fn view_pr(&self, owner: &str, repo: &str, pr_number: u64) -> anyhow::Result<PrSnapshot> {
        let pr_number = pr_number.to_string();
        let repository = format!("{owner}/{repo}");
        let args = [
            "pr",
            "view",
            pr_number.as_str(),
            "--repo",
            repository.as_str(),
            "--json",
            "state,headRefOid,headRefName,baseRefName,headRepository,mergeStateStatus,url",
        ];
        let output = run_gh(&args)?;
        let response: GhPrResponse = serde_json::from_slice(&output.stdout)
            .context("Failed to parse gh pr view response")?;
        Ok(PrSnapshot {
            state: parse_pr_state(&response.state)?,
            head_sha: response.head_ref_oid,
            head_ref_name: response.head_ref_name,
            base_ref_name: response.base_ref_name,
            head_repository: response
                .head_repository
                .map(|repository| repository.name_with_owner),
            merge_state_status: response.merge_state_status.unwrap_or_default(),
            url: response.url,
        })
    }

    async fn get_stack_for_pull_request(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> anyhow::Result<Option<PullRequestStack>> {
        GitHubClient::get()?
            .get_stack_for_pull_request(owner, repo, pr_number)
            .await
    }

    async fn get_pull_requests_in_stack(
        &self,
        owner: &str,
        repo: &str,
        stack_number: u64,
    ) -> anyhow::Result<Vec<PullRequestStackEntry>> {
        GitHubClient::get()?
            .get_pull_requests_in_stack(owner, repo, stack_number)
            .await
    }

    fn wait_for_required_checks(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> anyhow::Result<()> {
        checks::wait_for_required_checks(owner, repo, pr_number)
    }

    async fn merge_async(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        head_sha: &str,
    ) -> anyhow::Result<AsyncMergeResult> {
        GitHubClient::get()?
            .merge_pull_request_async(owner, repo, pr_number, head_sha)
            .await
    }

    async fn merge_sync(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        head_sha: &str,
    ) -> anyhow::Result<MergeResult> {
        GitHubClient::get()?
            .merge_pull_request(owner, repo, pr_number, head_sha)
            .await
    }

    async fn branch_requires_merge_queue(
        &self,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> anyhow::Result<bool> {
        GitHubClient::get()?
            .branch_requires_merge_queue(owner, repo, branch)
            .await
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPrResponse {
    state: String,
    head_ref_oid: String,
    head_ref_name: String,
    base_ref_name: String,
    head_repository: Option<GhRepository>,
    merge_state_status: Option<String>,
    url: String,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhRepository {
    name_with_owner: String,
}

fn run_gh(args: &[&str]) -> anyhow::Result<Output> {
    let mut command = ExternalTool::Gh.command();
    command.args(args);
    let output = command
        .output()
        .with_context(|| format!("Failed to run gh {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("gh {} failed: {}", args.join(" "), stderr.trim());
    }
    Ok(output)
}

fn parse_pr_state(state: &str) -> anyhow::Result<PrState> {
    match state {
        "OPEN" => Ok(PrState::Open),
        "CLOSED" => Ok(PrState::Closed),
        "MERGED" => Ok(PrState::Merged),
        _ => bail!("Unknown PR state returned by gh: {state}"),
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    use indoc::indoc;
    use rstest::{fixture, rstest};

    use crate::infra::github::AsyncMergeFallback;
    use crate::shared::config::Config;

    use super::*;

    struct FakePrMergeClient {
        snapshots: RefCell<VecDeque<PrSnapshot>>,
        events: Rc<RefCell<Vec<String>>>,
        checks_error: bool,
        merge_error: bool,
        fallback_error: Option<AsyncMergeFallback>,
        sync_merge_error: bool,
        merge_queue_enabled: bool,
        stack: Option<PullRequestStack>,
        stack_pull_requests: Vec<PullRequestStackEntry>,
        merge_result: AsyncMergeResult,
    }

    #[fixture]
    fn events() -> Rc<RefCell<Vec<String>>> {
        Rc::new(RefCell::new(Vec::new()))
    }

    impl PrMergeClient for FakePrMergeClient {
        fn view_pr(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
        ) -> anyhow::Result<PrSnapshot> {
            self.events.borrow_mut().push("view".to_string());
            self.snapshots
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("No fake PR snapshot remains"))
        }

        async fn get_stack_for_pull_request(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
        ) -> anyhow::Result<Option<PullRequestStack>> {
            self.events.borrow_mut().push("stack".to_string());
            Ok(self.stack.clone())
        }

        async fn get_pull_requests_in_stack(
            &self,
            _owner: &str,
            _repo: &str,
            _stack_number: u64,
        ) -> anyhow::Result<Vec<PullRequestStackEntry>> {
            self.events
                .borrow_mut()
                .push("stack-pull-requests".to_string());
            Ok(self.stack_pull_requests.clone())
        }

        fn wait_for_required_checks(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
        ) -> anyhow::Result<()> {
            self.events.borrow_mut().push("checks".to_string());
            if self.checks_error {
                anyhow::bail!("fake required checks failed");
            }
            Ok(())
        }

        async fn merge_async(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
            head_sha: &str,
        ) -> anyhow::Result<AsyncMergeResult> {
            self.events.borrow_mut().push(format!("merge:{head_sha}"));
            if self.merge_error {
                anyhow::bail!("fake merge failed");
            }
            if let Some(fallback_error) = &self.fallback_error {
                return Err(anyhow::Error::new(fallback_error.clone()));
            }
            Ok(self.merge_result.clone())
        }

        async fn merge_sync(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
            head_sha: &str,
        ) -> anyhow::Result<MergeResult> {
            self.events
                .borrow_mut()
                .push(format!("merge-sync:{head_sha}"));
            if self.sync_merge_error {
                anyhow::bail!("fake synchronous merge failed");
            }
            Ok(MergeResult::Merged {
                sha: Some("sync-merge".to_string()),
            })
        }

        async fn branch_requires_merge_queue(
            &self,
            _owner: &str,
            _repo: &str,
            branch: &str,
        ) -> anyhow::Result<bool> {
            self.events
                .borrow_mut()
                .push(format!("merge-queue:{branch}"));
            Ok(self.merge_queue_enabled)
        }
    }

    fn snapshot(state: PrState, head_sha: &str, merge_state_status: &str) -> PrSnapshot {
        PrSnapshot {
            state,
            head_sha: head_sha.to_string(),
            head_ref_name: "topic/example-branch".to_string(),
            base_ref_name: "main".to_string(),
            head_repository: Some("sample-org/sample-repo".to_string()),
            merge_state_status: merge_state_status.to_string(),
            url: "https://github.com/sample-org/sample-repo/pull/42".to_string(),
        }
    }

    fn fake_client(
        snapshots: &[PrSnapshot],
        events: Rc<RefCell<Vec<String>>>,
        checks_error: bool,
        merge_error: bool,
    ) -> FakePrMergeClient {
        FakePrMergeClient {
            snapshots: RefCell::new(snapshots.iter().cloned().collect()),
            events,
            checks_error,
            merge_error,
            fallback_error: None,
            sync_merge_error: false,
            merge_queue_enabled: false,
            stack: None,
            stack_pull_requests: Vec::new(),
            merge_result: AsyncMergeResult::Merged {
                sha: Some("merge-a".to_string()),
            },
        }
    }

    fn timed_out_merge_error() -> AsyncMergeFallback {
        AsyncMergeFallback::TimedOut {
            request_uuid: "request-id".to_string(),
            pull_request: 42,
        }
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_calls_hook_and_checks_before_async_merge(events: Rc<RefCell<Vec<String>>>) {
        let mut client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Merged, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        client.stack = Some(PullRequestStack {
            number: 99,
            position: 2,
            size: 2,
        });
        client.stack_pull_requests = vec![
            PullRequestStackEntry {
                number: 41,
                merged_at: Some("2026-01-02T03:04:05Z".to_string()),
            },
            PullRequestStackEntry {
                number: 42,
                merged_at: None,
            },
        ];
        let mut hook = |env: &[(&str, &str)]| {
            events.borrow_mut().push(format!("hook:{env:?}"));
            Ok(())
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42).await;

        assert_eq!(
            (result.is_ok(), events.borrow().clone()),
            (
                true,
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "stack-pull-requests".to_string(),
                    "hook:[(\"ARMYKNIFE_PR_NUMBER\", \"42\"), (\"ARMYKNIFE_PR_OWNER\", \"example-org\"), (\"ARMYKNIFE_PR_REPO\", \"example-repo\"), (\"ARMYKNIFE_PR_URL\", \"https://github.com/sample-org/sample-repo/pull/42\"), (\"ARMYKNIFE_PR_HEAD_SHA\", \"head-a\")]".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[case::merged_before_retry(
        vec![snapshot(PrState::Merged, "head-a", "CLEAN")],
        MergeOutcome::AlreadyMerged(snapshot(PrState::Merged, "head-a", "CLEAN")),
        vec!["view".to_string()]
    )]
    #[case::merged_during_required_checks(
        vec![
            snapshot(PrState::Open, "head-a", "CLEAN"),
            snapshot(PrState::Merged, "head-a", "CLEAN")
        ],
        MergeOutcome::Merged(snapshot(PrState::Merged, "head-a", "CLEAN")),
        vec![
            "view".to_string(),
            "stack".to_string(),
            "hook".to_string(),
            "checks".to_string(),
            "view".to_string()
        ]
    )]
    #[tokio::test]
    async fn run_merge_accepts_a_pr_merged_before_or_during_retry(
        events: Rc<RefCell<Vec<String>>>,
        #[case] snapshots: Vec<PrSnapshot>,
        #[case] expected_outcome: MergeOutcome,
        #[case] expected_events: Vec<String>,
    ) {
        let client = fake_client(&snapshots, Rc::clone(&events), false, false);
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            Ok(())
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42)
            .await
            .map_err(|error| error.to_string());

        assert_eq!(
            (result, events.borrow().clone()),
            (Ok(expected_outcome), expected_events),
        );
    }

    #[rstest]
    #[case::timeout(timed_out_merge_error())]
    #[case::different_pending_head(AsyncMergeFallback::HeadMismatch {
        request_uuid: "request-id".to_string(),
        pull_request: 42,
        pending_head_sha: "older-head".to_string(),
        current_head_sha: "head-a".to_string(),
    })]
    #[case::head_unavailable(AsyncMergeFallback::HeadUnavailable {
        request_uuid: "request-id".to_string(),
        pull_request: 42,
    })]
    #[case::request_uuid_mismatch(AsyncMergeFallback::RequestUuidMismatch {
        request_uuid: "request-id".to_string(),
        response_uuid: "different-request".to_string(),
        pull_request: 42,
    })]
    #[tokio::test]
    async fn run_merge_falls_back_to_sync_merge_with_the_confirmed_head(
        events: Rc<RefCell<Vec<String>>>,
        #[case] fallback_error: AsyncMergeFallback,
    ) {
        let mut client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Merged, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        client.fallback_error = Some(fallback_error);
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            Ok(())
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42).await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                events.borrow().clone(),
            ),
            (
                Ok(MergeOutcome::Merged(snapshot(
                    PrState::Merged,
                    "head-a",
                    "CLEAN",
                ))),
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "hook".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                    "merge-queue:main".to_string(),
                    "merge-sync:head-a".to_string(),
                    "view".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_accepts_a_late_async_merge_before_sync_fallback(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let mut client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Merged, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        client.fallback_error = Some(timed_out_merge_error());
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            Ok(())
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42).await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                events.borrow().clone(),
            ),
            (
                Ok(MergeOutcome::Merged(snapshot(
                    PrState::Merged,
                    "head-a",
                    "CLEAN",
                ))),
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "hook".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_does_not_sync_merge_after_the_head_changes(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let mut client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-b", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        client.fallback_error = Some(timed_out_merge_error());
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            Ok(())
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42).await;

        assert_eq!(
            (
                result.map(|_| ()).map_err(|error| error.to_string()),
                events.borrow().clone(),
            ),
            (
                Err(
                    "Cannot fall back to synchronous merge for async merge request request-id"
                        .to_string()
                ),
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "hook".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_does_not_sync_merge_after_the_pr_closes_before_fallback(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let mut client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Closed, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        client.fallback_error = Some(timed_out_merge_error());
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            Ok(())
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42)
            .await
            .map_err(|error| format!("{error:#}"));

        assert_eq!(
            (result, events.borrow().clone()),
            (
                Err("Cannot fall back to synchronous merge for async merge request request-id: PR #42 is not open (state: Closed).".to_string()),
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "hook".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_does_not_sync_merge_for_a_merge_queue_branch(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let mut client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        client.fallback_error = Some(timed_out_merge_error());
        client.merge_queue_enabled = true;
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            Ok(())
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42).await;

        assert_eq!(
            (
                result.map(|_| ()).map_err(|error| error.to_string()),
                events.borrow().clone(),
            ),
            (
                Err("Cannot use synchronous fallback for PR #42 because base branch `main` requires a merge queue. Pending async merge request request-id remains. Retrying waits on the same request and will time out again while it remains pending.".to_string()),
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "hook".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                    "merge-queue:main".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_accepts_a_sync_merge_error_when_the_pr_is_already_merged(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let mut client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Merged, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        client.fallback_error = Some(timed_out_merge_error());
        client.sync_merge_error = true;
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            Ok(())
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42).await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                events.borrow().clone(),
            ),
            (
                Ok(MergeOutcome::Merged(snapshot(
                    PrState::Merged,
                    "head-a",
                    "CLEAN",
                ))),
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "hook".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                    "merge-queue:main".to_string(),
                    "merge-sync:head-a".to_string(),
                    "view".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_keeps_worktree_open_when_sync_merge_fails_and_pr_is_open(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let mut client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        client.fallback_error = Some(timed_out_merge_error());
        client.sync_merge_error = true;
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            Ok(())
        };

        let result = run_merge_and_close(
            &client,
            &mut hook,
            "example-org",
            "example-repo",
            42,
            |_| {
                let close_events = Rc::clone(&events);
                async move {
                    close_events.borrow_mut().push("close".to_string());
                    Ok(())
                }
            },
        )
        .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                events.borrow().clone(),
            ),
            (
                Err("Synchronous merge fallback failed for PR #42 after async merge request request-id".to_string()),
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "hook".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                    "merge-queue:main".to_string(),
                    "merge-sync:head-a".to_string(),
                    "view".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[case::timed_out(
        timed_out_merge_error(),
        "Pending async merge request request-id remains. Retrying waits on the same request and will time out again while it remains pending."
    )]
    #[case::different_pending_head(
        AsyncMergeFallback::HeadMismatch {
            request_uuid: "request-id".to_string(),
            pull_request: 42,
            pending_head_sha: "older-head".to_string(),
            current_head_sha: "head-a".to_string(),
        },
        "Pending async merge request request-id remains. Retrying fails immediately because it targets head SHA older-head, not head-a."
    )]
    #[case::head_unavailable(
        AsyncMergeFallback::HeadUnavailable {
            request_uuid: "request-id".to_string(),
            pull_request: 42,
        },
        "Pending async merge request request-id remains. Retrying fails immediately because GitHub did not report its expected head SHA."
    )]
    #[case::request_uuid_mismatch(
        AsyncMergeFallback::RequestUuidMismatch {
            request_uuid: "request-id".to_string(),
            response_uuid: "different-request".to_string(),
            pull_request: 42,
        },
        "Pending async merge request request-id remains. Retrying fails immediately because GitHub returned a different request UUID."
    )]
    #[tokio::test]
    async fn run_merge_does_not_sync_merge_for_a_stacked_pr(
        events: Rc<RefCell<Vec<String>>>,
        #[case] fallback_error: AsyncMergeFallback,
        #[case] expected_retry_note: &str,
    ) {
        let mut client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        client.fallback_error = Some(fallback_error);
        client.stack = Some(PullRequestStack {
            number: 99,
            position: 1,
            size: 1,
        });
        client.stack_pull_requests = vec![PullRequestStackEntry {
            number: 42,
            merged_at: None,
        }];
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            Ok(())
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42).await;

        assert_eq!(
            (
                result.map(|_| ()).map_err(|error| error.to_string()),
                events.borrow().clone(),
            ),
            (
                Err(format!(
                    "Cannot use synchronous fallback for stacked PR #42. {expected_retry_note}"
                )),
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "stack-pull-requests".to_string(),
                    "hook".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_and_close_skips_worktree_for_a_pr_already_merged_before_retry(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let client = fake_client(
            &[snapshot(PrState::Merged, "head-a", "CLEAN")],
            Rc::clone(&events),
            false,
            false,
        );
        let mut hook = |_: &[(&str, &str)]| Ok(());

        let result = run_merge_and_close(
            &client,
            &mut hook,
            "example-org",
            "example-repo",
            42,
            |_| {
                let close_events = Rc::clone(&events);
                async move {
                    close_events.borrow_mut().push("close".to_string());
                    Ok(())
                }
            },
        )
        .await
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, events.borrow().clone()),
            (Ok(()), vec!["view".to_string()]),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_and_close_closes_the_confirmed_head_and_ignores_close_errors(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Merged, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        let mut hook = |_: &[(&str, &str)]| Ok(());
        let close = |merged_pr: PrSnapshot| {
            let events = Rc::clone(&events);
            async move {
                events.borrow_mut().push(format!(
                    "close:{}:{}",
                    merged_pr.head_ref_name,
                    merged_pr.head_repository.unwrap_or_default(),
                ));
                anyhow::bail!("fake close failed")
            }
        };

        let result =
            run_merge_and_close(&client, &mut hook, "sample-org", "sample-repo", 42, close).await;

        assert_eq!(
            (result.is_ok(), events.borrow().clone()),
            (
                true,
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                    "close:topic/example-branch:sample-org/sample-repo".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[case::open(PrState::Open)]
    #[case::closed(PrState::Closed)]
    #[tokio::test]
    async fn run_merge_and_close_continues_when_post_merge_state_is_not_merged(
        #[case] post_merge_state: PrState,
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(post_merge_state, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        let mut hook = |_: &[(&str, &str)]| Ok(());
        let close = |merged_pr: PrSnapshot| {
            let events = Rc::clone(&events);
            async move {
                events.borrow_mut().push(format!(
                    "close:{}:{}",
                    merged_pr.head_ref_name,
                    merged_pr.head_repository.unwrap_or_default(),
                ));
                Ok(())
            }
        };

        let result =
            run_merge_and_close(&client, &mut hook, "sample-org", "sample-repo", 42, close).await;

        assert_eq!(
            (result.is_ok(), events.borrow().clone()),
            (
                true,
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                    "close:topic/example-branch:sample-org/sample-repo".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_and_close_continues_when_post_merge_state_cannot_be_refreshed(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        let mut hook = |_: &[(&str, &str)]| Ok(());
        let close = |merged_pr: PrSnapshot| {
            let events = Rc::clone(&events);
            async move {
                events.borrow_mut().push(format!(
                    "close:{}:{}",
                    merged_pr.head_ref_name,
                    merged_pr.head_repository.unwrap_or_default(),
                ));
                Ok(())
            }
        };

        let result =
            run_merge_and_close(&client, &mut hook, "sample-org", "sample-repo", 42, close).await;

        assert_eq!(
            (result.is_ok(), events.borrow().clone()),
            (
                true,
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                    "close:topic/example-branch:sample-org/sample-repo".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_and_close_does_not_close_when_merge_stops_before_confirmation(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let client = fake_client(
            &[snapshot(PrState::Open, "head-a", "CLEAN")],
            Rc::clone(&events),
            false,
            false,
        );
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            anyhow::bail!("fake hook failed");
        };
        let close = |_: PrSnapshot| {
            let events = Rc::clone(&events);
            async move {
                events.borrow_mut().push("close".to_string());
                Ok(())
            }
        };

        let result =
            run_merge_and_close(&client, &mut hook, "sample-org", "sample-repo", 42, close).await;

        assert_eq!(
            (result.is_err(), events.borrow().clone()),
            (
                true,
                vec!["view".to_string(), "stack".to_string(), "hook".to_string()],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_stops_when_pre_merge_hook_fails(events: Rc<RefCell<Vec<String>>>) {
        let client = fake_client(
            &[snapshot(PrState::Open, "head-a", "CLEAN")],
            Rc::clone(&events),
            false,
            false,
        );
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            anyhow::bail!("fake hook failed");
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42).await;

        assert_eq!(
            (result.is_err(), events.borrow().clone()),
            (
                true,
                vec!["view".to_string(), "stack".to_string(), "hook".to_string()],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    #[case::closed(PrState::Closed, "CLEAN")]
    #[case::merge_conflicts(PrState::Open, "DIRTY")]
    async fn run_merge_stops_for_unsafe_initial_pr(
        #[case] state: PrState,
        #[case] merge_state_status: &str,
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let client = fake_client(
            &[snapshot(state, "head-a", merge_state_status)],
            Rc::clone(&events),
            false,
            false,
        );
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            Ok(())
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42).await;

        assert_eq!(
            (result.is_err(), events.borrow().clone()),
            (true, vec!["view".to_string()]),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_stops_when_required_checks_fail(events: Rc<RefCell<Vec<String>>>) {
        let client = fake_client(
            &[snapshot(PrState::Open, "head-a", "CLEAN")],
            Rc::clone(&events),
            true,
            false,
        );
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            Ok(())
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42).await;

        assert_eq!(
            (result.is_err(), events.borrow().clone()),
            (
                true,
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "hook".to_string(),
                    "checks".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_rejects_head_changes_after_required_checks(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-b", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        let mut hook = |_: &[(&str, &str)]| Ok(());

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42).await;

        assert_eq!(
            (result.is_err(), events.borrow().clone()),
            (
                true,
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_rejects_a_stack_pr_with_unmerged_lower_prs(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let mut client = fake_client(
            &[snapshot(PrState::Open, "head-a", "CLEAN")],
            Rc::clone(&events),
            false,
            false,
        );
        client.stack = Some(PullRequestStack {
            number: 99,
            position: 2,
            size: 2,
        });
        client.stack_pull_requests = vec![
            PullRequestStackEntry {
                number: 40,
                merged_at: None,
            },
            PullRequestStackEntry {
                number: 41,
                merged_at: Some("2026-01-02T03:04:05Z".to_string()),
            },
            PullRequestStackEntry {
                number: 42,
                merged_at: None,
            },
        ];
        let mut hook = |_: &[(&str, &str)]| {
            events.borrow_mut().push("hook".to_string());
            Ok(())
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42).await;

        assert_eq!(
            (
                result.map(|_| ()).map_err(|error| error.to_string()),
                events.borrow().clone()
            ),
            (
                Err("PR #42 cannot be merged because lower PR #40 is not merged.".to_string()),
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "stack-pull-requests".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_and_close_does_not_close_an_enqueued_pr(events: Rc<RefCell<Vec<String>>>) {
        let mut client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        client.merge_result = AsyncMergeResult::Enqueued {
            message: Some("Waiting for merge queue.".to_string()),
        };
        let mut hook = |_: &[(&str, &str)]| Ok(());
        let close = |_: PrSnapshot| {
            let events = Rc::clone(&events);
            async move {
                events.borrow_mut().push("close".to_string());
                Ok(())
            }
        };

        let result =
            run_merge_and_close(&client, &mut hook, "sample-org", "sample-repo", 42, close).await;

        assert_eq!(
            (result.is_ok(), events.borrow().clone()),
            (
                true,
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_and_close_accepts_async_merge_error_when_pr_is_merged(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Merged, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            true,
        );
        let mut hook = |_: &[(&str, &str)]| Ok(());
        let close = |merged_pr: PrSnapshot| {
            let events = Rc::clone(&events);
            async move {
                events
                    .borrow_mut()
                    .push(format!("close:{}", merged_pr.head_ref_name));
                Ok(())
            }
        };

        let result =
            run_merge_and_close(&client, &mut hook, "sample-org", "sample-repo", 42, close).await;

        assert_eq!(
            (result.is_ok(), events.borrow().clone()),
            (
                true,
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                    "close:topic/example-branch".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_and_close_keeps_worktree_when_merge_fails_and_pr_is_open(
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(PrState::Open, "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            true,
        );
        let mut hook = |_: &[(&str, &str)]| Ok(());
        let close = |_: PrSnapshot| {
            let events = Rc::clone(&events);
            async move {
                events.borrow_mut().push("close".to_string());
                Ok(())
            }
        };

        let result =
            run_merge_and_close(&client, &mut hook, "sample-org", "sample-repo", 42, close).await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                events.borrow().clone()
            ),
            (
                Err("fake merge failed".to_string()),
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge:head-a".to_string(),
                    "view".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    #[case::without_session(false, false, true)]
    #[case::vibe_session(true, true, true)]
    #[case::regular_agent_session(true, false, false)]
    fn ensure_vibe_access_cases(
        #[case] in_agent_session: bool,
        #[case] vibe_enabled: bool,
        #[case] expected: bool,
    ) {
        assert_eq!(
            ensure_vibe_access(in_agent_session, vibe_enabled).is_ok(),
            expected,
        );
    }

    #[rstest]
    #[case::human_ignores_repo_setting(false, "{}", Ok(()))]
    #[case::agent_uses_org_setting(
        true,
        indoc! {"
            orgs:
              example-org:
                vibe: true
        "},
        Ok(())
    )]
    #[case::agent_is_denied_by_default(
        true,
        "{}",
        Err("Vibe mode is disabled by configuration for repository example-org/example-repo.")
    )]
    fn repo_vibe_access_only_gates_agent_sessions(
        #[case] in_agent_session: bool,
        #[case] yaml: &str,
        #[case] expected: std::result::Result<(), &str>,
    ) {
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(
            ensure_repo_vibe_access_for_session(
                in_agent_session,
                || Ok(config),
                "example-org",
                "example-repo",
            )
            .map_err(|error| error.to_string()),
            expected.map_err(str::to_owned),
        );
    }

    #[rstest]
    #[case::no_candidates(vec![], HeadWorktreeSelection::NotFound)]
    #[case::repository_mismatch(
        vec![repository_candidate(
            Some(("other-org", "sample-repo")),
            &[("topic/example-branch", "/tmp/worktree-a")],
        )],
        HeadWorktreeSelection::NotFound,
    )]
    #[case::branch_mismatch(
        vec![repository_candidate(
            Some(("sample-org", "sample-repo")),
            &[("topic/another-branch", "/tmp/worktree-a")],
        )],
        HeadWorktreeSelection::NotFound,
    )]
    #[case::single_match(
        vec![repository_candidate(
            Some(("sample-org", "sample-repo")),
            &[("topic/example-branch", "/tmp/worktree-a")],
        )],
        HeadWorktreeSelection::Unique(PathBuf::from("/tmp/worktree-a")),
    )]
    #[case::repository_name_case_differs(
        vec![repository_candidate(
            Some(("Sample-Org", "Sample-Repo")),
            &[("topic/example-branch", "/tmp/worktree-a")],
        )],
        HeadWorktreeSelection::Unique(PathBuf::from("/tmp/worktree-a")),
    )]
    #[case::ambiguous_matches(
        vec![
            repository_candidate(
                Some(("sample-org", "sample-repo")),
                &[("topic/example-branch", "/tmp/worktree-a")],
            ),
            repository_candidate(
                Some(("sample-org", "sample-repo")),
                &[("topic/example-branch", "/tmp/worktree-b")],
            ),
        ],
        HeadWorktreeSelection::Ambiguous(2),
    )]
    fn select_head_worktree_cases(
        #[case] candidates: Vec<LocalRepositoryWorktrees>,
        #[case] expected: HeadWorktreeSelection,
    ) {
        assert_eq!(
            select_head_worktree(
                candidates,
                "sample-org",
                "sample-repo",
                "topic/example-branch",
            ),
            expected,
        );
    }

    fn repository_candidate(
        repository: Option<(&str, &str)>,
        worktrees: &[(&str, &str)],
    ) -> LocalRepositoryWorktrees {
        LocalRepositoryWorktrees {
            repository: repository.map(|(owner, name)| (owner.to_string(), name.to_string())),
            worktrees: worktrees
                .iter()
                .enumerate()
                .map(|(index, (branch, path))| LinkedWorktree {
                    name: format!("worktree-{index}"),
                    path: PathBuf::from(path),
                    branch: (*branch).to_string(),
                    commit: format!("commit-{index}"),
                })
                .collect(),
        }
    }

    #[test]
    fn existing_worktrees_skips_paths_that_are_not_local() {
        let root = tempfile::tempdir().unwrap();
        let existing_path = root.path().join("worktree");
        std::fs::create_dir_all(&existing_path).unwrap();

        let result = existing_worktrees(vec![
            LinkedWorktree {
                name: "exact".to_string(),
                path: existing_path.clone(),
                branch: "topic/example-branch".to_string(),
                commit: "commit-a".to_string(),
            },
            LinkedWorktree {
                name: "missing".to_string(),
                path: root.path().join("missing"),
                branch: "topic/example-branch".to_string(),
                commit: "commit-c".to_string(),
            },
        ]);

        assert_eq!(
            result
                .into_iter()
                .map(|worktree| (worktree.branch, worktree.path))
                .collect::<Vec<_>>(),
            vec![("topic/example-branch".to_string(), existing_path)],
        );
    }

    #[rstest]
    #[case::open("OPEN", Ok(PrState::Open))]
    #[case::closed("CLOSED", Ok(PrState::Closed))]
    #[case::merged("MERGED", Ok(PrState::Merged))]
    #[case::unknown("QUEUED", Err("Unknown PR state returned by gh: QUEUED"))]
    fn parse_pr_state_cases(#[case] state: &str, #[case] expected: Result<PrState, &str>) {
        assert_eq!(
            parse_pr_state(state).map_err(|error| error.to_string()),
            expected.map_err(str::to_string),
        );
    }

    #[rstest]
    #[tokio::test]
    #[case::closed(PrState::Closed, "CLEAN")]
    #[case::merge_conflicts(PrState::Open, "DIRTY")]
    async fn run_merge_stops_if_pr_becomes_unsafe_after_checks(
        #[case] state: PrState,
        #[case] merge_state_status: &str,
        events: Rc<RefCell<Vec<String>>>,
    ) {
        let client = fake_client(
            &[
                snapshot(PrState::Open, "head-a", "CLEAN"),
                snapshot(state, "head-a", merge_state_status),
            ],
            Rc::clone(&events),
            false,
            false,
        );
        let mut hook = |_: &[(&str, &str)]| Ok(());

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42).await;

        assert_eq!(
            (result.is_err(), events.borrow().clone()),
            (
                true,
                vec![
                    "view".to_string(),
                    "stack".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                ],
            ),
        );
    }
}
