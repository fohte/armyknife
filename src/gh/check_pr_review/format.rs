use super::models::{Comment, PrData, Review, ReviewState, ReviewThread};
use super::{CheckPrReviewError, Result};
use regex::Regex;
use std::sync::LazyLock;

static DETAILS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)<details[^>]*>\s*<summary[^>]*>(.*?)</summary>.*?</details>").unwrap()
});

pub struct FormatOptions {
    pub open_details: bool,
}

/// Collapse <details> blocks, keeping only <summary> content
fn collapse_details(text: &str) -> String {
    DETAILS_RE
        .replace_all(text, |caps: &regex::Captures| format!("[▶ {}]", &caps[1]))
        .to_string()
}

fn process_body(body: &str, options: &FormatOptions) -> String {
    if options.open_details {
        body.to_string()
    } else {
        collapse_details(body)
    }
}

fn truncate_text(text: &str, max_length: usize) -> String {
    let first_line = text.lines().next().unwrap_or("").trim_start();
    let char_count = first_line.chars().count();
    if char_count > max_length {
        let truncated: String = first_line.chars().take(max_length).collect();
        format!("{truncated}...")
    } else {
        first_line.to_string()
    }
}

fn format_datetime(iso_date: &str) -> String {
    // Simple formatting: extract date and time parts
    // Input: 2024-01-15T10:30:00Z
    // Output: 2024-01-15 10:30
    iso_date
        .replace('T', " ")
        .replace('Z', "")
        .chars()
        .take(16)
        .collect()
}

fn state_indicator(state: ReviewState) -> &'static str {
    match state {
        ReviewState::Approved => "[approved]",
        ReviewState::ChangesRequested => "[changes requested]",
        ReviewState::Commented => "[commented]",
        ReviewState::Dismissed => "[dismissed]",
        ReviewState::Pending => "[pending]",
    }
}

fn author_login(review: &Review) -> &str {
    review
        .author
        .as_ref()
        .map(|a| a.login.as_str())
        .unwrap_or("unknown")
}

// ============================================================================
// Summary Mode
// ============================================================================

pub fn print_summary(pr_data: &PrData) {
    let sorted_reviews = pr_data.sorted_reviews();

    for (index, review) in sorted_reviews.iter().enumerate() {
        let review_num = index + 1;
        let review_threads = pr_data.threads_for_review(review.database_id);
        let thread_count = review_threads.len();
        let unresolved_count = ReviewThread::count_unresolved(&review_threads);

        let thread_info = if thread_count > 0 {
            format!(" - {unresolved_count}/{thread_count} unresolved")
        } else {
            String::new()
        };

        println!(
            "[{}] @{} ({}){thread_info}",
            review_num,
            author_login(review),
            review.state.as_str()
        );

        if !review.body.is_empty() {
            let body_preview = truncate_text(&review.body, 70);
            println!("    \"{body_preview}\"");
        }

        for thread in &review_threads {
            print_thread_summary(thread);
        }

        println!();
    }

    // Show orphan threads
    let orphan_threads = pr_data.orphan_threads();
    if !orphan_threads.is_empty() {
        println!(
            "Orphan threads (not associated with a review): {}",
            orphan_threads.len()
        );
        for thread in orphan_threads {
            print_thread_summary(thread);
        }
    }
}

fn print_thread_summary(thread: &ReviewThread) {
    if let Some(root) = thread.root_comment() {
        let path = root.path.as_deref().unwrap_or("?");
        let line = root
            .effective_line()
            .map(|l| l.to_string())
            .unwrap_or_else(|| "?".to_string());
        let comment_count = thread.comments.nodes.len();

        let resolved_mark = if thread.is_resolved {
            " [resolved]"
        } else {
            ""
        };

        let body_preview = truncate_text(&root.body, 50);
        println!("    - {path}:{line} ({comment_count} comments){resolved_mark}");
        println!("      @{}: \"{body_preview}\"", root.author_login());
    }
}

// ============================================================================
// Full Mode
// ============================================================================

pub fn print_full(pr_data: &PrData, options: &FormatOptions) {
    let sorted_reviews = pr_data.sorted_reviews();

    for review in sorted_reviews {
        print_review_with_threads(review, pr_data, options);
    }

    // Show orphan threads
    let orphan_threads = pr_data.orphan_threads();
    for thread in orphan_threads {
        print_thread(thread, options);
    }
}

