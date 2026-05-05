use lazy_regex::regex_captures;

use super::markers;
use super::serializer::ThreadsFrontmatter;
use crate::commands::gh::pr_review::error::PrReviewError;
use crate::commands::gh::pr_review::models::ReviewState;

pub struct MarkdownParser;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedThreadsFile {
    pub frontmatter: ThreadsFrontmatter,
    pub reviews: Vec<ParsedReview>,
    pub threads: Vec<ParsedThread>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReview {
    pub author: String,
    pub state: Option<ReviewState>,
    pub created_at: String,
    pub body: String,
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
        let (reviews, body_after_reviews) = extract_reviews_section(body)?;
        let threads = parse_threads(body_after_reviews)?;

        Ok(ParsedThreadsFile {
            frontmatter,
            reviews,
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
    let skip = if content.starts_with("---\r\n") { 5 } else { 4 };
    let after_open = &content[skip..]; // skip "---\n" or "---\r\n"
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

/// Extract the `<!-- reviews -->` ... `<!-- /reviews -->` section, returning the parsed
/// reviews and the body content with that section removed (so thread parsing can proceed).
///
/// Markers must appear at the start of a line. Review bodies that quote a marker mid-line
/// must not split parsing — that's the whole reason for the line-prefix rule.
fn extract_reviews_section(body: &str) -> Result<(Vec<ParsedReview>, &str), PrReviewError> {
    let Some(open_idx) = find_line_start(body, markers::REVIEWS_OPEN) else {
        return Ok((Vec::new(), body));
    };
    // Require the open marker to occupy the whole line (followed by '\n' or EOF).
    let after_open_marker = &body[open_idx + markers::REVIEWS_OPEN.len()..];
    if !(after_open_marker.is_empty() || after_open_marker.starts_with('\n')) {
        return Ok((Vec::new(), body));
    }
    let after_open = after_open_marker.strip_prefix('\n').unwrap_or("");

    let close_idx = find_line_start(after_open, markers::REVIEWS_CLOSE).ok_or_else(|| {
        PrReviewError::ThreadParseError {
            line: 0,
            details: "unclosed reviews section".to_string(),
        }
    })?;
    let inner = &after_open[..close_idx];

    let after_close_marker = &after_open[close_idx + markers::REVIEWS_CLOSE.len()..];
    let rest = after_close_marker.strip_prefix('\n').unwrap_or("");

    let reviews = parse_reviews_inner(inner)?;
    Ok((reviews, rest))
}

/// Find `needle` only when it starts at the beginning of a line (start-of-string or
/// immediately after '\n'). Returns the byte offset of the match.
fn find_line_start(haystack: &str, needle: &str) -> Option<usize> {
    let mut search_from = 0;
    while let Some(rel) = haystack[search_from..].find(needle) {
        let abs = search_from + rel;
        if abs == 0 || haystack.as_bytes()[abs - 1] == b'\n' {
            return Some(abs);
        }
        search_from = abs + 1;
    }
    None
}

fn parse_reviews_inner(inner: &str) -> Result<Vec<ParsedReview>, PrReviewError> {
    let mut reviews = Vec::new();
    let mut cursor = 0;

    while cursor < inner.len() {
        let Some(open_rel) = find_line_start(&inner[cursor..], markers::REVIEW_OPEN_PREFIX) else {
            break;
        };
        let header_start = cursor + open_rel + markers::REVIEW_OPEN_PREFIX.len();

        // Header runs to the first " -->" on the same line.
        let line_end = inner[header_start..]
            .find('\n')
            .map(|n| header_start + n)
            .unwrap_or(inner.len());
        let header_line = &inner[header_start..line_end];
        let header =
            header_line
                .strip_suffix(" -->")
                .ok_or_else(|| PrReviewError::ThreadParseError {
                    line: 0,
                    details: "unclosed review header comment".to_string(),
                })?;
        let (author, state, created_at) = parse_review_header(header.trim())?;

        // Body starts after the newline that ends the header line.
        let body_start = if line_end < inner.len() {
            line_end + 1
        } else {
            line_end
        };
        let close_rel =
            find_line_start(&inner[body_start..], markers::REVIEW_CLOSE).ok_or_else(|| {
                PrReviewError::ThreadParseError {
                    line: 0,
                    details: "unclosed review body".to_string(),
                }
            })?;
        let body_end = body_start + close_rel;
        let body = inner[body_start..body_end]
            .trim_end_matches('\n')
            .to_string();

        reviews.push(ParsedReview {
            author,
            state,
            created_at,
            body,
        });

        // Advance past the closing marker line.
        let after_close = body_end + markers::REVIEW_CLOSE.len();
        cursor = if after_close < inner.len() && inner.as_bytes()[after_close] == b'\n' {
            after_close + 1
        } else {
            after_close
        };
    }

    Ok(reviews)
}

fn parse_review_header(
    header: &str,
) -> Result<(String, Option<ReviewState>, String), PrReviewError> {
    // header format: "@{author} state={STATE} {timestamp}"
    if let Some((_, author, state_str, created_at)) =
        regex_captures!(r"^@(\S+)\s+state=(\S+)\s+(\S+)$", header)
    {
        let state = ReviewState::from_graphql_str(state_str);
        return Ok((author.to_string(), state, created_at.to_string()));
    }

    Err(PrReviewError::ThreadParseError {
        line: 0,
        details: format!("invalid review header: {header}"),
    })
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
    // header format: "{thread_id} path: {location} author: @{login}"
    // or legacy format without author: "{thread_id} path: {location}"
    if let Some((_, thread_id, location)) =
        regex_captures!(r"^(\S+)\s+path:\s+(.+?)(?:\s+author:\s+@\S+)?$", header)
    {
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
    use indoc::{formatdoc, indoc};
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
    fn test_parse_crlf_line_endings() {
        let lf_content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- thread: RT_crlf path: src/main.rs:1 -->
            - [ ] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Comment
            <!-- /comment -->
        "#};
        let content = lf_content.replace('\n', "\r\n");

        let parsed = MarkdownParser::parse(&content).unwrap();
        assert_eq!(parsed.frontmatter.pr, 42);
        assert_eq!(parsed.threads.len(), 1);
        assert_eq!(parsed.threads[0].thread_id, "RT_crlf");
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
    #[case::with_line_number(
        "RT_abc123",
        "src/main.rs:42 author: @reviewer",
        "src/main.rs",
        Some(42)
    )]
    #[case::without_line_number("RT_abc", "src/main.rs author: @reviewer", "src/main.rs", None)]
    fn test_parse_thread_with_author(
        #[case] thread_id: &str,
        #[case] location: &str,
        #[case] expected_path: &str,
        #[case] expected_line: Option<u64>,
    ) {
        let content = formatdoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- thread: {thread_id} path: {location} -->
            - [ ] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Comment
            <!-- /comment -->
        "#};

        let parsed = MarkdownParser::parse(&content).unwrap();
        assert_eq!(parsed.threads.len(), 1);

        let thread = &parsed.threads[0];
        assert_eq!(thread.thread_id, thread_id);
        assert_eq!(thread.path, expected_path);
        assert_eq!(thread.line, expected_line);
    }

    #[rstest]
    fn test_parse_reviews_section() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- reviews -->
            <!-- review: @alice state=APPROVED 2024-01-15T10:00:00Z -->
            LGTM!
            <!-- /review -->
            <!-- review: @gemini-code-assist state=COMMENTED 2024-01-15T11:00:00Z -->
            Looks fine overall.
            One nit below.
            <!-- /review -->
            <!-- /reviews -->

            <!-- thread: RT_abc path: src/main.rs:42 -->
            - [ ] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Fix this
            <!-- /comment -->
        "#};

        let parsed = MarkdownParser::parse(content).unwrap();
        assert_eq!(parsed.reviews.len(), 2);

        assert_eq!(parsed.reviews[0].author, "alice");
        assert_eq!(parsed.reviews[0].state, Some(ReviewState::Approved));
        assert_eq!(parsed.reviews[0].created_at, "2024-01-15T10:00:00Z");
        assert_eq!(parsed.reviews[0].body, "LGTM!");

        assert_eq!(parsed.reviews[1].author, "gemini-code-assist");
        assert_eq!(parsed.reviews[1].state, Some(ReviewState::Commented));
        assert_eq!(
            parsed.reviews[1].body,
            "Looks fine overall.\nOne nit below."
        );

        // Threads must still parse correctly after the reviews section.
        assert_eq!(parsed.threads.len(), 1);
        assert_eq!(parsed.threads[0].thread_id, "RT_abc");
    }

    #[rstest]
    fn test_parse_no_reviews_section() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- thread: RT_abc path: src/main.rs:42 -->
            - [ ] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Fix this
            <!-- /comment -->
        "#};

        let parsed = MarkdownParser::parse(content).unwrap();
        assert!(parsed.reviews.is_empty());
        assert_eq!(parsed.threads.len(), 1);
    }

    #[rstest]
    fn test_parse_empty_reviews_section() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- reviews -->
            <!-- /reviews -->

            <!-- thread: RT_abc path: src/main.rs:42 -->
            - [ ] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Fix this
            <!-- /comment -->
        "#};

        let parsed = MarkdownParser::parse(content).unwrap();
        assert!(parsed.reviews.is_empty());
        assert_eq!(parsed.threads.len(), 1);
    }

    #[rstest]
    fn test_parse_review_body_quoting_marker_does_not_split() {
        // A reviewer literally quotes the marker in their body (e.g., explaining the
        // format). The line-start rule means it should not be treated as a new review.
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- reviews -->
            <!-- review: @alice state=COMMENTED 2024-01-15T10:00:00Z -->
            See `<!-- review: @x state=COMMENTED 2024 -->` -- this is just an inline quote.
            Still part of the same review body.
            <!-- /review -->
            <!-- /reviews -->
        "#};

        let parsed = MarkdownParser::parse(content).unwrap();
        assert_eq!(parsed.reviews.len(), 1);
        assert_eq!(parsed.reviews[0].author, "alice");
        assert!(parsed.reviews[0].body.contains("inline quote"));
        assert!(parsed.reviews[0].body.contains("Still part of the same"));
    }

    #[rstest]
    fn test_parse_unclosed_reviews_section_errors() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- reviews -->
            <!-- review: @alice state=COMMENTED 2024-01-15T10:00:00Z -->
            no closing markers
        "#};

        let result = MarkdownParser::parse(content);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_parse_unclosed_review_body_errors() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- reviews -->
            <!-- review: @alice state=COMMENTED 2024-01-15T10:00:00Z -->
            forgot to close
            <!-- /reviews -->
        "#};

        let result = MarkdownParser::parse(content);
        assert!(result.is_err());
    }

