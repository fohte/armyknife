use lazy_regex::regex_captures;

use super::serializer::ThreadsFrontmatter;
use crate::commands::gh::pr_review::error::PrReviewError;

pub struct MarkdownParser;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedThreadsFile {
    pub frontmatter: ThreadsFrontmatter,
    pub threads: Vec<ParsedThread>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedThread {
    pub thread_id: String,
    pub path: String,
    pub line: Option<u64>,
    pub resolve: bool,
    pub draft_reply: Option<String>,
}

impl MarkdownParser {
    /// Parse threads.md content into structured data.
    pub fn parse(content: &str) -> Result<ParsedThreadsFile, PrReviewError> {
        let (frontmatter, body) = parse_frontmatter(content)?;
        let threads = parse_threads(body)?;

        Ok(ParsedThreadsFile {
            frontmatter,
            threads,
        })
    }
}

/// Find the closing `---` delimiter within frontmatter content.
///
/// Scans line-by-line to avoid embedding `\n` in string literals, which triggers
/// the `prefer-indoc` lint even when the string is a search pattern rather than
/// a display string.
///
/// Returns `(yaml_end, body_start)` where `yaml_end` is the byte offset up to
/// (but not including) the `---` line, and `body_start` is the byte offset of
/// the first character after the closing delimiter line.
fn find_frontmatter_close(content: &str) -> Option<(usize, usize)> {
    let mut pos = 0;
    for line in content.split('\n') {
        let bare = line.strip_suffix('\r').unwrap_or(line);
        if bare == "---" {
            // `pos` points to the start of the "---" line (including any preceding
            // newline that was consumed by the split). The yaml content ends at the
            // newline before this line, i.e. `pos.saturating_sub(1)` if pos > 0,
            // but it's cleaner to let the caller slice `..pos` and strip trailing
            // whitespace via serde_yaml.
            //
            // body_start advances past: current pos + "---" (or "---\r") + '\n'
            let body_start = pos + line.len() + 1;
            return Some((pos, body_start));
        }
        // Advance past this line plus its '\n' separator.
        pos += line.len() + 1;
    }
    None
}

fn parse_frontmatter(content: &str) -> Result<(ThreadsFrontmatter, &str), PrReviewError> {
    let content = content.trim_start();

    if !content.starts_with("---\n") && !content.starts_with("---\r\n") {
        return Err(PrReviewError::FrontmatterParseError {
            details: "missing opening ---".to_string(),
        });
    }

    // Find the closing ---
    let after_open = &content[4..]; // skip "---\n"
    let (yaml_end, body_start) =
        find_frontmatter_close(after_open).ok_or_else(|| PrReviewError::FrontmatterParseError {
            details: "missing closing ---".to_string(),
        })?;

    let yaml_str = &after_open[..yaml_end];
    let body = if body_start <= after_open.len() {
        &after_open[body_start..]
    } else {
        ""
    };

    let frontmatter: ThreadsFrontmatter =
        serde_yaml::from_str(yaml_str).map_err(|e| PrReviewError::FrontmatterParseError {
            details: e.to_string(),
        })?;

    Ok((frontmatter, body))
}

fn parse_threads(body: &str) -> Result<Vec<ParsedThread>, PrReviewError> {
    let mut threads = Vec::new();

    // Split by thread headers
    let parts: Vec<&str> = body.split("<!-- thread: ").collect();

    // Skip the first part (before any thread header)
    for part in parts.iter().skip(1) {
        let thread = parse_single_thread(part)?;
        threads.push(thread);
    }

    Ok(threads)
}

fn parse_single_thread(part: &str) -> Result<ParsedThread, PrReviewError> {
    // Extract thread_id and path from the header line
    // Format: {thread_id} path: {file_path}:{line} -->
    let header_end = part
        .find("-->")
        .ok_or_else(|| PrReviewError::ThreadParseError {
            line: 0,
            details: "unclosed thread header comment".to_string(),
        })?;

    let header = &part[..header_end].trim();

    let (thread_id, path, line) = parse_thread_header(header)?;

    // Rest of the thread content after the header line
    let rest = &part[header_end + 3..]; // skip "-->"
    let rest = rest.strip_prefix('\n').unwrap_or(rest);

    // Parse resolve checkbox
    let resolve = parse_resolve_checkbox(rest);

    // Parse draft reply (text after the last <!-- /comment -->)
    let draft_reply = extract_draft_reply(rest);

    Ok(ParsedThread {
        thread_id,
        path,
        line,
        resolve,
        draft_reply,
    })
}

fn parse_thread_header(header: &str) -> Result<(String, String, Option<u64>), PrReviewError> {
    // header format: "{thread_id} path: {file_path}:{line}" or "{thread_id} path: {file_path}"
    if let Some((_, thread_id, location)) = regex_captures!(r"^(\S+)\s+path:\s+(.+)$", header) {
        // Try to split location into path:line
        if let Some((_, path, line_str)) = regex_captures!(r"^(.+):(\d+)$", location) {
            let line = line_str.parse::<u64>().ok();
            return Ok((thread_id.to_string(), path.to_string(), line));
        }
        return Ok((thread_id.to_string(), location.to_string(), None));
    }

    Err(PrReviewError::ThreadParseError {
        line: 0,
        details: format!("invalid thread header: {header}"),
    })
}

fn parse_resolve_checkbox(content: &str) -> bool {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "- [x] resolve" {
            return true;
        }
        if trimmed == "- [ ] resolve" {
            return false;
        }
    }
    false
}

