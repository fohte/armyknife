use std::collections::{HashMap, HashSet};

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};

use super::{
    Result, SessionTasks, TqError, TqTaskSummary, parse_session_list, parse_task_list,
    run_tq_command,
};

const SESSION_LIST_ARGS: &[&str] = &["session", "list", "--limit", "unlimited"];
const TASK_LIST_ARGS: &[&str] = &[
    "task",
    "list",
    "--context",
    "all",
    "--status",
    "all",
    "--limit",
    "unlimited",
    "--sort-by",
    "priority",
];

/// Keep variable query parameters below common reverse-proxy request-line
/// limits while reserving space for the endpoint and fixed filters.
const MAX_TQ_REQUEST_URI_BYTES: usize = 4 * 1024;
const TQ_STATIC_REQUEST_URI_RESERVE_BYTES: usize = 1024;
const MAX_TQ_ID_QUERY_BYTES: usize = MAX_TQ_REQUEST_URI_BYTES - TQ_STATIC_REQUEST_URI_RESERVE_BYTES;

/// Runs `tq session list --limit unlimited --session-id <id> ...` once per
/// query chunk and combines the parsed results. Blocking, so callers must run
/// it via [`tokio::task::spawn_blocking`].
pub(super) fn run_session_list(session_ids: &[String]) -> Result<Vec<SessionTasks>> {
    run_chunked_query(
        session_ids,
        MAX_TQ_ID_QUERY_BYTES,
        0,
        "session-id=".len(),
        1,
        |chunk| {
            let args = build_session_list_args(chunk);
            let output = run_tq_command(&args)?;
            parse_session_list(&String::from_utf8_lossy(&output.stdout))
        },
    )
}

/// Builds the argv for [`run_session_list`]. Isolated from the process spawn
/// so it can be unit tested without invoking the real `tq` binary.
pub(super) fn build_session_list_args(session_ids: &[String]) -> Vec<String> {
    let mut args: Vec<String> = SESSION_LIST_ARGS.iter().map(|s| s.to_string()).collect();
    for id in session_ids {
        args.push("--session-id".to_string());
        args.push(id.clone());
    }
    args
}

pub(super) fn build_task_list_args(task_ids: &[String]) -> Vec<String> {
    let mut args: Vec<String> = TASK_LIST_ARGS.iter().map(|s| s.to_string()).collect();
    args.push("--ids".to_string());
    args.push(task_ids.join(","));
    args
}

pub(super) fn run_task_list(task_ids: &[String]) -> Result<Vec<TqTaskSummary>> {
    let chunks = run_chunked_query_batches(
        task_ids,
        MAX_TQ_ID_QUERY_BYTES,
        "ids=".len(),
        0,
        5,
        |chunk| {
            let args = build_task_list_args(chunk);
            let output = run_tq_command(&args)?;
            parse_task_list(&String::from_utf8_lossy(&output.stdout))
        },
    )?;

    merge_task_chunks(chunks, MAX_TQ_ID_QUERY_BYTES, run_task_summary_query)
}

fn merge_task_chunks(
    chunks: Vec<Vec<TqTaskSummary>>,
    max_query_bytes: usize,
    run_query: impl FnMut(&[String]) -> Result<Vec<TqTaskSummary>>,
) -> Result<Vec<TqTaskSummary>> {
    if chunks.len() <= 1 {
        return Ok(chunks.into_iter().flatten().collect());
    }

    let mut tasks = chunks.iter().flatten().cloned().collect::<Vec<_>>();
    if let Some(priority_order) = resolve_task_priority_order(&chunks, max_query_bytes, run_query) {
        sort_tasks_by_configured_order(&mut tasks, &priority_order);
    } else {
        sort_tasks_by_priority(&mut tasks);
    }
    Ok(tasks)
}

fn run_chunked_query<T>(
    ids: &[String],
    max_query_bytes: usize,
    fixed_bytes: usize,
    item_overhead_bytes: usize,
    separator_bytes: usize,
    run_chunk: impl FnMut(&[String]) -> Result<Vec<T>>,
) -> Result<Vec<T>> {
    Ok(run_chunked_query_batches(
        ids,
        max_query_bytes,
        fixed_bytes,
        item_overhead_bytes,
        separator_bytes,
        run_chunk,
    )?
    .into_iter()
    .flatten()
    .collect())
}

