//! Pre-processing that strips noise from PR review/comment bodies.
//!
//! Bot reviewers (Devin, Gemini Code Assist, Copilot, etc.) embed:
//! - Hidden tracking metadata in HTML comments (`<!-- devin-review-comment {...} -->`).
//! - Visual badges via `<picture>` blocks that render as broken-looking
//!   markup in terminals and add no signal for an LLM agent.
//! - `<details>` collapsibles that hold their longest content
//!   (prompts-for-agents, full-file references) far from the actual review point.
//!
//! All three waste context for both human readers and LLM agents pulling the
//! review file. This module returns a cleaned body suitable for terminal
//! rendering (`check`) and for inclusion in the Markdown threads file
//! (`reply pull`).

use lazy_regex::{Lazy, Regex, lazy_regex, regex_captures};

static DETAILS_RE: Lazy<Regex> = lazy_regex!(r"(?si)<details[^>]*>(.*?)</details>");
static HTML_COMMENT_RE: Lazy<Regex> = lazy_regex!(r"(?s)<!--.*?-->");
static PICTURE_RE: Lazy<Regex> = lazy_regex!(r"(?si)<picture[^>]*>.*?</picture>");

/// Strip noise (HTML comments, `<picture>` badges) and collapse `<details>`
/// blocks to one-line markers. Use for review/comment bodies before display
/// or serialization.
pub fn clean_review_body(text: &str) -> String {
    let stripped = strip_noise(text);
    collapse_details(&stripped)
}

/// Like [`clean_review_body`] but leaves `<details>` blocks intact (still
/// strips HTML comments and `<picture>` badges). Used when the caller has
/// opted into expanding details (`-d/--open-details`).
pub fn strip_noise_keep_details(text: &str) -> String {
    strip_noise(text)
}

fn strip_noise(text: &str) -> String {
    let no_comments = HTML_COMMENT_RE.replace_all(text, "");
    PICTURE_RE.replace_all(&no_comments, "").to_string()
}

fn collapse_details(text: &str) -> String {
    DETAILS_RE
        .replace_all(text, |caps: &regex::Captures| {
            let inner = &caps[1];
            if let Some((_, summary)) =
                regex_captures!(r"(?si)^\s*<summary[^>]*>(.*?)</summary>", inner)
            {
                format!("[▶ {}]", summary.trim())
            } else {
                "[▶ ...]".to_string()
            }
        })
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    #[rstest]
    #[case::with_summary(
        "Before <details><summary>Click me</summary>Hidden content</details> After",
        "Before [▶ Click me] After"
    )]
    #[case::multiline(
        indoc! {"
            Text
            <details>
            <summary>Summary</summary>
            Lots of
            hidden
            stuff
            </details>
            More"},
        indoc! {"
            Text
            [▶ Summary]
            More"},
    )]
    #[case::without_summary(
        "Before <details>Hidden content</details> After",
        "Before [▶ ...] After"
    )]
    #[case::uppercase_tag(
        "Before <DETAILS><SUMMARY>Click</SUMMARY>Hidden</DETAILS> After",
        "Before [▶ Click] After"
    )]
    #[case::summary_with_whitespace(
        "<details><summary>  spaced  </summary>body</details>",
        "[▶ spaced]"
    )]
    #[case::strips_html_comment(
        r#"Before <!-- devin-review-comment {"id": "BUG_x"} --> After"#,
        "Before  After"
    )]
    #[case::strips_multiline_html_comment(
        indoc! {r#"
            Before
            <!-- devin-review-badge-begin -->
            <a href="x"><img src="y"></a>
            <!-- devin-review-badge-end -->
            After"#},
        indoc! {r#"
            Before

            <a href="x"><img src="y"></a>

            After"#}
    )]
    #[case::strips_picture_badge(
        indoc! {r##"
            Header
            <picture>
              <source media="(prefers-color-scheme: dark)" srcset="dark.svg">
              <img src="light.svg" alt="badge">
            </picture>
            Body"##},
        indoc! {"
            Header

            Body"}
    )]
    #[case::strips_inline_picture(
        "Before <picture><img src=\"x\"></picture> After",
        "Before  After"
    )]
    fn test_clean_review_body(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(clean_review_body(input), expected);
    }

    #[rstest]
    fn test_strip_noise_keep_details_preserves_details_block() {
        // HTML comments and `<picture>` are stripped, but the `<details>`
        // block is left intact so the caller (with `--open-details`) can
        // render or reveal it.
        let input = indoc! {"
            <!-- meta -->
            <details><summary>S</summary>Body</details>
            <picture><img></picture>
            Tail"};
        let expected = indoc! {"

            <details><summary>S</summary>Body</details>

            Tail"};
        assert_eq!(strip_noise_keep_details(input), expected);
    }
}