fn print_review_with_threads(review: &Review, pr_data: &PrData, options: &FormatOptions) {
    if !review.body.is_empty() {
        print_review(review, options);
    }

    let review_threads = pr_data.threads_for_review(review.database_id);
    for thread in review_threads {
        print_thread(thread, options);
    }
}

fn print_review(review: &Review, options: &FormatOptions) {
    let formatted_date = format_datetime(&review.created_at);
    println!(
        "@{} ({}) {}",
        author_login(review),
        formatted_date,
        state_indicator(review.state)
    );

    let body = process_body(&review.body, options);
    println!("{body}");
    println!();
}

fn print_thread(thread: &ReviewThread, options: &FormatOptions) {
    if let Some(root) = thread.root_comment() {
        print_comment(root, "", false, thread.is_resolved, options);

        for reply in thread.replies() {
            print_comment(reply, "  ", true, thread.is_resolved, options);
        }
    }
}

fn print_comment(
    comment: &Comment,
    indent: &str,
    is_reply: bool,
    is_resolved: bool,
    options: &FormatOptions,
) {
    let formatted_date = format_datetime(&comment.created_at);

    if is_reply {
        println!("{indent}└─ @{} ({formatted_date})", comment.author_login());
    } else {
        let resolved_indicator = if is_resolved { " [resolved]" } else { "" };
        println!(
            "@{} ({formatted_date}){resolved_indicator}",
            comment.author_login()
        );
    }

    // Print diff context (only for root comments)
    if !is_reply {
        if let Some(diff_hunk) = &comment.diff_hunk {
            let path = comment.path.as_deref().unwrap_or("?");
            println!("File: {path}");
            // Print last 3 lines of diff hunk for context
            let lines: Vec<&str> = diff_hunk.lines().collect();
            let start = lines.len().saturating_sub(3);
            for line in &lines[start..] {
                println!("{line}");
            }
        }
    }

    let body = process_body(&comment.body, options);
    if is_reply {
        for line in body.lines() {
            println!("{indent}   {line}");
        }
    } else {
        println!("{body}");
    }
    println!();
}

// ============================================================================
// Review Details Mode
// ============================================================================

pub fn print_review_details(
    pr_data: &PrData,
    review_num: usize,
    options: &FormatOptions,
) -> Result<()> {
    if review_num == 0 {
        return Err(CheckPrReviewError::ReviewNotFound(review_num));
    }

    let sorted_reviews = pr_data.sorted_reviews();

    let review = sorted_reviews
        .get(review_num - 1)
        .ok_or(CheckPrReviewError::ReviewNotFound(review_num))?;

    print_review_with_threads(review, pr_data, options);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collapse_details() {
        let input = "Before <details><summary>Click me</summary>Hidden content</details> After";
        let expected = "Before [▶ Click me] After";
        assert_eq!(collapse_details(input), expected);
    }

    #[test]
    fn test_collapse_details_multiline() {
        let input =
            "Text\n<details>\n<summary>Summary</summary>\nLots of\nhidden\nstuff\n</details>\nMore";
        let result = collapse_details(input);
        assert!(result.contains("[▶ Summary]"));
        assert!(!result.contains("hidden"));
    }

    #[test]
    fn test_truncate_text() {
        assert_eq!(truncate_text("Hello World", 20), "Hello World");
        assert_eq!(truncate_text("Hello World", 5), "Hello...");
        assert_eq!(truncate_text("  Leading spaces", 10), "Leading sp...");
    }

    #[test]
    fn test_truncate_text_utf8() {
        // Should not panic on multibyte characters
        assert_eq!(truncate_text("日本語テスト", 3), "日本語...");
        assert_eq!(truncate_text("こんにちは世界", 5), "こんにちは...");
        assert_eq!(truncate_text("Hello 世界", 7), "Hello 世...");
        // Full string when under limit
        assert_eq!(truncate_text("日本語", 10), "日本語");
    }

    #[test]
    fn test_format_datetime() {
        assert_eq!(format_datetime("2024-01-15T10:30:00Z"), "2024-01-15 10:30");
        assert_eq!(
            format_datetime("2024-12-31T23:59:59.123Z"),
            "2024-12-31 23:59"
        );
    }
}