    #[rstest]
    fn test_parse_review_with_unknown_state() {
        let content = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            ---

            <!-- reviews -->
            <!-- review: @alice state=UNKNOWN_STATE 2024-01-15T10:00:00Z -->
            Body
            <!-- /review -->
            <!-- /reviews -->
        "#};

        let parsed = MarkdownParser::parse(content).unwrap();
        assert_eq!(parsed.reviews.len(), 1);
        assert_eq!(parsed.reviews[0].state, None);
    }

    #[rstest]
    fn test_roundtrip_reviews_serialize_parse() {
        use super::super::serializer::{MarkdownSerializer, ThreadsFrontmatter};
        use crate::commands::gh::pr_review::models::PrData;
        use crate::commands::gh::pr_review::models::Review;
        use crate::commands::gh::pr_review::models::comment::Author;

        let body_text = indoc! {"
            Multi
            line
            body
        "}
        .trim_end_matches('\n');

        let pr_data = PrData {
            reviews: vec![Review {
                database_id: 7,
                author: Some(Author {
                    login: "alice".to_string(),
                }),
                body: body_text.to_string(),
                state: ReviewState::ChangesRequested,
                created_at: "2024-01-15T09:00:00Z".to_string(),
            }],
            threads: vec![],
        };
        let frontmatter = ThreadsFrontmatter {
            pr: 42,
            repo: "fohte/armyknife".to_string(),
            pulled_at: "2024-01-15T10:00:00Z".to_string(),
            submit: false,
        };
        let serialized = MarkdownSerializer::serialize(&pr_data, &frontmatter);
        let parsed = MarkdownParser::parse(&serialized).unwrap();

        assert_eq!(parsed.reviews.len(), 1);
        assert_eq!(parsed.reviews[0].author, "alice");
        assert_eq!(parsed.reviews[0].state, Some(ReviewState::ChangesRequested));
        assert_eq!(parsed.reviews[0].created_at, "2024-01-15T09:00:00Z");
        assert_eq!(parsed.reviews[0].body, body_text);
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
            submit: false,
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
