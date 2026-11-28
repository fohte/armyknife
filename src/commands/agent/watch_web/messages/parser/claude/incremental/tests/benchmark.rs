use std::hint::black_box;
use std::io::Cursor;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{IncrementalAppend, IncrementalClaudeParser, empty_additions, fixed_time, parse_all};

const RECORDS_PER_SHAPE: usize = 20_000;
const APPENDS_PER_ROUND: usize = 200;
const APPEND_ROUNDS: usize = 3;
const RENDER_ITERATIONS: usize = 50;
const FULL_PARSE_ITERATIONS: usize = 5;

#[test]
#[ignore = "manual performance comparison"]
fn measure_20k_transcript_shapes() {
    let shape_a_prefix = jsonl(linear_records(0, RECORDS_PER_SHAPE, None));
    let shape_a_last_uuid = format!("bench-a-{}", RECORDS_PER_SHAPE - 1);
    let shape_a_suffix = linear_records(
        RECORDS_PER_SHAPE,
        APPENDS_PER_ROUND,
        Some(shape_a_last_uuid),
    );
    measure_shape("A", &shape_a_prefix, &shape_a_suffix);

    let shape_b_cycles = RECORDS_PER_SHAPE / 10;
    let shape_b_prefix = jsonl(parallel_tool_cycles(
        "bench-b",
        0,
        shape_b_cycles,
        None,
        false,
    ));
    let shape_b_last_uuid = format!("bench-b-{}-continuation", shape_b_cycles - 1);
    let shape_b_suffix = parallel_tool_cycles(
        "bench-b",
        shape_b_cycles,
        APPENDS_PER_ROUND / 10,
        Some(shape_b_last_uuid),
        false,
    );
    measure_shape("B", &shape_b_prefix, &shape_b_suffix);

    let shape_c_prefix = jsonl(parallel_tool_cycles(
        "bench-c",
        0,
        shape_b_cycles,
        None,
        true,
    ));
    let shape_c_last_uuid = format!("bench-c-{}-continuation", shape_b_cycles - 1);
    let shape_c_suffix = parallel_tool_cycles(
        "bench-c",
        shape_b_cycles,
        APPENDS_PER_ROUND / 10,
        Some(shape_c_last_uuid),
        true,
    );
    measure_shape("C", &shape_c_prefix, &shape_c_suffix);
}

