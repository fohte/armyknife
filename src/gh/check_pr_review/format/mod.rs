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

    #[test]
    fn test_collapse_details_with_summary() {
        assert_eq!(
            collapse_details(
                "Before <details><summary>Click me</summary>Hidden content</details> After"
            ),
            "Before [▶ Click me] After"
        );
    }

    #[test]
    fn test_collapse_details_multiline() {
        assert_eq!(
            collapse_details(
                "Text\n<details>\n<summary>Summary</summary>\nLots of\nhidden\nstuff\n</details>\nMore"
            ),
            "Text\n[▶ Summary]\nMore"
        );
    }

    #[test]
    fn test_collapse_details_without_summary() {
        assert_eq!(
            collapse_details("Before <details>Hidden content</details> After"),
            "Before [▶ ...] After"
        );
    }

    #[test]
    fn test_truncate_text() {
        assert_eq!(truncate_text("Hello World", 20), "Hello World");
        assert_eq!(truncate_text("Hello World", 5), "Hello...");
        assert_eq!(truncate_text("  Leading spaces", 10), "Leading sp...");
    }

    #[test]
    fn test_truncate_text_utf8() {
        assert_eq!(truncate_text("日本語テスト", 3), "日本語...");
        assert_eq!(truncate_text("こんにちは世界", 5), "こんにちは...");
        assert_eq!(truncate_text("Hello 世界", 7), "Hello 世...");
        assert_eq!(truncate_text("日本語", 10), "日本語");
    }

    #[test]
    fn test_format_datetime() {
        // Valid ISO 8601 should produce YYYY-MM-DD HH:MM format (converted to local time)
        let result = format_datetime("2024-06-15T12:30:00Z");
        assert!(result.contains("2024-06-15") || result.contains("2024-06-16")); // depends on timezone
        assert_eq!(result.len(), 16); // "YYYY-MM-DD HH:MM"

        // Milliseconds should be handled
        let result = format_datetime("2024-06-15T12:30:00.123Z");
        assert_eq!(result.len(), 16);

        // Invalid format falls back to string manipulation
        assert_eq!(format_datetime("invalid"), "invalid");
    }
}
