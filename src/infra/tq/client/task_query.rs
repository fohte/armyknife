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

/// Keep fallback priority comparisons bounded when configured groups are numerous.
const MAX_TQ_PRIORITY_ORDER_PROBES: usize = 4;

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
    let priority_order = resolve_task_priority_order(&chunks, max_query_bytes, run_query)?;
    sort_tasks_by_priority(&mut tasks, &priority_order);
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
) -> Result<Vec<Option<i32>>> {
    let mut representatives = Vec::new();
    let mut seen_priorities = HashSet::new();
    for task in chunks.iter().flatten() {
        if seen_priorities.insert(task.priority) {
            representatives.push(task.clone());
        }
    }

    if representatives.len() <= 1 {
        return Ok(representatives
            .into_iter()
            .map(|task| task.priority)
            .collect());
    }

    let representative_ids = representatives
        .iter()
        .map(|task| task.id.clone())
        .collect::<Vec<_>>();
    let priority_chunks = chunk_ids(&representative_ids, max_query_bytes, "ids=".len(), 0, 5)?;
    if priority_chunks.len() == 1 {
        let sorted = run_query(&representative_ids)?;
        let priority_order = ordered_priorities(&sorted);
        if priority_order.len() != representatives.len() {
            return Err(TqError::command_failed(
                TASK_LIST_ARGS,
                "priority order query omitted a requested priority",
                None,
            ));
        }
        return Ok(priority_order);
    }

    // Pairwise probes stay within the URI budget when the representative list does not fit.
    let mut probe_count = 0;
    sort_by_fallible(&mut representatives, &mut |left, right| {
        if probe_count >= MAX_TQ_PRIORITY_ORDER_PROBES {
            return Err(TqError::command_failed(
                TASK_LIST_ARGS,
                "priority ordering exceeds the tq probe limit",
                None,
            ));
        }
        probe_count += 1;
        compare_task_priorities(left, right, max_query_bytes, &mut run_query)
    })?;
    Ok(representatives
        .into_iter()
        .map(|task| task.priority)
        .collect())
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

fn compare_task_priorities(
    left: &TqTaskSummary,
    right: &TqTaskSummary,
    max_query_bytes: usize,
    run_query: &mut impl FnMut(&[String]) -> Result<Vec<TqTaskSummary>>,
) -> Result<std::cmp::Ordering> {
    let ids = [left.id.clone(), right.id.clone()];
    let chunks = chunk_ids(&ids, max_query_bytes, "ids=".len(), 0, 5)?;
    if chunks.len() != 1 {
        return Err(TqError::command_failed(
            TASK_LIST_ARGS,
            "priority comparison identifiers exceed the tq query size budget",
            None,
        ));
    }
    let results = run_query(&chunks[0])?;
    let left_index = results
        .iter()
        .position(|task| task.priority == left.priority);
    let right_index = results
        .iter()
        .position(|task| task.priority == right.priority);
    match (left_index, right_index) {
        (Some(left_index), Some(right_index)) => Ok(left_index.cmp(&right_index)),
        _ => Err(TqError::command_failed(
            TASK_LIST_ARGS,
            "priority comparison query omitted a requested task",
            None,
        )),
    }
}

fn sort_by_fallible<T: Clone>(
    items: &mut [T],
    compare: &mut impl FnMut(&T, &T) -> Result<std::cmp::Ordering>,
) -> Result<()> {
    if items.len() <= 1 {
        return Ok(());
    }

    let middle = items.len() / 2;
    sort_by_fallible(&mut items[..middle], compare)?;
    sort_by_fallible(&mut items[middle..], compare)?;

    let left = items[..middle].to_vec();
    let right = items[middle..].to_vec();
    let mut merged = Vec::with_capacity(items.len());
    let mut left_index = 0;
    let mut right_index = 0;
    while left_index < left.len() && right_index < right.len() {
        if compare(&left[left_index], &right[right_index])? != std::cmp::Ordering::Greater {
            merged.push(left[left_index].clone());
            left_index += 1;
        } else {
            merged.push(right[right_index].clone());
            right_index += 1;
        }
    }
    merged.extend_from_slice(&left[left_index..]);
    merged.extend_from_slice(&right[right_index..]);
    items.clone_from_slice(&merged);
    Ok(())
}

fn run_task_summary_query(task_ids: &[String]) -> Result<Vec<TqTaskSummary>> {
    let args = build_task_list_args(task_ids);
    let output = run_tq_command(&args)?;
    parse_task_list(&String::from_utf8_lossy(&output.stdout))
}

fn sort_tasks_by_priority(tasks: &mut [TqTaskSummary], priority_order: &[Option<i32>]) {
    // Priority values are configurable, so use the order returned by tq.
    let order = priority_order
        .iter()
        .enumerate()
        .map(|(index, priority)| (*priority, index))
        .collect::<HashMap<_, _>>();
    tasks.sort_by_key(|task| order.get(&task.priority).copied().unwrap_or(usize::MAX));
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
    use rstest::rstest;

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
        let tasks = merge_task_chunks(chunks, MAX_TQ_ID_QUERY_BYTES, |_| {
            Ok(vec![
                task_summary("normal", Some(0)),
                task_summary("high", Some(1)),
            ])
        })
        .unwrap();

        assert_eq!(
            tasks,
            vec![
                task_summary("normal", Some(0)),
                task_summary("high", Some(1))
            ],
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

    #[test]
    fn priority_order_fallback_uses_tq_order_for_pairs() {
        let chunks = vec![
            vec![task_summary("c", Some(-1))],
            vec![task_summary("b", Some(1))],
            vec![task_summary("a", Some(0))],
        ];
        let priority_order = resolve_task_priority_order(&chunks, 12, |ids| {
            let mut results = ids
                .iter()
                .map(|id| match id.as_str() {
                    "a" => task_summary("a", Some(0)),
                    "b" => task_summary("b", Some(1)),
                    "c" => task_summary("c", Some(-1)),
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>();
            results.sort_by_key(|task| match task.priority {
                Some(0) => 0,
                Some(-1) => 1,
                Some(1) => 2,
                _ => 3,
            });
            Ok(results)
        })
        .unwrap();

        assert_eq!(priority_order, vec![Some(0), Some(-1), Some(1)]);
    }

    #[test]
    fn priority_order_probe_failure_is_propagated() {
        let chunks = vec![
            vec![task_summary("first", Some(0))],
            vec![task_summary("second", Some(1))],
        ];
        let mut probe_count = 0;

        let result = merge_task_chunks(chunks, MAX_TQ_ID_QUERY_BYTES, |_| {
            probe_count += 1;
            Err(TqError::command_failed(
                TASK_LIST_ARGS,
                "probe failed",
                None,
            ))
        });

        assert_eq!(
            (
                result.map(|_| ()).map_err(|error| error.to_string()),
                probe_count,
            ),
            (
                Err("tq task list --context all --status all --limit unlimited --sort-by priority failed: probe failed".to_string()),
                1,
            ),
        );
    }

    #[test]
    fn priority_order_fallback_stops_after_the_probe_limit() {
        let chunks = ('a'..='h')
            .rev()
            .map(|id| {
                let priority = i32::from(id as u8 - b'a');
                vec![task_summary(&id.to_string(), Some(priority))]
            })
            .collect::<Vec<_>>();
        let mut probe_count = 0;

        let result = resolve_task_priority_order(&chunks, 16, |ids| {
            probe_count += 1;
            let mut tasks = ids
                .iter()
                .map(|id| {
                    let priority = i32::from(id.as_bytes()[0] - b'a');
                    task_summary(id, Some(priority))
                })
                .collect::<Vec<_>>();
            tasks.sort_by_key(|task| task.priority);
            Ok(tasks)
        });

        assert_eq!(
            (
                result.map(|_| ()).map_err(|error| error.to_string()),
                probe_count,
            ),
            (
                Err("tq task list --context all --status all --limit unlimited --sort-by priority failed: priority ordering exceeds the tq probe limit".to_string()),
                MAX_TQ_PRIORITY_ORDER_PROBES,
            ),
        );
    }
}