fn measure_shape(name: &str, prefix: &str, suffix_records: &[Value]) {
    let mut append_total = Duration::ZERO;
    let mut restart_reparse_total = Duration::ZERO;
    let mut restart_count = 0;
    let mut final_state = None;

    for _ in 0..APPEND_ROUNDS {
        let mut parser =
            IncrementalClaudeParser::new(Cursor::new(prefix.as_bytes()), None).unwrap();
        black_box(parser.render_with(fixed_time(), empty_additions()).unwrap());
        let mut transcript = prefix.as_bytes().to_vec();

        for record in suffix_records {
            let mut line = record.to_string().into_bytes();
            line.push(b'\n');

            let started = Instant::now();
            let result = parser.append_bytes(&line, fixed_time()).unwrap();
            append_total += started.elapsed();
            transcript.extend_from_slice(&line);

            if result == IncrementalAppend::RestartRequired {
                restart_count += 1;
                let started = Instant::now();
                parser =
                    IncrementalClaudeParser::new(Cursor::new(transcript.as_slice()), None).unwrap();
                restart_reparse_total += started.elapsed();
            }
            black_box(parser.render_with(fixed_time(), empty_additions()).unwrap());
        }

        let actual = parser.render_with(fixed_time(), empty_additions()).unwrap();
        let expected = parse_all(&transcript, empty_additions());
        let first_message_difference = actual
            .0
            .iter()
            .zip(&expected.0)
            .position(|(actual, expected)| actual != expected);
        assert!(
            actual == expected,
            "shape={name} actual_messages={} expected_messages={} first_message_difference={first_message_difference:?}",
            actual.0.len(),
            expected.0.len(),
        );
        final_state = Some((parser, transcript));
    }

    let (parser, _) = final_state.unwrap();
    let mut render_total = Duration::ZERO;
    for _ in 0..RENDER_ITERATIONS {
        let started = Instant::now();
        black_box(parser.render_with(fixed_time(), empty_additions()).unwrap());
        render_total += started.elapsed();
    }

    let mut full_parse_total = Duration::ZERO;
    for _ in 0..FULL_PARSE_ITERATIONS {
        let started = Instant::now();
        black_box(parse_all(prefix.as_bytes(), empty_additions()));
        full_parse_total += started.elapsed();
    }

    let append_count = APPEND_ROUNDS * suffix_records.len();
    let append_mean_us = append_total.as_nanos() as f64 / append_count as f64 / 1_000.0;
    let restart_rate_pct = restart_count as f64 / append_count as f64 * 100.0;
    let restart_reparse_mean_ms = if restart_count == 0 {
        0.0
    } else {
        restart_reparse_total.as_nanos() as f64 / restart_count as f64 / 1_000_000.0
    };
    let render_mean_ms = render_total.as_nanos() as f64 / RENDER_ITERATIONS as f64 / 1_000_000.0;
    let full_parse_mean_ms =
        full_parse_total.as_nanos() as f64 / FULL_PARSE_ITERATIONS as f64 / 1_000_000.0;

    println!(
        "shape={name} records={RECORDS_PER_SHAPE} append_samples={append_count} \
         append_mean_us={append_mean_us:.3} restart_rate_pct={restart_rate_pct:.1} \
         restart_reparse_mean_ms={restart_reparse_mean_ms:.3} \
         render_mean_ms={render_mean_ms:.3} full_parse_mean_ms={full_parse_mean_ms:.3}"
    );
}

fn jsonl(records: impl IntoIterator<Item = Value>) -> String {
    let mut transcript = String::new();
    for record in records {
        transcript.push_str(&record.to_string());
        transcript.push('\n');
    }
    transcript
}

fn linear_records(start_index: usize, count: usize, mut parent_uuid: Option<String>) -> Vec<Value> {
    (start_index..start_index + count)
        .map(|index| {
            let uuid = format!("bench-a-{index}");
            let record = if index % 2 == 0 {
                json!({
                    "type": "user",
                    "uuid": uuid,
                    "parentUuid": parent_uuid,
                    "message": {"content": "Sample request."},
                })
            } else {
                json!({
                    "type": "assistant",
                    "uuid": uuid,
                    "parentUuid": parent_uuid,
                    "message": {"content": [{"type": "text", "text": "Sample response."}]},
                })
            };
            parent_uuid = Some(uuid);
            record
        })
        .collect()
}

