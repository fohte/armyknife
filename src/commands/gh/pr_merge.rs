use std::process::Output;
use std::time::Duration;

use anyhow::{Context, bail};
use clap::Args;

use crate::commands::agent::is_vibe_enabled;
use crate::infra::{external_tool::ExternalTool, git, process};
use crate::shared::{env_var::EnvVars, hooks};

const REQUIRED_CHECKS_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Args, Clone, PartialEq, Eq)]
pub struct PrMergeArgs {
    /// PR number
    pub pr_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,
}

pub fn run(args: &PrMergeArgs) -> anyhow::Result<()> {
    let in_agent_session = EnvVars::load().own_session_id().is_some();
    let vibe_enabled = in_agent_session && is_vibe_enabled()?;
    ensure_vibe_access(in_agent_session, vibe_enabled)?;

    let (owner, repo) = git::get_repo_owner_and_name(args.repo.as_deref())?;
    let client = GhCli;
    run_merge(
        &client,
        &mut |env| hooks::run_hook("pre-pr-merge", env),
        &owner,
        &repo,
        args.pr_number,
    )?;
    println!("PR #{} merged.", args.pr_number);
    Ok(())
}

fn ensure_vibe_access(in_agent_session: bool, vibe_enabled: bool) -> anyhow::Result<()> {
    if in_agent_session && !vibe_enabled {
        bail!("PR merge is only allowed in vibe mode inside an agent session.");
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
    fn squash_merge(&self, owner: &str, repo: &str, pr_number: u64) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrSnapshot {
    state: String,
    head_sha: String,
    merge_state_status: String,
    url: String,
}

fn run_merge(
    client: &impl PrMergeClient,
    hook_runner: &mut impl FnMut(&[(&str, &str)]) -> anyhow::Result<()>,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> anyhow::Result<()> {
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

    if let Err(merge_error) = client.squash_merge(owner, repo, pr_number) {
        if client
            .view_pr(owner, repo, pr_number)
            .is_ok_and(|pr| pr.state == "MERGED")
        {
            return Ok(());
        }
        return Err(merge_error);
    }

    let merged = client.view_pr(owner, repo, pr_number)?;
    if merged.state != "MERGED" {
        bail!(
            "The squash merge command succeeded, but PR #{pr_number} is not merged (state: {}).",
            merged.state
        );
    }

    Ok(())
}

fn ensure_pr_is_open_and_safe(
    pr: &PrSnapshot,
    pr_number: u64,
    expected_head_sha: Option<&str>,
) -> anyhow::Result<()> {
    if pr.state != "OPEN" {
        bail!("PR #{pr_number} is not open (state: {}).", pr.state);
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
            "state,headRefOid,mergeStateStatus,url",
        ];
        let output = run_gh(&args)?;
        let response: GhPrResponse = serde_json::from_slice(&output.stdout)
            .context("Failed to parse gh pr view response")?;
        Ok(PrSnapshot {
            state: response.state,
            head_sha: response.head_ref_oid,
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
        let output = match process::run_with_timeout(command, REQUIRED_CHECKS_TIMEOUT) {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                bail!("Timed out after 300 seconds waiting for required checks.");
            }
            Err(error) => return Err(error).context("Failed to wait for required checks"),
        };

        let output_text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if output.status.success() || no_required_checks_reported(&output_text) {
            return Ok(());
        }

        bail!("Required checks failed: {}", output_text.trim());
    }

    fn squash_merge(&self, owner: &str, repo: &str, pr_number: u64) -> anyhow::Result<()> {
        let pr_number = pr_number.to_string();
        let repository = format!("{owner}/{repo}");
        let args = [
            "pr",
            "merge",
            pr_number.as_str(),
            "--repo",
            repository.as_str(),
            "--squash",
        ];
        run_gh(&args).map(|_| ())
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPrResponse {
    state: String,
    head_ref_oid: String,
    merge_state_status: Option<String>,
    url: String,
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

fn no_required_checks_reported(output: &str) -> bool {
    output.contains("no required checks reported") || output.contains("no checks reported on the")
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque, rc::Rc};

    use rstest::{fixture, rstest};

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

        fn squash_merge(&self, _owner: &str, _repo: &str, _pr_number: u64) -> anyhow::Result<()> {
            self.events.borrow_mut().push("merge".to_string());
            if self.merge_error {
                anyhow::bail!("fake merge failed");
            }
            Ok(())
        }
    }

    fn snapshot(state: &str, head_sha: &str, merge_state_status: &str) -> PrSnapshot {
        PrSnapshot {
            state: state.to_string(),
            head_sha: head_sha.to_string(),
            merge_state_status: merge_state_status.to_string(),
            url: "https://github.com/example-org/example-repo/pull/42".to_string(),
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
                snapshot("OPEN", "head-a", "CLEAN"),
                snapshot("OPEN", "head-a", "CLEAN"),
                snapshot("MERGED", "head-a", "CLEAN"),
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
                    "hook:[(\"ARMYKNIFE_PR_NUMBER\", \"42\"), (\"ARMYKNIFE_PR_OWNER\", \"example-org\"), (\"ARMYKNIFE_PR_REPO\", \"example-repo\"), (\"ARMYKNIFE_PR_URL\", \"https://github.com/example-org/example-repo/pull/42\"), (\"ARMYKNIFE_PR_HEAD_SHA\", \"head-a\")]".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge".to_string(),
                    "view".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    fn run_merge_stops_when_pre_merge_hook_fails(events: Rc<RefCell<Vec<String>>>) {
        let client = fake_client(
            &[snapshot("OPEN", "head-a", "CLEAN")],
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
    #[case::closed("CLOSED", "CLEAN")]
    #[case::merge_conflicts("OPEN", "DIRTY")]
    fn run_merge_stops_for_unsafe_initial_pr(
        #[case] state: &str,
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
            &[snapshot("OPEN", "head-a", "CLEAN")],
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
                snapshot("OPEN", "head-a", "CLEAN"),
                snapshot("OPEN", "head-b", "CLEAN"),
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
    fn run_merge_accepts_merge_error_when_pr_is_already_merged(events: Rc<RefCell<Vec<String>>>) {
        let client = fake_client(
            &[
                snapshot("OPEN", "head-a", "CLEAN"),
                snapshot("OPEN", "head-a", "CLEAN"),
                snapshot("MERGED", "head-a", "CLEAN"),
            ],
            Rc::clone(&events),
            false,
            true,
        );
        let mut hook = |_: &[(&str, &str)]| Ok(());

        let result = run_merge(&client, &mut hook, "example-org", "example-repo", 42);

        assert_eq!(
            (result.is_ok(), events.borrow().clone()),
            (
                true,
                vec![
                    "view".to_string(),
                    "checks".to_string(),
                    "view".to_string(),
                    "merge".to_string(),
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
    #[case::required_checks_absent("no required checks reported", true)]
    #[case::checks_absent("no checks reported on the head commit", true)]
    #[case::checks_present("Required checks are pending", false)]
    fn no_required_checks_reported_cases(#[case] output: &str, #[case] expected: bool) {
        assert_eq!(no_required_checks_reported(output), expected);
    }
}
