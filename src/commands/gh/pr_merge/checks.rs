use std::{thread, time::Duration};

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::infra::{external_tool::ExternalTool, process};

const GH_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const CHECK_POLL_INTERVAL: Duration = Duration::from_secs(10);

pub(super) fn wait_for_required_checks(
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> anyhow::Result<()> {
    println!("Waiting for required checks...");
    let mut required_checks_configured = None;
    wait_until_complete(
        || query_required_check_status(owner, repo, pr_number, &mut required_checks_configured),
        || {
            println!("Required checks are still pending; retrying in 10 seconds...");
            thread::sleep(CHECK_POLL_INTERVAL);
        },
    )
}

fn wait_until_complete(
    mut poll: impl FnMut() -> anyhow::Result<CheckWaitStatus>,
    mut wait: impl FnMut(),
) -> anyhow::Result<()> {
    loop {
        match poll()? {
            CheckWaitStatus::Complete => return Ok(()),
            CheckWaitStatus::Pending => wait(),
        }
    }
}

fn query_required_check_status(
    owner: &str,
    repo: &str,
    pr_number: u64,
    required_checks_configured: &mut Option<bool>,
) -> anyhow::Result<CheckWaitStatus> {
    let repository = format!("{owner}/{repo}");
    let args = [
        "pr".to_string(),
        "checks".to_string(),
        pr_number.to_string(),
        "--repo".to_string(),
        repository,
        "--required".to_string(),
        "--json".to_string(),
        "bucket".to_string(),
    ];
    // With --json, gh skips status exit codes; no required checks fail before JSON output.
    let output = run_gh(&args).context("Failed to query required checks")?;

    match classify_check_result(output.status.success(), &output.stdout)? {
        Some(CheckOutcome::Complete) => Ok(CheckWaitStatus::Complete),
        Some(CheckOutcome::Pending) => Ok(CheckWaitStatus::Pending),
        Some(CheckOutcome::Failed) => {
            bail!("Required checks failed: {}", command_output_text(&output));
        }
        None => {
            let has_required_checks = match *required_checks_configured {
                Some(has_required_checks) => has_required_checks,
                None => {
                    let has_required_checks = has_required_checks(owner, repo, pr_number)?;
                    *required_checks_configured = Some(has_required_checks);
                    has_required_checks
                }
            };
            Ok(status_for_missing_check_output(has_required_checks))
        }
    }
}

fn status_for_missing_check_output(has_required_checks: bool) -> CheckWaitStatus {
    if has_required_checks {
        CheckWaitStatus::Pending
    } else {
        CheckWaitStatus::Complete
    }
}

fn classify_check_result(
    status_success: bool,
    stdout: &[u8],
) -> anyhow::Result<Option<CheckOutcome>> {
    if !status_success {
        return Ok((!stdout.is_empty()).then_some(CheckOutcome::Failed));
    }

    let checks = serde_json::from_slice::<Vec<GhCheck>>(stdout)
        .context("Failed to parse gh pr checks JSON response")?;

    if checks.is_empty() {
        bail!("gh pr checks returned an empty JSON result");
    }

    if checks
        .iter()
        .any(|check| matches!(check.bucket, CheckBucket::Fail | CheckBucket::Cancel))
    {
        return Ok(Some(CheckOutcome::Failed));
    }
    if checks
        .iter()
        .any(|check| check.bucket == CheckBucket::Pending)
    {
        return Ok(Some(CheckOutcome::Pending));
    }

    Ok(Some(CheckOutcome::Complete))
}

fn command_output_text(output: &std::process::Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
    .trim()
    .to_string()
}

fn has_required_checks(owner: &str, repo: &str, pr_number: u64) -> anyhow::Result<bool> {
    has_required_checks_with(
        || fetch_required_check_page(owner, repo, pr_number),
        |branch| fetch_required_status_check_rules(owner, repo, branch),
    )
}

fn has_required_checks_with(
    fetch_page: impl FnOnce() -> anyhow::Result<RequiredCheckPage>,
    fetch_rules: impl FnOnce(&str) -> anyhow::Result<bool>,
) -> anyhow::Result<bool> {
    let page = fetch_page()?;
    if page.has_required_check {
        return Ok(true);
    }
    if page.has_next_page {
        bail!(
            "Cannot confirm that no required checks exist because GitHub returned additional check-context pages."
        );
    }
    // A missing status rollup can mean configured checks have not reported yet.
    fetch_rules(&page.base_ref_name)
}

fn fetch_required_check_page(
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> anyhow::Result<RequiredCheckPage> {
    let args = [
        "api".to_string(),
        "graphql".to_string(),
        "--jq".to_string(),
        REQUIRED_CHECKS_JQ.to_string(),
        "-f".to_string(),
        format!("query={REQUIRED_CHECKS_QUERY}"),
        "-f".to_string(),
        format!("owner={owner}"),
        "-f".to_string(),
        format!("name={repo}"),
        "-F".to_string(),
        format!("number={pr_number}"),
    ];

    let output = run_gh(&args).context("Failed to inspect required check configuration")?;
    if !output.status.success() {
        bail!("gh api graphql failed: {}", command_output_text(&output));
    }
    parse_required_check_page(&output.stdout)
}

const REQUIRED_CHECKS_QUERY: &str = r#"
    query($owner: String!, $name: String!, $number: Int!) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          baseRefName
          commits(last: 1) {
            nodes {
              commit {
                statusCheckRollup {
                  contexts(first: 100) {
                    nodes {
                      ... on CheckRun { isRequired(pullRequestNumber: $number) }
                      ... on StatusContext { isRequired(pullRequestNumber: $number) }
                    }
                    pageInfo { hasNextPage }
                  }
                }
              }
            }
          }
        }
      }
    }
