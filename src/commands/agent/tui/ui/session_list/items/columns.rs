use chrono::{DateTime, Utc};
use unicode_width::UnicodeWidthStr;

/// Pads `s` with trailing spaces up to `width` display columns (no-op if
/// already at or over width). Used for left-aligned fixed-width columns.
pub(super) fn pad_to_width(s: &str, width: usize) -> String {
    let display_width = s.width();
    if display_width >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - display_width))
    }
}

/// Pads `s` with leading spaces up to `width` display columns (no-op if
/// already at or over width). Used for the right-aligned time column.
pub(super) fn pad_left_to_width(s: &str, width: usize) -> String {
    let display_width = s.width();
    if display_width >= width {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(width - display_width))
    }
}

/// Compact relative-time formatter for the fixed-width time column.
/// Deliberately not `worktree_session_children::format_relative_time`
/// (which returns the longer `"{n}m ago"` style) -- this column is too
/// narrow for that and every row must fit the same 9-column budget.
pub(super) fn format_compact_time(dt: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = now.signed_duration_since(dt).num_seconds().max(0);
    if seconds < 60 {
        return "just now".to_string();
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    format!("{}d", hours / 24)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::just_now(0, "just now")]
    #[case::one_minute(60, "1m")]
    #[case::two_hours(7200, "2h")]
    #[case::one_day(86400, "1d")]
    fn test_format_compact_time(#[case] seconds_ago: i64, #[case] expected: &str) {
        let now = Utc::now();
        let dt = now - chrono::Duration::seconds(seconds_ago);
        assert_eq!(format_compact_time(dt, now), expected);
    }
}
