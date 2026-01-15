use chrono::{DateTime, Utc};

/// Formats an ISO 8601 timestamp as a human-readable relative time string.
///
/// Returns strings like "just now", "5 minutes ago", "3 hours ago", "2 days ago", "1 weeks ago".
///
/// # Arguments
/// * `timestamp` - An ISO 8601 formatted timestamp string (e.g., "2024-01-15T10:30:00Z")
///
/// # Returns
/// A human-readable relative time string, or the original timestamp if parsing fails.
#[allow(dead_code)]
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
        format!("{} minutes ago", seconds / 60)
    } else if seconds < 86400 {
        format!("{} hours ago", seconds / 3600)
    } else if seconds < 604800 {
        format!("{} days ago", seconds / 86400)
    } else {
        format!("{} weeks ago", seconds / 604800)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn timestamp_ago(duration: Duration) -> String {
        (Utc::now() - duration).to_rfc3339()
    }

    #[test]
    fn test_just_now() {
        let ts = timestamp_ago(Duration::seconds(30));
        assert_eq!(format_relative_time(&ts), "just now");
    }

    #[test]
    fn test_minutes_ago() {
        let ts = timestamp_ago(Duration::minutes(5));
        assert_eq!(format_relative_time(&ts), "5 minutes ago");

        let ts = timestamp_ago(Duration::minutes(59));
        assert_eq!(format_relative_time(&ts), "59 minutes ago");
    }

    #[test]
    fn test_hours_ago() {
        let ts = timestamp_ago(Duration::hours(1));
        assert_eq!(format_relative_time(&ts), "1 hours ago");

        let ts = timestamp_ago(Duration::hours(23));
        assert_eq!(format_relative_time(&ts), "23 hours ago");
    }

    #[test]
    fn test_days_ago() {
        let ts = timestamp_ago(Duration::days(1));
        assert_eq!(format_relative_time(&ts), "1 days ago");

        let ts = timestamp_ago(Duration::days(6));
        assert_eq!(format_relative_time(&ts), "6 days ago");
    }

    #[test]
    fn test_weeks_ago() {
        let ts = timestamp_ago(Duration::weeks(1));
        assert_eq!(format_relative_time(&ts), "1 weeks ago");

        let ts = timestamp_ago(Duration::weeks(4));
        assert_eq!(format_relative_time(&ts), "4 weeks ago");
    }

    #[test]
    fn test_invalid_timestamp() {
        assert_eq!(format_relative_time("invalid"), "invalid");
        assert_eq!(format_relative_time("not-a-date"), "not-a-date");
    }

    #[test]
    fn test_future_timestamp() {
        let ts = (Utc::now() + Duration::hours(1)).to_rfc3339();
        // Future timestamps return the original string
        assert_eq!(format_relative_time(&ts), ts);
    }
}
