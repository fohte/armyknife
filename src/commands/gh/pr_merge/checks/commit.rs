use std::{
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use serde::Deserialize;

use super::{
    CHECK_POLL_INTERVAL, CheckBucket, CheckWaitStatus, GhCheck, RequiredStatusCheckContext,
    command_output_text, format_check_details, is_failed_check, required_status_check_contexts,
    run_gh, wait_until_complete,
};

const COMMIT_CHECK_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(180);

pub(super) fn wait_for_base_branch_checks(
    owner: &str,
    repo: &str,
    branch: &str,
    allow_failures: bool,
) -> anyhow::Result<()> {
    let required_contexts = fetch_required_status_check_contexts(owner, repo, branch)?;
    if required_contexts.is_empty() {
        return Ok(());
    }

    let sha = fetch_branch_tip_sha(owner, repo, branch)?;
    println!("Checking required checks on base branch `{branch}` ({sha})...");
    wait_for_commit_checks(
        owner,
        repo,
        branch,
        &sha,
        &required_contexts,
        allow_failures,
    )
}

pub(super) fn wait_for_merge_commit_checks(
    owner: &str,
    repo: &str,
    branch: &str,
    merge_sha: Option<&str>,
) -> anyhow::Result<()> {
    let required_contexts = fetch_required_status_check_contexts(owner, repo, branch)?;
    if required_contexts.is_empty() {
        return Ok(());
    }

    let merge_sha = merge_sha.context("GitHub did not report the confirmed merge commit SHA")?;
    println!("Waiting for required checks on merge commit {merge_sha}...");
    wait_for_commit_checks(owner, repo, branch, merge_sha, &required_contexts, false)
}

fn wait_for_commit_checks(
    owner: &str,
    repo: &str,
    branch: &str,
    sha: &str,
    required_contexts: &[RequiredStatusCheckContext],
    allow_failures: bool,
) -> anyhow::Result<()> {
    wait_for_commit_checks_with(
        required_contexts,
        allow_failures,
        || fetch_commit_checks(owner, repo, sha),
        |details| {
            println!(
                "Required checks on `{branch}` are pending: {details}; retrying in 10 seconds..."
            );
            thread::sleep(CHECK_POLL_INTERVAL);
        },
        Instant::now,
        COMMIT_CHECK_DISCOVERY_TIMEOUT,
    )
}

fn wait_for_commit_checks_with(
    required_contexts: &[RequiredStatusCheckContext],
    allow_failures: bool,
    mut fetch_checks: impl FnMut() -> anyhow::Result<Vec<GhCheck>>,
    wait: impl FnMut(&str),
    mut now: impl FnMut() -> Instant,
    discovery_timeout: Duration,
) -> anyhow::Result<()> {
    let discovery_deadline = now() + discovery_timeout;
    wait_until_complete(
        || {
            let checks = fetch_checks()?;
            match classify_required_commit_checks(&checks, required_contexts, allow_failures)? {
                CommitCheckStatus::Complete => Ok(CheckWaitStatus::Complete),
                CommitCheckStatus::Pending(details) => Ok(CheckWaitStatus::Pending(details)),
                CommitCheckStatus::Unreported if now() < discovery_deadline => {
                    Ok(CheckWaitStatus::Pending(
                        "No required checks have been reported yet".to_string(),
                    ))
                }
                CommitCheckStatus::Unreported => Ok(CheckWaitStatus::Complete),
            }
        },
        wait,
    )
}

fn classify_required_commit_checks(
    checks: &[GhCheck],
    required_contexts: &[RequiredStatusCheckContext],
    allow_failures: bool,
) -> anyhow::Result<CommitCheckStatus> {
    let required_checks = checks
        .iter()
        .filter(|check| {
            required_contexts.iter().any(|required| {
                required.context == check.name
                    && required
                        .integration_id
                        .is_none_or(|integration_id| check.app_id == Some(integration_id))
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    if required_checks.is_empty() {
        return Ok(CommitCheckStatus::Unreported);
    }

    let mut failed = Vec::new();
    let mut pending = Vec::new();
    for check in required_checks {
        if is_failed_check(&check) {
            failed.push(check);
        } else if check.bucket == CheckBucket::Pending {
            pending.push(check);
        }
    }
    if !failed.is_empty() && !allow_failures {
        bail!("Required checks failed: {}", format_check_details(&failed));
    }
    if !pending.is_empty() {
        return Ok(CommitCheckStatus::Pending(format_check_details(&pending)));
    }
    Ok(CommitCheckStatus::Complete)
}

#[derive(Debug, PartialEq, Eq)]
enum CommitCheckStatus {
    Complete,
    Pending(String),
    Unreported,
}

fn fetch_required_status_check_contexts(
    owner: &str,
    repo: &str,
    branch: &str,
) -> anyhow::Result<Vec<RequiredStatusCheckContext>> {
    let rules = super::fetch_required_status_check_rules(owner, repo, branch)?;
    Ok(required_status_check_contexts(&rules))
}

fn fetch_branch_tip_sha(owner: &str, repo: &str, branch: &str) -> anyhow::Result<String> {
    let encoded_branch =
        percent_encoding::utf8_percent_encode(branch, percent_encoding::NON_ALPHANUMERIC);
    let args = [
        "api".to_string(),
        format!("repos/{owner}/{repo}/git/ref/heads/{encoded_branch}"),
        "--jq".to_string(),
        ".object.sha".to_string(),
    ];
    let output = run_gh(&args).context("Failed to get base branch tip SHA")?;
    if !output.status.success() {
        bail!(
            "gh api failed to get base branch tip: {}",
            command_output_text(&output)
        );
    }
    String::from_utf8(output.stdout)
        .context("GitHub returned a non-UTF-8 base branch SHA")
        .map(|sha| sha.trim().to_string())
}

fn fetch_commit_checks(owner: &str, repo: &str, sha: &str) -> anyhow::Result<Vec<GhCheck>> {
    let check_run_args = [
        "api".to_string(),
        format!("repos/{owner}/{repo}/commits/{sha}/check-runs"),
        "--paginate".to_string(),
        "--slurp".to_string(),
        "-F".to_string(),
        "per_page=100".to_string(),
        "-F".to_string(),
        "filter=latest".to_string(),
    ];
    let output = run_gh(&check_run_args).context("Failed to inspect commit check runs")?;
    if !output.status.success() {
        bail!(
            "gh api failed to inspect commit check runs: {}",
            command_output_text(&output)
        );
    }
    let pages = serde_json::from_slice::<Vec<CheckRunPage>>(&output.stdout)
        .context("Failed to parse commit check runs")?;
    let mut checks = pages
        .into_iter()
        .flat_map(|page| page.check_runs)
        .map(GhCheck::from)
        .collect::<Vec<_>>();

    let status_args = [
        "api".to_string(),
        format!("repos/{owner}/{repo}/commits/{sha}/status"),
    ];
    let output = run_gh(&status_args).context("Failed to inspect commit statuses")?;
    if !output.status.success() {
        bail!(
            "gh api failed to inspect commit statuses: {}",
            command_output_text(&output)
        );
    }
    let status = serde_json::from_slice::<CombinedCommitStatus>(&output.stdout)
        .context("Failed to parse commit statuses")?;
    checks.extend(status.statuses.into_iter().map(GhCheck::from));
    Ok(checks)
}

#[derive(Deserialize)]
struct CheckRunPage {
    check_runs: Vec<CheckRun>,
}

#[derive(Deserialize)]
struct CheckRun {
    name: String,
    status: String,
    conclusion: Option<String>,
    html_url: String,
    app: Option<CheckRunApp>,
}

#[derive(Deserialize)]
struct CheckRunApp {
    id: u64,
}

impl From<CheckRun> for GhCheck {
    fn from(check_run: CheckRun) -> Self {
        let state = check_run
            .conclusion
            .map(|conclusion| conclusion.to_uppercase())
            .unwrap_or_else(|| check_run.status.to_uppercase());
        let bucket = match state.as_str() {
            "SUCCESS" | "NEUTRAL" | "SKIPPED" => CheckBucket::Pass,
            "QUEUED" | "IN_PROGRESS" | "REQUESTED" | "WAITING" | "PENDING" => CheckBucket::Pending,
            _ => CheckBucket::Fail,
        };
        GhCheck {
            name: check_run.name,
            state,
            bucket,
            link: check_run.html_url,
            app_id: check_run.app.map(|app| app.id),
        }
    }
}

#[derive(Deserialize)]
struct CombinedCommitStatus {
    statuses: Vec<CommitStatus>,
}

#[derive(Deserialize)]
struct CommitStatus {
    context: String,
    state: String,
    target_url: Option<String>,
}

impl From<CommitStatus> for GhCheck {
    fn from(status: CommitStatus) -> Self {
        let state = status.state.to_uppercase();
        let bucket = match state.as_str() {
            "SUCCESS" => CheckBucket::Pass,
            "PENDING" => CheckBucket::Pending,
            _ => CheckBucket::Fail,
        };
        GhCheck {
            name: status.context,
            state,
            bucket,
            link: status.target_url.unwrap_or_default(),
            app_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, collections::VecDeque};

    use rstest::rstest;

    use super::*;

    fn required(context: &str, integration_id: Option<u64>) -> RequiredStatusCheckContext {
        RequiredStatusCheckContext {
            context: context.to_string(),
            integration_id,
        }
    }

    fn gh_check(name: &str, state: &str, bucket: CheckBucket, link: &str) -> GhCheck {
        GhCheck {
            name: name.to_string(),
            state: state.to_string(),
            bucket,
            link: link.to_string(),
            app_id: None,
        }
    }

    #[rstest]
    #[case::check_run_success_is_normalized(
        r#"{"name":"unit","status":"completed","conclusion":"success","html_url":"https://example.invalid/checks/unit","app":{"id":42}}"#,
        GhCheck {
            name: "unit".to_string(),
            state: "SUCCESS".to_string(),
            bucket: CheckBucket::Pass,
            link: "https://example.invalid/checks/unit".to_string(),
            app_id: Some(42),
        }
    )]
    #[case::check_run_pending(
        r#"{"name":"unit","status":"in_progress","conclusion":null,"html_url":"https://example.invalid/checks/unit","app":{"id":42}}"#,
        GhCheck {
            name: "unit".to_string(),
            state: "IN_PROGRESS".to_string(),
            bucket: CheckBucket::Pending,
            link: "https://example.invalid/checks/unit".to_string(),
            app_id: Some(42),
        }
    )]
    #[case::check_run_failure(
        r#"{"name":"unit","status":"completed","conclusion":"failure","html_url":"https://example.invalid/checks/unit","app":{"id":42}}"#,
        GhCheck {
            name: "unit".to_string(),
            state: "FAILURE".to_string(),
            bucket: CheckBucket::Fail,
            link: "https://example.invalid/checks/unit".to_string(),
            app_id: Some(42),
        }
    )]
    fn check_run_payloads_map_to_full_check_values(
        #[case] response: &str,
        #[case] expected: GhCheck,
    ) {
        let check_run = serde_json::from_str::<CheckRun>(response).unwrap();

        assert_eq!(GhCheck::from(check_run), expected);
    }

    #[test]
    fn commit_status_payload_maps_state_and_optional_url() {
        let status = serde_json::from_str::<CommitStatus>(
            r#"{"context":"unit","state":"error","target_url":null}"#,
        )
        .unwrap();

        assert_eq!(
            GhCheck::from(status),
            GhCheck {
                name: "unit".to_string(),
                state: "ERROR".to_string(),
                bucket: CheckBucket::Fail,
                link: String::new(),
                app_id: None,
            },
        );
    }

    #[test]
    fn app_scoped_required_check_ignores_other_apps_with_the_same_context() {
        let checks = [
            GhCheck {
                name: "unit".to_string(),
                state: "FAILURE".to_string(),
                bucket: CheckBucket::Fail,
                link: "https://example.invalid/checks/wrong-app".to_string(),
                app_id: Some(7),
            },
            GhCheck {
                name: "unit".to_string(),
                state: "SUCCESS".to_string(),
                bucket: CheckBucket::Pass,
                link: "https://example.invalid/checks/required-app".to_string(),
                app_id: Some(42),
            },
        ];

        assert_eq!(
            classify_required_commit_checks(&checks, &[required("unit", Some(42))], false).unwrap(),
            CommitCheckStatus::Complete,
        );
    }

    #[rstest]
    #[case::unreported_required_check_is_ignored(
        vec![],
        vec![required("unit", None)],
        false,
        Ok(CommitCheckStatus::Unreported)
    )]
    #[case::unrelated_failed_check_is_ignored(
        vec![gh_check("release", "FAILURE", CheckBucket::Fail, "https://example.invalid/release")],
        vec![required("unit", None)],
        false,
        Ok(CommitCheckStatus::Unreported)
    )]
    #[case::reported_required_check_waits(
        vec![gh_check("unit", "IN_PROGRESS", CheckBucket::Pending, "https://example.invalid/unit")],
        vec![required("unit", None)],
        false,
        Ok(CommitCheckStatus::Pending(
            "unit (IN_PROGRESS) https://example.invalid/unit".to_string()
        ))
    )]
    #[case::failed_required_check_names_its_url(
        vec![gh_check("unit", "FAILURE", CheckBucket::Fail, "https://example.invalid/unit")],
        vec![required("unit", None)],
        false,
        Err("Required checks failed: unit (FAILURE) https://example.invalid/unit".to_string())
    )]
    #[case::base_failure_can_be_allowed(
        vec![gh_check("unit", "FAILURE", CheckBucket::Fail, "https://example.invalid/unit")],
        vec![required("unit", None)],
        true,
        Ok(CommitCheckStatus::Complete)
    )]
    #[case::allowed_base_failure_still_waits_for_pending_checks(
        vec![
            gh_check("unit", "FAILURE", CheckBucket::Fail, "https://example.invalid/unit"),
            gh_check("integration", "IN_PROGRESS", CheckBucket::Pending, "https://example.invalid/integration"),
        ],
        vec![required("unit", None), required("integration", None)],
        true,
        Ok(CommitCheckStatus::Pending(
            "integration (IN_PROGRESS) https://example.invalid/integration".to_string()
        ))
    )]
    fn required_commit_check_classification_cases(
        #[case] checks: Vec<GhCheck>,
        #[case] required_contexts: Vec<RequiredStatusCheckContext>,
        #[case] allow_failures: bool,
        #[case] expected: Result<CommitCheckStatus, String>,
    ) {
        let result = classify_required_commit_checks(&checks, &required_contexts, allow_failures)
            .map_err(|error| error.to_string());

        assert_eq!(result, expected);
    }

    struct FakeCommitCheckClient {
        responses: VecDeque<Vec<GhCheck>>,
        request_count: usize,
    }

    impl FakeCommitCheckClient {
        fn fetch(&mut self) -> anyhow::Result<Vec<GhCheck>> {
            self.request_count += 1;
            self.responses
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("No fake check response remains"))
        }
    }

    #[test]
    fn waits_for_a_required_check_that_appears_after_an_empty_response() {
        let mut client = FakeCommitCheckClient {
            responses: VecDeque::from([
                vec![],
                vec![gh_check(
                    "unit",
                    "FAILURE",
                    CheckBucket::Fail,
                    "https://example.invalid/checks/unit",
                )],
            ]),
            request_count: 0,
        };
        let started_at = Instant::now();
        let elapsed = Cell::new(Duration::ZERO);
        let mut waits = 0;

        let result = wait_for_commit_checks_with(
            &[required("unit", None)],
            false,
            || client.fetch(),
            |_| {
                waits += 1;
                elapsed.set(elapsed.get() + CHECK_POLL_INTERVAL);
            },
            || started_at + elapsed.get(),
            COMMIT_CHECK_DISCOVERY_TIMEOUT,
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, client.request_count, waits),
            (
                Err(
                    "Required checks failed: unit (FAILURE) https://example.invalid/checks/unit"
                        .to_string()
                ),
                2,
                1,
            ),
        );
    }

    #[test]
    fn stops_waiting_when_no_required_check_is_reported_before_the_deadline() {
        let mut client = FakeCommitCheckClient {
            responses: VecDeque::from([vec![], vec![]]),
            request_count: 0,
        };
        let started_at = Instant::now();
        let elapsed = Cell::new(Duration::ZERO);
        let discovery_timeout = Duration::from_secs(30);
        let mut waits = 0;

        let result = wait_for_commit_checks_with(
            &[required("pr-title-check", None)],
            false,
            || client.fetch(),
            |_| {
                waits += 1;
                elapsed.set(elapsed.get() + discovery_timeout);
            },
            || started_at + elapsed.get(),
            discovery_timeout,
        )
        .map_err(|error| error.to_string());

        assert_eq!((result, client.request_count, waits), (Ok(()), 2, 1));
    }
}