fn extract_draft_reply(content: &str) -> Option<String> {
    // Find the last occurrence of <!-- /comment -->
    let last_comment_end = content.rfind("<!-- /comment -->");
    let after_last_comment = match last_comment_end {
        Some(pos) => {
            let after = &content[pos + "<!-- /comment -->".len()..];
            after.strip_prefix('\n').unwrap_or(after)
        }
        None => return None,
    };

    // Check if there's another thread starting (should not happen within a single thread part)
    // The draft is everything after the last /comment, trimmed
    let draft = after_last_comment.trim();

    if draft.is_empty() {
        None
    } else {
        Some(draft.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    #[rstest]
    fn test_parse_simple_thread() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- thread: RT_abc123 path: src/main.rs:42 -->
            - [ ] resolve
            <!-- diff -->
            ```diff
            @@ -40,3 +40,5 @@
             context
            -old
            +new
            ```
            <!-- /diff -->
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Fix this bug
            <!-- /comment -->
        "#};

        let parsed = MarkdownParser::parse(content).unwrap();
        assert_eq!(parsed.frontmatter.pr, 42);
        assert_eq!(parsed.frontmatter.repo, "fohte/armyknife");
        assert_eq!(parsed.frontmatter.pulled_at, "2024-01-15T10:00:00Z");
        assert_eq!(parsed.threads.len(), 1);

        let thread = &parsed.threads[0];
        assert_eq!(thread.thread_id, "RT_abc123");
        assert_eq!(thread.path, "src/main.rs");
        assert_eq!(thread.line, Some(42));
        assert!(!thread.resolve);
        assert!(thread.draft_reply.is_none());
    }

    #[rstest]
    fn test_parse_with_draft_reply() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- thread: RT_abc123 path: src/main.rs:42 -->
            - [ ] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Fix this bug
            <!-- /comment -->
            This is my draft reply.
        "#};

        let parsed = MarkdownParser::parse(content).unwrap();
        let thread = &parsed.threads[0];
        assert_eq!(
            thread.draft_reply,
            Some("This is my draft reply.".to_string())
        );
    }

    #[rstest]
    fn test_parse_resolved_thread() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- thread: RT_abc123 path: src/main.rs:42 -->
            - [x] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Fixed
            <!-- /comment -->
        "#};

        let parsed = MarkdownParser::parse(content).unwrap();
        assert!(parsed.threads[0].resolve);
    }

    #[rstest]
    fn test_parse_multiple_threads() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- thread: RT_abc path: src/a.rs:10 -->
            - [ ] resolve
            <!-- comment: @alice 2024-01-15T10:30:00Z -->
            Comment A
            <!-- /comment -->

            <!-- thread: RT_def path: src/b.rs:20 -->
            - [x] resolve
            <!-- comment: @bob 2024-01-15T11:00:00Z -->
            Comment B
            <!-- /comment -->
        "#};

        let parsed = MarkdownParser::parse(content).unwrap();
        assert_eq!(parsed.threads.len(), 2);
        assert_eq!(parsed.threads[0].thread_id, "RT_abc");
        assert_eq!(parsed.threads[0].path, "src/a.rs");
        assert_eq!(parsed.threads[0].line, Some(10));
        assert!(!parsed.threads[0].resolve);

        assert_eq!(parsed.threads[1].thread_id, "RT_def");
        assert_eq!(parsed.threads[1].path, "src/b.rs");
        assert_eq!(parsed.threads[1].line, Some(20));
        assert!(parsed.threads[1].resolve);
    }

    #[rstest]
    fn test_parse_thread_no_line_number() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- thread: RT_abc path: src/main.rs -->
            - [ ] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Comment
            <!-- /comment -->
        "#};

        let parsed = MarkdownParser::parse(content).unwrap();
        assert_eq!(parsed.threads[0].path, "src/main.rs");
        assert_eq!(parsed.threads[0].line, None);
    }

    #[rstest]
    fn test_parse_missing_frontmatter() {
        let content = "No frontmatter here";
        let result = MarkdownParser::parse(content);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_parse_invalid_frontmatter() {
        let content = indoc! {"
            ---
            invalid: yaml: content: [
            ---
        "};
        let result = MarkdownParser::parse(content);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_parse_no_threads() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---
        "#};

        let parsed = MarkdownParser::parse(content).unwrap();
        assert!(parsed.threads.is_empty());
    }

    #[rstest]
    fn test_parse_multiline_draft_reply() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- thread: RT_abc123 path: src/main.rs:42 -->
            - [ ] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Fix this
            <!-- /comment -->
            Line 1 of draft
            Line 2 of draft
        "#};

        let parsed = MarkdownParser::parse(content).unwrap();
        assert_eq!(
            parsed.threads[0].draft_reply,
            Some("Line 1 of draft\nLine 2 of draft".to_string())
        );
    }

    #[rstest]
    fn test_roundtrip_serialize_parse() {
        use super::super::serializer::{MarkdownSerializer, ThreadsFrontmatter};
        use crate::commands::gh::pr_review::models::{
            Comment, PrData, ReviewThread,
            comment::{Author, PullRequestReview},
            thread::CommentsNode,
        };

        let pr_data = PrData {
            reviews: vec![],
            threads: vec![ReviewThread {
                id: Some("RT_roundtrip".to_string()),
                is_resolved: false,
                comments: CommentsNode {
                    nodes: vec![Comment {
                        database_id: 1,
                        author: Some(Author {
                            login: "reviewer".to_string(),
                        }),
                        body: "Please fix this".to_string(),
                        created_at: "2024-01-15T10:30:00Z".to_string(),
                        path: Some("src/main.rs".to_string()),
                        line: Some(42),
                        original_line: None,
                        diff_hunk: Some("@@ -40,3 +40,5 @@\n context\n-old\n+new".to_string()),
                        reply_to: None,
                        pull_request_review: Some(PullRequestReview { database_id: 100 }),
                    }],
                },
            }],
        };

        let frontmatter = ThreadsFrontmatter {
            pr: 42,
            repo: "fohte/armyknife".to_string(),
            pulled_at: "2024-01-15T10:00:00Z".to_string(),
        };

        let serialized = MarkdownSerializer::serialize(&pr_data, &frontmatter);
        let parsed = MarkdownParser::parse(&serialized).unwrap();

        assert_eq!(parsed.frontmatter, frontmatter);
        assert_eq!(parsed.threads.len(), 1);
        assert_eq!(parsed.threads[0].thread_id, "RT_roundtrip");
        assert_eq!(parsed.threads[0].path, "src/main.rs");
        assert_eq!(parsed.threads[0].line, Some(42));
        assert!(!parsed.threads[0].resolve);
        assert!(parsed.threads[0].draft_reply.is_none());
    }
}