fn parallel_tool_cycles(
    id_prefix: &str,
    start_cycle: usize,
    cycle_count: usize,
    mut parent_uuid: Option<String>,
    include_realistic_records: bool,
) -> Vec<Value> {
    let mut records = Vec::with_capacity(cycle_count * 10);
    for cycle in start_cycle..start_cycle + cycle_count {
        let request = format!("{id_prefix}-{cycle}-request");
        let first_call = format!("{id_prefix}-{cycle}-call-a");
        let second_call = format!("{id_prefix}-{cycle}-call-b");
        let first_result = format!("{id_prefix}-{cycle}-result-a");
        let first_followup = format!("{id_prefix}-{cycle}-followup-a");
        let second_result = format!("{id_prefix}-{cycle}-result-b");
        let second_followup = format!("{id_prefix}-{cycle}-followup-b");
        let hook = format!("{id_prefix}-{cycle}-hook");
        let queued_command = format!("{id_prefix}-{cycle}-queued");
        let continuation = format!("{id_prefix}-{cycle}-continuation");
        let first_tool_id = format!("{id_prefix}-{cycle}-tool-a");
        let second_tool_id = format!("{id_prefix}-{cycle}-tool-b");

        records.extend([
            json!({
                "type": "user",
                "uuid": request,
                "parentUuid": parent_uuid,
                "message": {"content": "Sample request."},
            }),
            json!({
                "type": "assistant",
                "uuid": first_call,
                "parentUuid": request,
                "message": {"content": [{
                    "type": "tool_use",
                    "id": first_tool_id,
                    "name": "Read",
                    "input": {"file_path": "/fixture/sample-a.md"},
                }]},
            }),
            json!({
                "type": "assistant",
                "uuid": second_call,
                "parentUuid": first_call,
                "message": {"content": [{
                    "type": "tool_use",
                    "id": second_tool_id,
                    "name": "Read",
                    "input": {"file_path": "/fixture/sample-b.md"},
                }]},
            }),
            json!({
                "type": "user",
                "uuid": first_result,
                "parentUuid": first_call,
                "message": {"content": [{
                    "type": "tool_result",
                    "tool_use_id": first_tool_id,
                    "content": "Sample output.",
                }]},
            }),
            json!({
                "type": "assistant",
                "uuid": first_followup,
                "parentUuid": first_result,
                "message": {"content": [{"type": "text", "text": "First result is ready."}]},
            }),
            json!({
                "type": "user",
                "uuid": second_result,
                "parentUuid": second_call,
                "message": {"content": [{
                    "type": "tool_result",
                    "tool_use_id": second_tool_id,
                    "content": "Sample output.",
                }]},
            }),
            json!({
                "type": "assistant",
                "uuid": second_followup,
                "parentUuid": second_result,
                "message": {"content": [{"type": "text", "text": "Second result is ready."}]},
            }),
        ]);
        records.push(json!({
            "type": "attachment",
            "uuid": hook,
            "parentUuid": second_followup,
            "attachment": {"type": "hook_success", "hookName": "PostToolUse"},
        }));
        let (intermediate_record, continuation_parent) = if include_realistic_records {
            match cycle % 4 {
                0 => (
                    json!({
                        "type": "attachment",
                        "uuid": queued_command,
                        "parentUuid": hook,
                        "attachment": {"type": "queued_command", "prompt": ""},
                    }),
                    queued_command,
                ),
                1 => (
                    json!({
                        "type": "attachment",
                        "uuid": queued_command,
                        "parentUuid": hook,
                        "attachment": {"type": "silent_turn_reminder"},
                    }),
                    queued_command,
                ),
                2 => {
                    let message = format!(
                        "<task-notification><task-id>fixture-task-{cycle}</task-id><status>completed</status></task-notification>"
                    );
                    (
                        json!({
                            "type": "user",
                            "uuid": queued_command,
                            "parentUuid": hook,
                            "origin": {"kind": "task-notification"},
                            "message": {"content": message},
                        }),
                        queued_command,
                    )
                }
                _ => {
                    let message = format!(
                        "<task-notification><task-id>fixture-task-{cycle}</task-id><status>completed</status></task-notification>"
                    );
                    (
                        json!({
                            "type": "user",
                            "parentUuid": hook,
                            "origin": {"kind": "task-notification"},
                            "message": {"content": message},
                        }),
                        hook,
                    )
                }
            }
        } else {
            (
                json!({
                    "type": "attachment",
                    "uuid": queued_command,
                    "parentUuid": hook,
                    "attachment": {"type": "queued_command", "prompt": ""},
                }),
                queued_command,
            )
        };
        records.push(intermediate_record);
        records.push(json!({
            "type": "assistant",
            "uuid": continuation,
            "parentUuid": continuation_parent,
            "message": {"content": [{"type": "text", "text": "Sample continuation."}]},
        }));
        parent_uuid = Some(continuation);
    }
    records
}
