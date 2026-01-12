mod full;
mod summary;

pub use full::{print_full, print_review_details};
pub use summary::print_summary;

use super::models::{Review, ReviewState};
use regex::Regex;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

// Matches <details> blocks with optional <summary>
static DETAILS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<details[^>]*>(.*?)</details>").unwrap());

static SUMMARY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^\s*<summary[^>]*>(.*?)</summary>").unwrap());

pub struct FormatOptions {
    pub open_details: bool,
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
            if let Some(summary_caps) = SUMMARY_RE.captures(inner) {
                format!("[▶ {}]", &summary_caps[1])
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

/// Print diff hunk with delta syntax highlighting (last 3 lines only)
pub(super) fn print_diff_with_delta(path: &str, diff_hunk: &str) {
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

    // Try to pipe through delta, fall back to plain output on any failure
    let run_delta = || -> std::io::Result<()> {
        let mut child = Command::new("delta")
            .args(["--paging=never", "--line-numbers"])
            .stdin(Stdio::piped())
            .stdout(Stdio::inherit())
            .spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(diff_input.as_bytes())?;
        }

        let status = child.wait()?;
        if !status.success() {
            return Err(std::io::Error::other(format!(
                "delta exited with status: {status}"
            )));
        }
        Ok(())
    };

    if run_delta().is_err() {
        // delta not available or failed, show last 3 lines plain
        for line in last_3_lines {
            println!("{line}");
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
    use crate::gh::check_pr_review::models::Author;
    use rstest::rstest;

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
        assert!(result.contains("2024-06-1")); // day may shift due to timezone
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
        assert!(result.contains("Hello"));
        assert!(result.contains("world"));
    }

    #[rstest]
    fn test_render_markdown_code_block() {
        let result = render_markdown("```rust\nlet x = 1;\n```");
        assert!(result.contains("let x = 1"));
    }

    #[rstest]
    #[case::open(true)]
    #[case::collapsed(false)]
    fn test_process_body_details(#[case] open_details: bool) {
        let options = FormatOptions { open_details };
        let body = "<details><summary>Sum</summary>Content</details>";
        let result = process_body(body, &options);
        if open_details {
            assert!(result.contains("Content") || result.contains("Sum"));
        } else {
            assert!(result.contains("▶"));
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
        assert!(result.contains(label));
        assert!(result.contains(color_code));
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
}