"#;

const REQUIRED_CHECKS_JQ: &str = r#"
    if (.errors // [] | length) > 0 then error("GitHub returned GraphQL errors")
    else
      .data.repository.pullRequest as $pr
      | if $pr == null then error("GitHub response did not include the pull request")
        else
          $pr.commits.nodes[0].commit as $commit
          | if $commit == null then error("GitHub response did not include the pull request head commit")
            elif $commit.statusCheckRollup == null then {baseRefName: $pr.baseRefName, hasRequiredCheck: false, hasNextPage: false}
            else
              $commit.statusCheckRollup.contexts as $contexts
              | {
                  baseRefName: $pr.baseRefName,
                  hasRequiredCheck: any($contexts.nodes[]?; .isRequired),
                  hasNextPage: $contexts.pageInfo.hasNextPage
                }
            end
        end
    end
"#;

const REQUIRED_STATUS_CHECK_RULES_JQ: &str =
    r#"add | any(.[]; .type == "required_status_checks" or .type == "workflows")"#;

fn parse_required_check_page(response: &[u8]) -> anyhow::Result<RequiredCheckPage> {
    serde_json::from_slice(response).context("Failed to parse required check query result")
}

fn fetch_required_status_check_rules(
    owner: &str,
    repo: &str,
    branch: &str,
) -> anyhow::Result<bool> {
    let encoded_branch =
        percent_encoding::utf8_percent_encode(branch, percent_encoding::NON_ALPHANUMERIC);
    let endpoint = format!("repos/{owner}/{repo}/rules/branches/{encoded_branch}");
    let args = [
        "api".to_string(),
        endpoint,
        "--paginate".to_string(),
        "--slurp".to_string(),
        "--jq".to_string(),
        REQUIRED_STATUS_CHECK_RULES_JQ.to_string(),
        "-F".to_string(),
        "per_page=100".to_string(),
    ];

    let output = run_gh(&args).context("Failed to inspect required status check rules")?;
    if !output.status.success() {
        bail!(
            "gh api failed to inspect branch rules: {}",
            command_output_text(&output)
        );
    }
    parse_required_status_check_rules(&output.stdout)
}

fn parse_required_status_check_rules(response: &[u8]) -> anyhow::Result<bool> {
    serde_json::from_slice(response).context("Failed to parse required status check rule result")
}

fn run_gh(args: &[String]) -> anyhow::Result<std::process::Output> {
    let mut command = ExternalTool::Gh.command();
    command.args(args);
    process::run_with_timeout(command, GH_COMMAND_TIMEOUT)
        .with_context(|| format!("Failed to run gh {}", args.join(" ")))
}

#[derive(Debug, PartialEq, Eq)]
enum CheckOutcome {
    Complete,
    Pending,
    Failed,
}

#[derive(Debug, PartialEq, Eq)]
enum CheckWaitStatus {
    Complete,
    Pending,
}

#[derive(Deserialize)]
struct GhCheck {
    bucket: CheckBucket,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum CheckBucket {
    #[serde(rename = "pass")]
    Pass,
    #[serde(rename = "fail")]
    Fail,
    #[serde(rename = "pending")]
    Pending,
    #[serde(rename = "skipping")]
    Skipping,
    #[serde(rename = "cancel")]
    Cancel,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct RequiredCheckPage {
    has_required_check: bool,
    has_next_page: bool,
    base_ref_name: String,
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::all_pass(true, r#"[{"bucket":"pass"}]"#, Some(CheckOutcome::Complete))]
    #[case::skipped_check_passes(true, r#"[{"bucket":"skipping"}]"#, Some(CheckOutcome::Complete))]
    #[case::pending_check_waits(true, r#"[{"bucket":"pending"}]"#, Some(CheckOutcome::Pending))]
    #[case::failed_check_fails(true, r#"[{"bucket":"fail"}]"#, Some(CheckOutcome::Failed))]
    #[case::cancelled_check_fails(true, r#"[{"bucket":"cancel"}]"#, Some(CheckOutcome::Failed))]
    #[case::pass_and_pending_wait(
        true,
        r#"[{"bucket":"pass"},{"bucket":"pending"}]"#,
        Some(CheckOutcome::Pending)
    )]
    #[case::pending_and_fail_fails(
        true,
        r#"[{"bucket":"pending"},{"bucket":"fail"}]"#,
        Some(CheckOutcome::Failed)
    )]
    #[case::pass_and_cancel_fails(
        true,
        r#"[{"bucket":"pass"},{"bucket":"cancel"}]"#,
        Some(CheckOutcome::Failed)
    )]
    #[case::empty_error_needs_required_check_lookup(false, "", None)]
    fn classify_check_result_cases(
        #[case] status_success: bool,
        #[case] stdout: &str,
        #[case] expected: Option<CheckOutcome>,
    ) {
        assert_eq!(
            classify_check_result(status_success, stdout.as_bytes()).unwrap(),
            expected,
        );
    }

    #[rstest]
    #[case::no_required_check(
        r#"{"hasRequiredCheck":false,"hasNextPage":false,"baseRefName":"target-branch"}"#,
        RequiredCheckPage {
            has_required_check: false,
            has_next_page: false,
            base_ref_name: "target-branch".to_string(),
        },
    )]
    #[case::required_check(
        r#"{"hasRequiredCheck":true,"hasNextPage":false,"baseRefName":"target-branch"}"#,
        RequiredCheckPage {
            has_required_check: true,
            has_next_page: false,
            base_ref_name: "target-branch".to_string(),
        },
    )]
    #[case::more_context_pages(
        r#"{"hasRequiredCheck":false,"hasNextPage":true,"baseRefName":"target-branch"}"#,
        RequiredCheckPage {
            has_required_check: false,
            has_next_page: true,
            base_ref_name: "target-branch".to_string(),
        },
    )]
    fn parse_required_check_page_cases(
        #[case] response: &str,
        #[case] expected: RequiredCheckPage,
    ) {
        let page = parse_required_check_page(response.as_bytes()).unwrap();

        assert_eq!(page, expected);
    }

    #[rstest]
    #[case::required_status_rules(b"true", true)]
    #[case::no_required_status_rules(b"false", false)]
    fn parse_required_status_check_rules_cases(#[case] response: &[u8], #[case] expected: bool) {
        assert_eq!(
            parse_required_status_check_rules(response).unwrap(),
            expected
        );
    }

    #[rstest]
    #[case::no_required_check_in_rules(false, false, false, Ok(false), Some("target-branch"))]
    #[case::required_rule_waits_for_first_status(
        false,
        false,
        true,
        Ok(true),
        Some("target-branch")
    )]
    #[case::required_check_found(true, false, false, Ok(true), None)]
    #[case::uninspected_page_fails_closed(
        false,
        true,
        false,
        Err("Cannot confirm that no required checks exist because GitHub returned additional check-context pages.".to_string()),
        None,
    )]
    fn has_required_checks_cases(
        #[case] has_required_check: bool,
        #[case] has_next_page: bool,
        #[case] has_required_rule: bool,
        #[case] expected_result: Result<bool, String>,
        #[case] expected_branch: Option<&str>,
    ) {
        let mut queried_branch = None;
        let result = has_required_checks_with(
            || {
                Ok(RequiredCheckPage {
                    has_required_check,
                    has_next_page,
                    base_ref_name: "target-branch".to_string(),
                })
            },
            |branch| {
                queried_branch = Some(branch.to_string());
                Ok(has_required_rule)
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, queried_branch),
            (expected_result, expected_branch.map(str::to_string))
        );
    }

    #[rstest]
    #[case::configured_check_is_pending(true, CheckWaitStatus::Pending)]
    #[case::no_configured_check_is_complete(false, CheckWaitStatus::Complete)]
    fn missing_check_output_cases(
        #[case] has_required_checks: bool,
        #[case] expected: CheckWaitStatus,
    ) {
        assert_eq!(
            status_for_missing_check_output(has_required_checks),
            expected
        );
    }

    #[rstest]
    fn wait_repeats_pending_queries_until_checks_finish() {
        let mut statuses = VecDeque::from([
            CheckWaitStatus::Pending,
            CheckWaitStatus::Pending,
            CheckWaitStatus::Complete,
        ]);
        let mut waits = 0;

        let result = wait_until_complete(
            || statuses.pop_front().context("No fake check result remains"),
            || waits += 1,
        );

        assert_eq!((result.is_ok(), statuses.len(), waits), (true, 0, 2));
    }
}