fn run_chunked_query_batches<T>(
    ids: &[String],
    max_query_bytes: usize,
    fixed_bytes: usize,
    item_overhead_bytes: usize,
    separator_bytes: usize,
    mut run_chunk: impl FnMut(&[String]) -> Result<Vec<T>>,
) -> Result<Vec<Vec<T>>> {
    let chunks = chunk_ids(
        ids,
        max_query_bytes,
        fixed_bytes,
        item_overhead_bytes,
        separator_bytes,
    )?;
    let mut results = Vec::new();
    for chunk in chunks {
        results.push(run_chunk(&chunk)?);
    }
    Ok(results)
}

fn resolve_task_priority_order(
    chunks: &[Vec<TqTaskSummary>],
    max_query_bytes: usize,
    mut run_query: impl FnMut(&[String]) -> Result<Vec<TqTaskSummary>>,
) -> Option<Vec<Option<i32>>> {
    let mut representatives = Vec::new();
    let mut seen_priorities = HashSet::new();
    for task in chunks.iter().flatten() {
        if seen_priorities.insert(task.priority) {
            representatives.push(task.clone());
        }
    }

    if representatives.len() <= 1 {
        return Some(
            representatives
                .into_iter()
                .map(|task| task.priority)
                .collect(),
        );
    }

    let representative_ids = representatives
        .iter()
        .map(|task| task.id.clone())
        .collect::<Vec<_>>();
    let priority_chunks = match chunk_ids(&representative_ids, max_query_bytes, "ids=".len(), 0, 5)
    {
        Ok(chunks) => chunks,
        Err(error) => {
            tracing::warn!(%error, "Falling back to local tq task priority ordering");
            return None;
        }
    };
    if priority_chunks.len() != 1 {
        tracing::warn!(
            "Skipping tq priority-order query because representative IDs exceed the URI budget"
        );
        return None;
    }

    let sorted = match run_query(&representative_ids) {
        Ok(tasks) => tasks,
        Err(error) => {
            tracing::warn!(%error, "Falling back to local tq task priority ordering");
            return None;
        }
    };
    let requested_ids = representative_ids.iter().collect::<HashSet<_>>();
    let returned_ids = sorted.iter().map(|task| &task.id).collect::<HashSet<_>>();
    let requested_priorities = representatives
        .iter()
        .map(|task| task.priority)
        .collect::<HashSet<_>>();
    let returned_priorities = sorted
        .iter()
        .map(|task| task.priority)
        .collect::<HashSet<_>>();
    if sorted.len() != representatives.len()
        || requested_ids != returned_ids
        || requested_priorities != returned_priorities
    {
        tracing::warn!("Tq returned an incomplete priority-order result; using local ordering");
        return None;
    }

    Some(ordered_priorities(&sorted))
}

fn ordered_priorities(tasks: &[TqTaskSummary]) -> Vec<Option<i32>> {
    let mut priorities = Vec::new();
    for task in tasks {
        if !priorities.contains(&task.priority) {
            priorities.push(task.priority);
        }
    }
    priorities
}

fn run_task_summary_query(task_ids: &[String]) -> Result<Vec<TqTaskSummary>> {
    let args = build_task_list_args(task_ids);
    let output = run_tq_command(&args)?;
    parse_task_list(&String::from_utf8_lossy(&output.stdout))
}

fn sort_tasks_by_configured_order(tasks: &mut [TqTaskSummary], priority_order: &[Option<i32>]) {
    // Priority values are configurable, so use the order returned by tq.
    let order = priority_order
        .iter()
        .enumerate()
        .map(|(index, priority)| (*priority, index))
        .collect::<HashMap<_, _>>();
    tasks.sort_by_key(|task| order.get(&task.priority).copied().unwrap_or(usize::MAX));
}

fn sort_tasks_by_priority(tasks: &mut [TqTaskSummary]) {
    tasks.sort_by_key(|task| match task.priority {
        Some(priority) if priority >= 0 => (0, std::cmp::Reverse(priority)),
        None => (1, std::cmp::Reverse(0)),
        Some(priority) => (2, std::cmp::Reverse(priority)),
    });
}

