use chrono::{DateTime, Utc};

/// Formats an ISO 8601 timestamp as a human-readable relative time string.
///
/// Returns strings like "just now", "5 minutes ago", "3 hours ago", "2 days ago", "1 week ago".
///
/// # Arguments
/// * `timestamp` - An ISO 8601 formatted timestamp string (e.g., "2024-01-15T10:30:00Z")
///
/// # Returns
/// A human-readable relative time string, or the original timestamp if parsing fails.
pub fn format_relative_time(timestamp: &str) -> String {
    let parsed: DateTime<Utc> = match timestamp.parse() {
        Ok(dt) => dt,
        Err(_) => return timestamp.to_string(),
    };

    let now = Utc::now();
    let diff = now.signed_duration_since(parsed);
    let seconds = diff.num_seconds();

    if seconds < 0 {
        return timestamp.to_string();
    }

    if seconds < 60 {
        "just now".to_string()
    } else if seconds < 3600 {
        let minutes = seconds / 60;
        format!(
            "{} minute{} ago",
            minutes,
            if minutes == 1 { "" } else { "s" }
        )
    } else if seconds < 86400 {
        let hours = seconds / 3600;
        format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
    } else if seconds < 604800 {
        let days = seconds / 86400;
        format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
    } else {
        let weeks = seconds / 604800;
        format!("{} week{} ago", weeks, if weeks == 1 { "" } else { "s" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use rstest::rstest;

    fn timestamp_ago(duration: Duration) -> String {
        (Utc::now() - duration).to_rfc3339()
    }

    #[rstest]
    #[case::just_now(Duration::seconds(30), "just now")]
    #[case::one_minute(Duration::minutes(1), "1 minute ago")]
    #[case::five_minutes(Duration::minutes(5), "5 minutes ago")]
    #[case::fifty_nine_minutes(Duration::minutes(59), "59 minutes ago")]
    #[case::one_hour(Duration::hours(1), "1 hour ago")]
    #[case::twenty_three_hours(Duration::hours(23), "23 hours ago")]
    #[case::one_day(Duration::days(1), "1 day ago")]
    #[case::six_days(Duration::days(6), "6 days ago")]
    #[case::one_week(Duration::weeks(1), "1 week ago")]
    #[case::four_weeks(Duration::weeks(4), "4 weeks ago")]
    fn test_relative_time(#[case] duration: Duration, #[case] expected: &str) {
        let ts = timestamp_ago(duration);
        assert_eq!(format_relative_time(&ts), expected);
    }

    #[rstest]
    #[case::invalid("invalid")]
    #[case::not_a_date("not-a-date")]
    fn test_invalid_timestamp(#[case] input: &str) {
        assert_eq!(format_relative_time(input), input);
    }

    #[test]
    fn test_future_timestamp() {
        let ts = (Utc::now() + Duration::hours(1)).to_rfc3339();
        // Future timestamps return the original string
        assert_eq!(format_relative_time(&ts), ts);
    }
}
