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
    wait_until_complete(
        || query_required_check_status(owner, repo, pr_number),
        || {
            println!("Required checks are still pending; retrying in 10 seconds...");
            thread::sleep(CHECK_POLL_INTERVAL);
        },
    )
}

fn wait_until_complete(
    mut poll: impl FnMut() -> anyhow::Result<CheckStatus>,
    mut wait: impl FnMut(),
) -> anyhow::Result<()> {
    loop {
        match poll()? {
            CheckStatus::Complete => return Ok(()),
            CheckStatus::Pending => wait(),
            CheckStatus::Failed => bail!("Required checks failed."),
        }
    }
}

fn query_required_check_status(
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> anyhow::Result<CheckStatus> {
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
    let output = run_gh(&args).context("Failed to query required checks")?;

    match classify_check_result(
        output.status.success(),
        output.status.code(),
        &output.stdout,
    )? {
        Some(CheckStatus::Complete) => Ok(CheckStatus::Complete),
        Some(CheckStatus::Pending) => Ok(CheckStatus::Pending),
        Some(CheckStatus::Failed) => {
            bail!("Required checks failed: {}", command_output_text(&output));
        }
        None if !has_required_checks(owner, repo, pr_number)? => Ok(CheckStatus::Complete),
        None => bail!(
            "gh pr checks returned no structured status for required checks: {}",
            command_output_text(&output),
        ),
    }
}

fn classify_check_result(
    status_success: bool,
    status_code: Option<i32>,
    stdout: &[u8],
) -> anyhow::Result<Option<CheckStatus>> {
    let checks = match serde_json::from_slice::<Vec<GhCheck>>(stdout) {
        Ok(checks) => checks,
        Err(_) if stdout.is_empty() => {
            return Ok((status_code == Some(8)).then_some(CheckStatus::Pending));
        }
        Err(error) => return Err(error).context("Failed to parse gh pr checks JSON response"),
    };

    if checks.is_empty() {
        return Ok(if status_success {
            Some(CheckStatus::Complete)
        } else if status_code == Some(8) {
            Some(CheckStatus::Pending)
        } else {
            None
        });
    }

    if checks
        .iter()
        .any(|check| matches!(check.bucket, CheckBucket::Fail | CheckBucket::Cancel))
    {
        return Ok(Some(CheckStatus::Failed));
    }
    if checks
        .iter()
        .any(|check| check.bucket == CheckBucket::Pending)
    {
        return Ok(Some(CheckStatus::Pending));
    }
    if status_code == Some(8) {
        return Ok(Some(CheckStatus::Failed));
    }

    Ok(Some(if status_success {
        CheckStatus::Complete
    } else {
        CheckStatus::Failed
    }))
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
    has_required_checks_with(|cursor| fetch_required_check_page(owner, repo, pr_number, cursor))
}

fn has_required_checks_with(
    mut fetch_page: impl FnMut(Option<&str>) -> anyhow::Result<RequiredCheckPage>,
) -> anyhow::Result<bool> {
    let mut cursor = None;
    loop {
        let page = fetch_page(cursor.as_deref())?;
        if page.has_required_check {
            return Ok(true);
        }
        let Some(next_cursor) = page.next_cursor else {
            return Ok(false);
        };
        cursor = Some(next_cursor);
    }
}

fn fetch_required_check_page(
    owner: &str,
    repo: &str,
    pr_number: u64,
    cursor: Option<&str>,
) -> anyhow::Result<RequiredCheckPage> {
    let mut args = vec![
        "api".to_string(),
        "graphql".to_string(),
        "-f".to_string(),
        format!("query={REQUIRED_CHECKS_QUERY}"),
        "-f".to_string(),
        format!("owner={owner}"),
        "-f".to_string(),
        format!("name={repo}"),
        "-F".to_string(),
        format!("number={pr_number}"),
    ];
    if let Some(cursor) = cursor {
        args.extend(["-f".to_string(), format!("endCursor={cursor}")]);
    }

    let output = run_gh(&args).context("Failed to inspect required check configuration")?;
    if !output.status.success() {
        bail!("gh api graphql failed: {}", command_output_text(&output));
    }
    parse_required_check_page(&output.stdout)
}

const REQUIRED_CHECKS_QUERY: &str = r#"
    query($owner: String!, $name: String!, $number: Int!, $endCursor: String) {
      repository(owner: $owner, name: $name) {
        pullRequest(number: $number) {
          commits(last: 1) {
            nodes {
              commit {
                statusCheckRollup {
                  contexts(first: 100, after: $endCursor) {
                    nodes {
                      ... on CheckRun { isRequired(pullRequestNumber: $number) }
                      ... on StatusContext { isRequired(pullRequestNumber: $number) }
                    }
                    pageInfo { hasNextPage endCursor }
                  }
                }
              }
            }
          }
        }
      }
    }
"#;

fn parse_required_check_page(response: &[u8]) -> anyhow::Result<RequiredCheckPage> {
    let response: GhGraphQlResponse = serde_json::from_slice(response)
        .context("Failed to parse required check query response")?;
    let repository = response
        .data
        .repository
        .context("GitHub GraphQL response did not include the repository")?;
    let pull_request = repository
        .pull_request
        .context("GitHub GraphQL response did not include the pull request")?;
    let commit = pull_request
        .commits
        .nodes
        .into_iter()
        .next()
        .context("GitHub GraphQL response did not include the pull request head commit")?;
    let Some(rollup) = commit.commit.status_check_rollup else {
        return Ok(RequiredCheckPage {
            has_required_check: false,
            next_cursor: None,
        });
    };
    let contexts = rollup.contexts;
    let next_cursor = if contexts.page_info.has_next_page {
        Some(
            contexts
                .page_info
                .end_cursor
                .context("GitHub GraphQL response omitted the next check cursor")?,
        )
    } else {
        None
    };

    Ok(RequiredCheckPage {
        has_required_check: contexts.nodes.iter().any(|check| check.is_required),
        next_cursor,
    })
}

fn run_gh(args: &[String]) -> anyhow::Result<std::process::Output> {
    let mut command = ExternalTool::Gh.command();
    command.args(args);
    process::run_with_timeout(command, GH_COMMAND_TIMEOUT)
        .with_context(|| format!("Failed to run gh {}", args.join(" ")))
}

#[derive(Debug, PartialEq, Eq)]
enum CheckStatus {
    Complete,
    Pending,
    Failed,
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

#[derive(Debug, PartialEq, Eq)]
struct RequiredCheckPage {
    has_required_check: bool,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct GhGraphQlResponse {
    data: GhGraphQlData,
}

#[derive(Deserialize)]
struct GhGraphQlData {
    repository: Option<GhGraphQlRepository>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhGraphQlRepository {
    pull_request: Option<GhGraphQlPullRequest>,
}

#[derive(Deserialize)]
struct GhGraphQlPullRequest {
    commits: GhGraphQlCommitConnection,
}

#[derive(Deserialize)]
struct GhGraphQlCommitConnection {
    nodes: Vec<GhGraphQlCommitNode>,
}

#[derive(Deserialize)]
struct GhGraphQlCommitNode {
    commit: GhGraphQlCommit,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhGraphQlCommit {
    status_check_rollup: Option<GhGraphQlStatusCheckRollup>,
}

#[derive(Deserialize)]
struct GhGraphQlStatusCheckRollup {
    contexts: GhGraphQlCheckContexts,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhGraphQlCheckContexts {
    nodes: Vec<GhGraphQlCheckContext>,
    page_info: GhGraphQlPageInfo,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhGraphQlCheckContext {
    is_required: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhGraphQlPageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use indoc::indoc;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::all_pass(true, Some(0), r#"[{"bucket":"pass"}]"#, Some(CheckStatus::Complete))]
    #[case::skipped_check_passes(
        true,
        Some(0),
        r#"[{"bucket":"skipping"}]"#,
        Some(CheckStatus::Complete)
    )]
    #[case::pending_bucket_waits(
        false,
        Some(8),
        r#"[{"bucket":"pending"}]"#,
        Some(CheckStatus::Pending)
    )]
    #[case::pending_json_does_not_need_failure_exit_code(
        true,
        Some(0),
        r#"[{"bucket":"pending"}]"#,
        Some(CheckStatus::Pending)
    )]
    #[case::pending_exit_code_waits(false, Some(8), "", Some(CheckStatus::Pending))]
    #[case::failed_bucket_fails(
        false,
        Some(1),
        r#"[{"bucket":"fail"}]"#,
        Some(CheckStatus::Failed)
    )]
    #[case::failed_json_does_not_need_failure_exit_code(
        true,
        Some(0),
        r#"[{"bucket":"fail"}]"#,
        Some(CheckStatus::Failed)
    )]
    #[case::cancelled_check_fails(
        true,
        Some(0),
        r#"[{"bucket":"cancel"}]"#,
        Some(CheckStatus::Failed)
    )]
    #[case::empty_success_is_no_required_checks(true, Some(0), "[]", Some(CheckStatus::Complete))]
    #[case::empty_error_needs_required_check_lookup(false, Some(1), "", None)]
    fn classify_check_result_cases(
        #[case] status_success: bool,
        #[case] status_code: Option<i32>,
        #[case] stdout: &str,
        #[case] expected: Option<CheckStatus>,
    ) {
        assert_eq!(
            classify_check_result(status_success, status_code, stdout.as_bytes()).unwrap(),
            expected,
        );
    }

    #[rstest]
    #[case::no_required_checks(
        indoc! {r#"
            {
              "data": {
                "repository": {
                  "pullRequest": {
                    "commits": {
                      "nodes": [{
                        "commit": {
                          "statusCheckRollup": {
                            "contexts": {
                              "nodes": [{"isRequired": false}],
                              "pageInfo": {"hasNextPage": false, "endCursor": null}
                            }
                          }
                        }
                      }]
                    }
                  }
                }
              }
            }
        "#},
        false,
        None,
    )]
    #[case::required_checks(
        indoc! {r#"
            {
              "data": {
                "repository": {
                  "pullRequest": {
                    "commits": {
                      "nodes": [{
                        "commit": {
                          "statusCheckRollup": {
                            "contexts": {
                              "nodes": [{"isRequired": true}],
                              "pageInfo": {"hasNextPage": true, "endCursor": "opaque-cursor"}
                            }
                          }
                        }
                      }]
                    }
                  }
                }
              }
            }
        "#},
        true,
        Some("opaque-cursor"),
    )]
    fn parse_required_check_page_cases(
        #[case] response: &str,
        #[case] expected_has_required_check: bool,
        #[case] expected_next_cursor: Option<&str>,
    ) {
        let page = parse_required_check_page(response.as_bytes()).unwrap();

        assert_eq!(
            (page.has_required_check, page.next_cursor.as_deref()),
            (expected_has_required_check, expected_next_cursor),
        );
    }

    #[rstest]
    fn has_required_checks_follows_pagination_until_a_required_check_is_found() {
        let mut pages = VecDeque::from([
            RequiredCheckPage {
                has_required_check: false,
                next_cursor: Some("opaque-cursor".to_string()),
            },
            RequiredCheckPage {
                has_required_check: true,
                next_cursor: None,
            },
        ]);
        let mut cursors = Vec::new();

        let result = has_required_checks_with(|cursor| {
            cursors.push(cursor.map(str::to_string));
            pages
                .pop_front()
                .context("No fake required-check page remains")
        });

        assert_eq!(
            (result.unwrap(), cursors),
            (true, vec![None, Some("opaque-cursor".to_string())]),
        );
    }

    #[rstest]
    fn wait_repeats_pending_queries_until_checks_finish() {
        let mut statuses = VecDeque::from([
            CheckStatus::Pending,
            CheckStatus::Pending,
            CheckStatus::Complete,
        ]);
        let mut waits = 0;

        let result = wait_until_complete(
            || statuses.pop_front().context("No fake check result remains"),
            || waits += 1,
        );

        assert_eq!((result.is_ok(), statuses.len(), waits), (true, 0, 2));
    }
}