/// Splits identifiers so their encoded query component stays within a
/// conservative request-URI budget. `fixed_bytes` accounts for one query
/// key, `item_overhead_bytes` for a key repeated per item, and
/// `separator_bytes` for separators between values.
fn chunk_ids(
    ids: &[String],
    max_query_bytes: usize,
    fixed_bytes: usize,
    item_overhead_bytes: usize,
    separator_bytes: usize,
) -> Result<Vec<Vec<String>>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut chunks = Vec::new();
    let mut chunk = Vec::new();
    let mut query_bytes = fixed_bytes;

    for id in ids {
        let item_bytes = encoded_query_value_len(id) + item_overhead_bytes;
        let separator = usize::from(!chunk.is_empty()) * separator_bytes;
        if fixed_bytes + item_bytes > max_query_bytes {
            return Err(TqError::command_failed(
                &[] as &[&str],
                "an identifier exceeds the tq query size budget",
                None,
            ));
        }
        if !chunk.is_empty() && query_bytes + separator + item_bytes > max_query_bytes {
            chunks.push(std::mem::take(&mut chunk));
            query_bytes = fixed_bytes;
        }

        query_bytes += usize::from(!chunk.is_empty()) * separator_bytes + item_bytes;
        chunk.push(id.clone());
    }

    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    Ok(chunks)
}

fn encoded_query_value_len(value: &str) -> usize {
    utf8_percent_encode(value, NON_ALPHANUMERIC)
        .to_string()
        .len()
}

#[cfg(test)]
mod tests {
    use rstest::{fixture, rstest};

    use super::*;

    fn task_summary(id: &str, priority: Option<i32>) -> TqTaskSummary {
        TqTaskSummary {
            id: id.to_string(),
            number: 1,
            title: id.to_string(),
            updated_at: None,
            priority,
            project_id: None,
        }
    }

    #[fixture]
    fn out_of_order_priority_chunks() -> Vec<Vec<TqTaskSummary>> {
        vec![
            vec![task_summary("d", Some(-1))],
            vec![task_summary("u", None)],
            vec![task_summary("n", Some(0))],
            vec![task_summary("h", Some(1))],
        ]
    }

