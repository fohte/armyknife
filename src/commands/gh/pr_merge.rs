use std::{future::Future, io::Write, path::PathBuf, process::Output, time::Duration};

use anyhow::{Context, bail};
use clap::Args;

use crate::commands::agent::{close_worktree_path, ensure_repo_vibe_access, is_vibe_enabled};
use crate::infra::{
    external_tool::ExternalTool,
    git::{self, GitRepo, github_owner_and_repo},
    github::PrState,
    process,
};
use crate::shared::{
    config::{Config, load_config},
    env_var::EnvVars,
    hooks,
    repos_root::{discover_repos_with_worktrees, resolve_repos_root},
    worktree::{LinkedWorktree, list_linked_worktrees},
};

const REQUIRED_CHECKS_TIMEOUT: Duration = Duration::from_secs(300);

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
    fn wait_for_required_checks(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> anyhow::Result<()>;
    fn squash_merge(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        head_sha: &str,
    ) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrSnapshot {
    state: PrState,
    head_sha: String,
    head_ref_name: String,
    head_repository: Option<String>,
    merge_state_status: String,
    url: String,
}

fn run_merge(
    client: &impl PrMergeClient,
    hook_runner: &mut impl FnMut(&[(&str, &str)]) -> anyhow::Result<()>,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> anyhow::Result<PrSnapshot> {
    let initial = client.view_pr(owner, repo, pr_number)?;
    ensure_pr_is_open_and_safe(&initial, pr_number, None)?;

    let pr_number_string = pr_number.to_string();
    let hook_env = [
        (EnvVars::pr_number_name(), pr_number_string.as_str()),
        (EnvVars::pr_owner_name(), owner),
        (EnvVars::pr_repo_name(), repo),
        (EnvVars::pr_url_name(), initial.url.as_str()),
        (EnvVars::pr_head_sha_name(), initial.head_sha.as_str()),
    ];
    hook_runner(&hook_env)?;

    client.wait_for_required_checks(owner, repo, pr_number)?;

    let current = client.view_pr(owner, repo, pr_number)?;
    ensure_pr_is_open_and_safe(&current, pr_number, Some(&initial.head_sha))?;

    if let Err(merge_error) = client.squash_merge(owner, repo, pr_number, &initial.head_sha) {
        if let Ok(merged) = client.view_pr(owner, repo, pr_number)
            && merged.state == PrState::Merged
        {
            return Ok(merged);
        }
        return Err(merge_error);
    }

    let merged = client.view_pr(owner, repo, pr_number)?;
    if merged.state != PrState::Merged {
        bail!(
            "The squash merge command succeeded, but PR #{pr_number} is not merged (state: {:?}).",
            merged.state,
        );
    }

    Ok(merged)
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
    let merged = run_merge(client, hook_runner, owner, repo, pr_number)?;
    println!("PR #{pr_number} merged.");
    let _ = std::io::stdout().flush();
    if let Err(error) = close_worktree(merged.clone()).await {
        eprintln!(
            "Could not close worktree for merged PR #{pr_number} (branch `{}`): {error:#}",
            merged.head_ref_name,
        );
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
            "state,headRefOid,headRefName,headRepository,mergeStateStatus,url",
        ];
        let output = run_gh(&args)?;
        let response: GhPrResponse = serde_json::from_slice(&output.stdout)
            .context("Failed to parse gh pr view response")?;
        Ok(PrSnapshot {
            state: parse_pr_state(&response.state)?,
            head_sha: response.head_ref_oid,
            head_ref_name: response.head_ref_name,
            head_repository: response
                .head_repository
                .map(|repository| repository.name_with_owner),
            merge_state_status: response.merge_state_status.unwrap_or_default(),
            url: response.url,
        })
    }

    fn wait_for_required_checks(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> anyhow::Result<()> {
        let repository = format!("{owner}/{repo}");
        let args = [
            "pr".to_string(),
            "checks".to_string(),
            pr_number.to_string(),
            "--repo".to_string(),
            repository,
            "--required".to_string(),
            "--watch".to_string(),
            "--fail-fast".to_string(),
        ];
        let mut command = ExternalTool::Gh.command();
        command.args(&args);
        println!(
            "Waiting for required checks (up to {} seconds)...",
            REQUIRED_CHECKS_TIMEOUT.as_secs()
        );
        let output = match process::run_with_timeout(command, REQUIRED_CHECKS_TIMEOUT) {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                bail!(
                    "Timed out after {} seconds waiting for required checks.",
                    REQUIRED_CHECKS_TIMEOUT.as_secs()
                );
            }
            Err(error) => return Err(error).context("Failed to wait for required checks"),
        };

        let output_text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if required_checks_succeeded(output.status.success(), &output_text) {
            return Ok(());
        }

        bail!("Required checks failed: {}", output_text.trim());
    }

    fn squash_merge(
        &self,
        owner: &str,
        repo: &str,
        pr_number: u64,
        head_sha: &str,
    ) -> anyhow::Result<()> {
        let pr_number = pr_number.to_string();
        let repository = format!("{owner}/{repo}");
        let args = [
            "pr",
            "merge",
            pr_number.as_str(),
            "--repo",
            repository.as_str(),
            "--squash",
            "--match-head-commit",
            head_sha,
        ];
        run_gh(&args).map(|_| ())
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPrResponse {
    state: String,
    head_ref_oid: String,
    head_ref_name: String,
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

fn required_checks_succeeded(status_success: bool, output: &str) -> bool {
    status_success
        || output.contains("no required checks reported")
        || output.contains("no checks reported on the")
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    use indoc::indoc;
    use rstest::{fixture, rstest};

    use crate::shared::config::Config;

    use super::*;

    struct FakePrMergeClient {
        snapshots: RefCell<VecDeque<PrSnapshot>>,
        events: Rc<RefCell<Vec<String>>>,
        checks_error: bool,
        merge_error: bool,
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

        fn squash_merge(
            &self,
            _owner: &str,
            _repo: &str,
            _pr_number: u64,
            head_sha: &str,
        ) -> anyhow::Result<()> {
            self.events.borrow_mut().push(format!("merge:{head_sha}"));
            if self.merge_error {
                anyhow::bail!("fake merge failed");
            }
            Ok(())
        }
    }

    fn snapshot(state: PrState, head_sha: &str, merge_state_status: &str) -> PrSnapshot {
        PrSnapshot {
            state,
            head_sha: head_sha.to_string(),
            head_ref_name: "topic/example-branch".to_string(),
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
        }
    }

    #[rstest]
    fn run_merge_calls_hook_and_checks_before_squash_merge(events: Rc<RefCell<Vec<String>>>) {
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
        let mut hook = |env: &[(&str, &str)]| {
            events.borrow_mut().push(format!("hook:{env:?}"));
            Ok(())
        };

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42);

        assert_eq!(
            (result.is_ok(), events.borrow().clone()),
            (
                true,
                vec![
                    "view".to_string(),
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
            (true, vec!["view".to_string(), "hook".to_string()]),
        );
    }

    #[rstest]
    fn run_merge_stops_when_pre_merge_hook_fails(events: Rc<RefCell<Vec<String>>>) {
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

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42);

        assert_eq!(
            (result.is_err(), events.borrow().clone()),
            (true, vec!["view".to_string(), "hook".to_string()]),
        );
    }

    #[rstest]
    #[case::closed(PrState::Closed, "CLEAN")]
    #[case::merge_conflicts(PrState::Open, "DIRTY")]
    fn run_merge_stops_for_unsafe_initial_pr(
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

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42);

        assert_eq!(
            (result.is_err(), events.borrow().clone()),
            (true, vec!["view".to_string()]),
        );
    }

    #[rstest]
    fn run_merge_stops_when_required_checks_fail(events: Rc<RefCell<Vec<String>>>) {
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

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42);

        assert_eq!(
            (result.is_err(), events.borrow().clone()),
            (
                true,
                vec!["view".to_string(), "hook".to_string(), "checks".to_string()],
            ),
        );
    }

    #[rstest]
    fn run_merge_rejects_head_changes_after_required_checks(events: Rc<RefCell<Vec<String>>>) {
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

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42);

        assert_eq!(
            (result.is_err(), events.borrow().clone()),
            (
                true,
                vec!["view".to_string(), "checks".to_string(), "view".to_string()],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn run_merge_and_close_accepts_merge_error_when_pr_is_already_merged(
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
    #[case::checks_succeeded(true, "", true)]
    #[case::required_checks_absent(false, "no required checks reported", true)]
    #[case::checks_absent(false, "no checks reported on the head commit", true)]
    #[case::checks_failed(false, "Required checks are pending", false)]
    fn required_checks_succeeded_cases(
        #[case] status_success: bool,
        #[case] output: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(required_checks_succeeded(status_success, output), expected);
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
    #[case::closed(PrState::Closed, "CLEAN")]
    #[case::merge_conflicts(PrState::Open, "DIRTY")]
    fn run_merge_stops_if_pr_becomes_unsafe_after_checks(
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

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42);

        assert_eq!(
            (result.is_err(), events.borrow().clone()),
            (
                true,
                vec!["view".to_string(), "checks".to_string(), "view".to_string()],
            ),
        );
    }
}
