use anyhow::Context;

use super::run_gh;

const PR_MERGE_QUEUE_STATUS_QUERY: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { pullRequest(number: $number) { isInMergeQueue } } }";

pub(super) fn is_pull_request_in_merge_queue(
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> anyhow::Result<bool> {
    let owner_arg = format!("owner={owner}");
    let name_arg = format!("name={repo}");
    let number_arg = format!("number={pr_number}");
    let query_arg = format!("query={PR_MERGE_QUEUE_STATUS_QUERY}");
    let args = [
        "api",
        "graphql",
        "-f",
        owner_arg.as_str(),
        "-f",
        name_arg.as_str(),
        "-F",
        number_arg.as_str(),
        "-f",
        query_arg.as_str(),
    ];
    let output = run_gh(&args)?;
    let response: GhPrMergeQueueResponse = serde_json::from_slice(&output.stdout)
        .context("Failed to parse gh api graphql merge queue response")?;
    Ok(response.data.repository.pull_request.is_in_merge_queue)
}

#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
struct GhPrMergeQueueResponse {
    data: GhPrMergeQueueData,
}

#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
struct GhPrMergeQueueData {
    repository: GhPrMergeQueueRepository,
}

#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPrMergeQueueRepository {
    pull_request: GhPrMergeQueueState,
}

#[derive(Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GhPrMergeQueueState {
    is_in_merge_queue: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gh_pr_merge_queue_response_reads_queue_status() {
        let response = serde_json::from_value::<GhPrMergeQueueResponse>(serde_json::json!({
            "data": {
                "repository": {
                    "pullRequest": { "isInMergeQueue": true },
                },
            },
        }))
        .expect("valid gh merge queue response");

        assert_eq!(
            response,
            GhPrMergeQueueResponse {
                data: GhPrMergeQueueData {
                    repository: GhPrMergeQueueRepository {
                        pull_request: GhPrMergeQueueState {
                            is_in_merge_queue: true,
                        },
                    },
                },
            },
        );
    }
}