    #[rstest]
    #[case::no_ids(&[], vec!["session", "list", "--limit", "unlimited"])]
    #[case::single_id(
        &["session-1".to_string()],
        vec!["session", "list", "--limit", "unlimited", "--session-id", "session-1"],
    )]
    #[case::multiple_ids(
        &["session-1".to_string(), "session-2".to_string()],
        vec![
            "session", "list", "--limit", "unlimited",
            "--session-id", "session-1",
            "--session-id", "session-2",
        ],
    )]
    fn builds_session_list_args_cases(#[case] ids: &[String], #[case] expected: Vec<&str>) {
        assert_eq!(build_session_list_args(ids), expected);
    }

    #[test]
    fn build_task_list_args_uses_all_statuses_unlimited_and_tq_priority_order() {
        assert_eq!(
            build_task_list_args(&["task-one".to_string(), "task-two".to_string()]),
            vec![
                "task",
                "list",
                "--context",
                "all",
                "--status",
                "all",
                "--limit",
                "unlimited",
                "--sort-by",
                "priority",
                "--ids",
                "task-one,task-two",
            ],
        );
    }

    #[test]
    fn chunks_repeated_session_ids_by_encoded_query_size() {
        assert_eq!(
            chunk_ids(
                &["aa".to_string(), "bb".to_string(), "cc".to_string()],
                32,
                0,
                "session-id=".len(),
                1,
            )
            .unwrap(),
            vec![
                vec!["aa".to_string(), "bb".to_string()],
                vec!["cc".to_string()],
            ],
        );
    }

    #[test]
    fn chunks_comma_separated_task_ids_with_encoded_delimiters() {
        assert_eq!(
            chunk_ids(
                &["a/b".to_string(), "cd".to_string(), "ef".to_string()],
                15,
                "ids=".len(),
                0,
                5,
            )
            .unwrap(),
            vec![
                vec!["a/b".to_string()],
                vec!["cd".to_string(), "ef".to_string()],
            ],
        );
    }

    #[test]
    fn runs_each_chunk_and_combines_all_results() {
        let ids = vec!["aa".to_string(), "bb".to_string(), "cc".to_string()];
        let mut executed_chunks = Vec::new();
        let results = run_chunked_query(&ids, 3, 0, 0, 1, |chunk| {
            executed_chunks.push(chunk.to_vec());
            Ok(chunk.to_vec())
        })
        .unwrap();

        assert_eq!(
            (results, executed_chunks),
            (
                ids,
                vec![
                    vec!["aa".to_string()],
                    vec!["bb".to_string()],
                    vec!["cc".to_string()],
                ],
            ),
        );
    }

    #[test]
    fn merged_task_summaries_keep_the_tq_configured_priority_order() {
        let chunks = vec![
            vec![task_summary("high", Some(1))],
            vec![task_summary("normal", Some(0))],
        ];
        let mut probe_count = 0;
        let tasks = merge_task_chunks(chunks, MAX_TQ_ID_QUERY_BYTES, |_| {
            probe_count += 1;
            Ok(vec![
                task_summary("normal", Some(0)),
                task_summary("high", Some(1)),
            ])
        })
        .unwrap();

        assert_eq!(
            (tasks, probe_count),
            (
                vec![
                    task_summary("normal", Some(0)),
                    task_summary("high", Some(1))
                ],
                1,
            ),
        );
    }

    #[test]
    fn single_task_query_preserves_tq_order_without_a_probe() {
        let tq_order = vec![
            task_summary("normal", Some(0)),
            task_summary("high", Some(1)),
        ];
        let mut probe_count = 0;

        let result = merge_task_chunks(vec![tq_order.clone()], MAX_TQ_ID_QUERY_BYTES, |_| {
            probe_count += 1;
            Ok(Vec::new())
        })
        .unwrap();

        assert_eq!((result, probe_count), (tq_order, 0));
    }

    #[rstest]
    fn priority_query_over_uri_budget_falls_back_to_local_order(
        out_of_order_priority_chunks: Vec<Vec<TqTaskSummary>>,
    ) {
        let mut probe_count = 0;

        let result = merge_task_chunks(out_of_order_priority_chunks, 12, |_| {
            probe_count += 1;
            Ok(Vec::new())
        })
        .unwrap();

        assert_eq!(
            (result, probe_count),
            (
                vec![
                    task_summary("h", Some(1)),
                    task_summary("n", Some(0)),
                    task_summary("u", None),
                    task_summary("d", Some(-1)),
                ],
                0,
            ),
        );
    }

    #[rstest]
    fn incomplete_priority_query_falls_back_to_local_order(
        out_of_order_priority_chunks: Vec<Vec<TqTaskSummary>>,
    ) {
        let mut probe_count = 0;

        let result = merge_task_chunks(out_of_order_priority_chunks, MAX_TQ_ID_QUERY_BYTES, |_| {
            probe_count += 1;
            Ok(vec![task_summary("h", Some(1))])
        })
        .unwrap();

        assert_eq!(
            (result, probe_count),
            (
                vec![
                    task_summary("h", Some(1)),
                    task_summary("n", Some(0)),
                    task_summary("u", None),
                    task_summary("d", Some(-1)),
                ],
                1,
            ),
        );
    }

    #[rstest]
    fn failed_priority_query_falls_back_to_local_order(
        out_of_order_priority_chunks: Vec<Vec<TqTaskSummary>>,
    ) {
        let mut probe_count = 0;

        let result = merge_task_chunks(out_of_order_priority_chunks, MAX_TQ_ID_QUERY_BYTES, |_| {
            probe_count += 1;
            Err(TqError::command_failed(
                TASK_LIST_ARGS,
                "probe failed",
                None,
            ))
        })
        .unwrap();

        assert_eq!(
            (result, probe_count),
            (
                vec![
                    task_summary("h", Some(1)),
                    task_summary("n", Some(0)),
                    task_summary("u", None),
                    task_summary("d", Some(-1)),
                ],
                1,
            ),
        );
    }
}
