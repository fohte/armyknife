use serde_json::Value;

use super::{EventKind, MessageKind, is_failure, one_line};

pub(super) fn classify(record: &Value, raw_text: &str) -> MessageKind {
    if record.get("subtype").and_then(Value::as_str).is_none() && raw_text.trim().is_empty() {
        return MessageKind::Hidden;
    }
    let subtype = record
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or("system event");
    let hook_errors = record
        .get("hookErrors")
        .and_then(Value::as_array)
        .filter(|errors| !errors.is_empty());
    let continuation_prevented =
        record.get("preventedContinuation").and_then(Value::as_bool) == Some(true);
    if subtype == "stop_hook_summary" && hook_errors.is_none() && !continuation_prevented {
        return MessageKind::Hidden;
    }

    let kind = if subtype == "compact_boundary" {
        EventKind::CompactBoundary
    } else {
        EventKind::System
    };
    let default_text = subtype.replace('_', " ");
    let text = match subtype {
        "compact_boundary" => "Context compacted".to_owned(),
        "resume" | "session_resume" => "Session resumed".to_owned(),
        "model_change" | "model_changed" => "Model changed".to_owned(),
        "away_summary" => record
            .get("content")
            .and_then(Value::as_str)
            .filter(|content| !content.trim().is_empty())
            .or_else(|| (!raw_text.trim().is_empty()).then_some(raw_text))
            .map(|content| format!("recap: {}", one_line(content)))
            .unwrap_or_else(|| default_text.clone()),
        "turn_duration" => turn_duration_text(record),
        "stop_hook_summary" => hook_errors.map_or_else(
            || "Stop hook summary: continuation was prevented".to_owned(),
            |errors| stop_hook_error_text(errors),
        ),
        _ if !raw_text.trim().is_empty() => one_line(raw_text),
        _ => default_text.clone(),
    };
    let failure_text = if subtype == "stop_hook_summary" {
        if raw_text.trim().is_empty() {
            &default_text
        } else {
            raw_text
        }
    } else {
        &text
    };
    MessageKind::Event {
        kind,
        failed: is_failure(record, subtype, failure_text),
        text,
    }
}

fn stop_hook_error_text(errors: &[Value]) -> String {
    let details = errors
        .iter()
        .filter_map(|error| {
            let name = error
                .get("hookName")
                .or_else(|| error.get("name"))
                .and_then(Value::as_str);
            let message = error
                .get("error")
                .or_else(|| error.get("message"))
                .and_then(Value::as_str);
            match (name, message) {
                (Some(name), Some(message)) => Some(format!("{name}: {message}")),
                (Some(name), None) => Some(name.to_owned()),
                (None, Some(message)) => Some(message.to_owned()),
                (None, None) => None,
            }
        })
        .collect::<Vec<_>>();
    if details.is_empty() {
        format!("Stop hook summary: {} hook error(s)", errors.len())
    } else {
        format!("Stop hook summary: {}", details.join("; "))
    }
}

fn turn_duration_text(record: &Value) -> String {
    let duration = record
        .get("durationMs")
        .and_then(Value::as_u64)
        .map(format_duration)
        .map(|duration| format!("Took {duration}"));
    let ended_at = record
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|timestamp| chrono::DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| {
            format!(
                "done {}",
                timestamp.with_timezone(&chrono::Local).format("%H:%M")
            )
        });
    [duration, ended_at]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ")
}

fn format_duration(duration_ms: u64) -> String {
    let seconds = duration_ms / 1000;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    let hours = minutes / 60;
    let minutes = minutes % 60;
    match (hours, minutes) {
        (0, 0) => format!("{seconds}s"),
        (0, _) => format!("{minutes}m {seconds}s"),
        _ => format!("{hours}h {minutes}m"),
    }
}
