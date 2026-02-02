mod full;
mod summary;

pub use full::{print_full, print_review_details};
pub use summary::print_summary;

#[cfg(test)]
pub use full::format_review_details;
#[cfg(test)]
pub use summary::format_summary;

use super::models::{Review, ReviewState};
use lazy_regex::{Lazy, Regex, lazy_regex, regex_captures};
use std::io::Write;
use std::process::{Command, Stdio};

static DETAILS_RE: Lazy<Regex> = lazy_regex!(r"(?s)<details[^>]*>(.*?)</details>");

#[derive(Default)]
pub struct FormatOptions {
    pub open_details: bool,
    /// Skip delta rendering and return plain diff lines instead
    pub skip_delta: bool,
}

// ANSI color codes matching the original script
pub(super) mod color {
    pub const RESET: &str = "\x1b[0m";
    pub const DIM: &str = "\x1b[2m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const RED: &str = "\x1b[31m";
    pub const BG_GRAY: &str = "\x1b[48;5;238m";
}

fn collapse_details(text: &str) -> String {
    DETAILS_RE
        .replace_all(text, |caps: &regex::Captures| {
            let inner = &caps[1];
            if let Some((_, summary)) =
                regex_captures!(r"(?s)^\s*<summary[^>]*>(.*?)</summary>", inner)
            {
                format!("[▶ {summary}]")
            } else {
                "[▶ ...]".to_string()
            }
        })
        .to_string()
}

pub(super) fn process_body(body: &str, options: &FormatOptions) -> String {
    let text = if options.open_details {
        body.to_string()
    } else {
        collapse_details(body)
    };
    render_markdown(&text)
}

/// Render markdown text for terminal display using termimad
pub(super) fn render_markdown(text: &str) -> String {
    use termimad::MadSkin;
    use termimad::crossterm::style::{Attribute, Color};

    if text.trim().is_empty() {
        return String::new();
    }

    let mut skin = MadSkin::default_dark();

    // Headers: bold white (like glow)
    for header in &mut skin.headers {
        header.compound_style.remove_attr(Attribute::Underlined);
        header.compound_style.set_fg(Color::White);
    }

    // Inline code: match glow's dark.json (color: 203, background: 236)
    skin.inline_code
        .set_fgbg(Color::AnsiValue(203), Color::AnsiValue(236));

    // Code block: match glow's chroma text color #C4C4C4 ≈ 251, background #373737 ≈ 237
    skin.code_block
        .set_fgbg(Color::AnsiValue(251), Color::AnsiValue(237));

    // Left margin (2 spaces like glow)
    skin.paragraph.left_margin = 2;
    for header in &mut skin.headers {
        header.left_margin = 2;
    }
    skin.code_block.left_margin = 4;

    skin.term_text(text).to_string()
}

/// Format diff hunk with delta syntax highlighting (last 3 lines only)
/// When skip_delta is true, returns plain text without running delta command.
pub(super) fn format_diff_with_delta(path: &str, diff_hunk: &str, skip_delta: bool) -> String {
    let lines: Vec<&str> = diff_hunk.lines().collect();

    // Extract hunk header (first line if it starts with @@)
    let hunk_header = lines.first().filter(|l| l.starts_with("@@")).copied();

    // Get content lines (excluding hunk header if present)
    let content_start = if hunk_header.is_some() { 1 } else { 0 };
    let content_lines = &lines[content_start..];

    // Only show last 3 lines of content (matching original script's `tail -n 3`)
    let last_3_start = content_lines.len().saturating_sub(3);
    let last_3_lines = &content_lines[last_3_start..];

    // Build diff format that delta expects
    let mut diff_input = String::new();
    diff_input.push_str(&format!("diff --git a/{path} b/{path}\n"));
    diff_input.push_str(&format!("--- a/{path}\n"));
    diff_input.push_str(&format!("+++ b/{path}\n"));
    diff_input.push_str(hunk_header.unwrap_or("@@ -1,1 +1,1 @@"));
    diff_input.push('\n');
    for line in last_3_lines {
        diff_input.push_str(line);
        diff_input.push('\n');
    }

    if skip_delta {
        // Return plain diff lines for testing
        let mut output = String::new();
        for line in last_3_lines {
            output.push_str(line);
            output.push('\n');
        }
        return output;
    }

    // Try to pipe through delta, fall back to plain output on any failure
    let run_delta = || -> std::io::Result<String> {
        let mut child = Command::new("delta")
            .args(["--paging=never", "--line-numbers"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(diff_input.as_bytes())?;
        }

        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(std::io::Error::other(format!(
                "delta exited with status: {}",
                output.status
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    };

    match run_delta() {
        Ok(output) => output,
        Err(_) => {
            // delta not available or failed, show last 3 lines plain
            let mut output = String::new();
            for line in last_3_lines {
                output.push_str(line);
                output.push('\n');
            }
            output
        }
    }
}

pub(super) fn truncate_text(text: &str, max_length: usize) -> String {
    let first_line = text.lines().next().unwrap_or("").trim_start();
    let char_count = first_line.chars().count();
    if char_count > max_length {
        let truncated: String = first_line.chars().take(max_length).collect();
        format!("{truncated}...")
    } else {
        first_line.to_string()
    }
}

pub(super) fn format_datetime(iso_date: &str) -> String {
    use chrono::{DateTime, Local, Utc};

    // Try to parse as ISO 8601 and convert to local time
    if let Ok(utc) = iso_date.parse::<DateTime<Utc>>() {
        let local: DateTime<Local> = utc.into();
        return local.format("%Y-%m-%d %H:%M").to_string();
    }

    // Fallback: simple string manipulation
    iso_date
        .replace('T', " ")
        .replace('Z', "")
        .chars()
        .take(16)
        .collect()
}

pub(super) fn state_indicator(state: ReviewState) -> String {
    use color::*;
    match state {
        ReviewState::Approved => format!("{GREEN}[approved]{RESET}"),
        ReviewState::ChangesRequested => format!("{RED}[changes requested]{RESET}"),
        ReviewState::Commented => format!("{YELLOW}[commented]{RESET}"),
        ReviewState::Dismissed => format!("{DIM}[dismissed]{RESET}"),
        ReviewState::Pending => format!("{DIM}[pending]{RESET}"),
    }
}

pub(super) fn author_login(review: &Review) -> &str {
    review
        .author
        .as_ref()
        .map(|a| a.login.as_str())
        .unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::gh::check_pr_review::models::Author;
    use rstest::rstest;

    mod test_helpers {
        use crate::commands::gh::check_pr_review::models::{
            Author, Comment, CommentsNode, PrData, PullRequestReview, ReplyTo, Review, ReviewState,
            ReviewThread,
        };

        pub fn make_comment(
            id: i64,
            author: &str,
            body: &str,
            review_id: Option<i64>,
            is_reply: bool,
        ) -> Comment {
            Comment {
                database_id: id,
                author: Some(Author {
                    login: author.to_string(),
                }),
                body: body.to_string(),
                created_at: "2024-01-15T10:30:00Z".to_string(),
                path: Some("src/main.rs".to_string()),
                line: Some(42),
                original_line: None,
                diff_hunk: Some("@@ -40,3 +40,5 @@\n context\n-old line\n+new line".to_string()),
                reply_to: if is_reply { Some(ReplyTo {}) } else { None },
                pull_request_review: review_id.map(|id| PullRequestReview { database_id: id }),
            }
        }

        pub fn make_thread(comments: Vec<Comment>, is_resolved: bool) -> ReviewThread {
            ReviewThread {
                is_resolved,
                comments: CommentsNode { nodes: comments },
            }
        }

        pub fn make_review(id: i64, author: &str, body: &str, state: ReviewState) -> Review {
            Review {
                database_id: id,
                author: Some(Author {
                    login: author.to_string(),
                }),
                body: body.to_string(),
                state,
                created_at: "2024-01-15T10:00:00Z".to_string(),
            }
        }

        pub fn make_pr_data(reviews: Vec<Review>, threads: Vec<ReviewThread>) -> PrData {
            PrData { reviews, threads }
        }
    }

    #[rstest]
    #[case::with_summary(
        "Before <details><summary>Click me</summary>Hidden content</details> After",
        "Before [▶ Click me] After"
    )]
    #[case::multiline(
        "Text\n<details>\n<summary>Summary</summary>\nLots of\nhidden\nstuff\n</details>\nMore",
        "Text\n[▶ Summary]\nMore"
    )]
    #[case::without_summary(
        "Before <details>Hidden content</details> After",
        "Before [▶ ...] After"
    )]
    fn test_collapse_details(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(collapse_details(input), expected);
    }

    #[rstest]
    #[case::no_truncation("Hello World", 20, "Hello World")]
    #[case::truncate("Hello World", 5, "Hello...")]
    #[case::leading_spaces("  Leading spaces", 10, "Leading sp...")]
    #[case::utf8_japanese("日本語テスト", 3, "日本語...")]
    #[case::utf8_mixed("Hello 世界", 7, "Hello 世...")]
    #[case::utf8_no_truncation("日本語", 10, "日本語")]
    fn test_truncate_text(#[case] input: &str, #[case] max_len: usize, #[case] expected: &str) {
        assert_eq!(truncate_text(input, max_len), expected);
    }

    #[rstest]
    #[case::valid_iso("2024-06-15T12:30:00Z")]
    #[case::with_millis("2024-06-15T12:30:00.123Z")]
    fn test_format_datetime_valid(#[case] input: &str) {
        let result = format_datetime(input);
        assert_eq!(result.len(), 16); // "YYYY-MM-DD HH:MM"
        assert!(
            result.contains("2024-06-1"),
            "expected to contain '2024-06-1', got: {}",
            result
        ); // day may shift due to timezone
    }

    #[rstest]
    fn test_format_datetime_invalid_fallback() {
        assert_eq!(format_datetime("invalid"), "invalid");
    }

    #[rstest]
    #[case::empty("", "")]
    #[case::whitespace("   ", "")]
    fn test_render_markdown_empty(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(render_markdown(input), expected);
    }

    #[rstest]
    fn test_render_markdown_simple() {
        let result = render_markdown("Hello **world**");
        assert!(
            result.contains("Hello"),
            "expected to contain 'Hello', got: {}",
            result
        );
        assert!(
            result.contains("world"),
            "expected to contain 'world', got: {}",
            result
        );
    }

    #[rstest]
    fn test_render_markdown_code_block() {
        let result = render_markdown("```rust\nlet x = 1;\n```");
        assert!(
            result.contains("let x = 1"),
            "expected to contain 'let x = 1', got: {}",
            result
        );
    }

    #[rstest]
    #[case::open(true)]
    #[case::collapsed(false)]
    fn test_process_body_details(#[case] open_details: bool) {
        let options = FormatOptions {
            open_details,
            skip_delta: true,
        };
        let body = "<details><summary>Sum</summary>Content</details>";
        let result = process_body(body, &options);
        if open_details {
            assert!(
                result.contains("Content") || result.contains("Sum"),
                "expected to contain 'Content' or 'Sum', got: {}",
                result
            );
        } else {
            assert!(
                result.contains("▶"),
                "expected to contain '▶', got: {}",
                result
            );
        }
    }

    #[rstest]
    #[case::approved(ReviewState::Approved, "[approved]", color::GREEN)]
    #[case::changes_requested(ReviewState::ChangesRequested, "[changes requested]", color::RED)]
    #[case::commented(ReviewState::Commented, "[commented]", color::YELLOW)]
    #[case::dismissed(ReviewState::Dismissed, "[dismissed]", color::DIM)]
    #[case::pending(ReviewState::Pending, "[pending]", color::DIM)]
    fn test_state_indicator(
        #[case] state: ReviewState,
        #[case] label: &str,
        #[case] color_code: &str,
    ) {
        let result = state_indicator(state);
        assert!(
            result.contains(label),
            "expected to contain '{}', got: {}",
            label,
            result
        );
        assert!(
            result.contains(color_code),
            "expected to contain '{}', got: {}",
            color_code,
            result
        );
    }

    #[rstest]
    #[case::with_author(Some(Author { login: "testuser".to_string() }), "testuser")]
    #[case::without_author(None, "unknown")]
    fn test_author_login(#[case] author: Option<Author>, #[case] expected: &str) {
        let review = Review {
            database_id: 1,
            author,
            body: String::new(),
            state: ReviewState::Approved,
            created_at: String::new(),
        };
        assert_eq!(author_login(&review), expected);
    }

    // Integration tests for format output
    mod integration {
        use super::test_helpers::*;
        use crate::commands::gh::check_pr_review::format::{
            FormatOptions, format_review_details, format_summary,
        };
        use crate::commands::gh::check_pr_review::models::{PrData, ReviewState};
        use indoc::indoc;
        use rstest::rstest;

        fn test_options() -> FormatOptions {
            FormatOptions {
                open_details: false,
                skip_delta: true,
            }
        }

        fn build_summary_single_review_with_body() -> PrData {
            make_pr_data(
                vec![make_review(
                    100,
                    "reviewer1",
                    "LGTM!",
                    ReviewState::Approved,
                )],
                vec![],
            )
        }

        fn build_summary_with_threads() -> PrData {
            make_pr_data(
                vec![make_review(
                    100,
                    "reviewer1",
                    "",
                    ReviewState::ChangesRequested,
                )],
                vec![
                    make_thread(
                        vec![make_comment(1, "reviewer1", "Fix this", Some(100), false)],
                        false,
                    ),
                    make_thread(
                        vec![make_comment(2, "reviewer1", "Also this", Some(100), false)],
                        true,
                    ),
                ],
            )
        }

        fn build_summary_multiple_reviews() -> PrData {
            make_pr_data(
                vec![
                    make_review(100, "alice", "", ReviewState::Commented),
                    make_review(200, "bob", "Looks good", ReviewState::Approved),
                ],
                vec![],
            )
        }

        fn build_summary_orphan_threads() -> PrData {
            make_pr_data(
                vec![make_review(100, "alice", "", ReviewState::Approved)],
                vec![
                    make_thread(
                        vec![make_comment(1, "alice", "Normal thread", Some(100), false)],
                        false,
                    ),
                    make_thread(
                        vec![make_comment(2, "bob", "Orphan comment", Some(999), false)],
                        false,
                    ),
                ],
            )
        }

        #[rstest]
        #[case::single_review_with_body(
            build_summary_single_review_with_body(),
            indoc! {r#"
                [1] @reviewer1 (approved)
                    "LGTM!"

            "#}
        )]
        #[case::with_threads_unresolved_count(
            build_summary_with_threads(),
            indoc! {r#"
                [1] @reviewer1 (changes_requested) - 1/2 unresolved
                    - src/main.rs:42 (1 comments)
                      @reviewer1: "Fix this"
                    - src/main.rs:42 (1 comments) [resolved]
                      @reviewer1: "Also this"

            "#}
        )]
        #[case::multiple_reviews(
            build_summary_multiple_reviews(),
            indoc! {r#"
                [1] @alice (commented)

                [2] @bob (approved)
                    "Looks good"

            "#}
        )]
        #[case::orphan_threads(
            build_summary_orphan_threads(),
            indoc! {r#"
                [1] @alice (approved) - 1/1 unresolved
                    - src/main.rs:42 (1 comments)
                      @alice: "Normal thread"

                Orphan threads (not associated with a review): 1
                    - src/main.rs:42 (1 comments)
                      @bob: "Orphan comment"
            "#}
        )]
        fn test_format_summary(#[case] pr_data: PrData, #[case] expected: &str) {
            assert_eq!(format_summary(&pr_data), expected);
        }

        #[rstest]
        #[case::review_not_found_zero(0)]
        #[case::review_not_found_out_of_range(2)]
        fn test_format_review_details_not_found(#[case] review_num: usize) {
            let pr_data = make_pr_data(
                vec![make_review(100, "alice", "", ReviewState::Approved)],
                vec![],
            );
            assert!(format_review_details(&pr_data, review_num, &test_options()).is_err());
        }
    }
}
