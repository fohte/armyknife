//! Compress GitHub `diffHunk` payloads down to the lines surrounding the
//! commented region. GraphQL returns the hunk from its header through the
//! commented line, which can run into hundreds of lines on long files; the
//! Markdown threads file feeds an LLM, so the unbounded payload wastes
//! context.
//!
//! Strategy: keep the hunk header, the commented line(s), `LINES_BEFORE`
//! lines of leading context, and `LINES_AFTER` lines of trailing context.
//! Replace anything else with a single `... [N lines omitted] ...` marker.
//! When the comment line cannot be located in the hunk body — most often
//! because GitHub truncates the hunk before the comment line — keep the
//! hunk header and show the last `FALLBACK_TAIL_LINES` body lines so the
//! reader still gets function context plus the most recent diff content.
//! Only when the hunk header itself cannot be parsed is the result flagged
//! as `parse_failed = true`, so the caller can surface a CLI warning.

use lazy_regex::regex_captures;

const LINES_BEFORE: usize = 5;
const LINES_AFTER: usize = 3;
const FALLBACK_TAIL_LINES: usize = 30;

/// Inputs for compressing a single `diffHunk` payload.
pub struct CompressInput<'a> {
    pub diff_hunk: &'a str,
    /// Commented line on the post-image (`+` side). `None` when the comment is
    /// on a deleted line / outdated context, in which case `original_line` is
    /// used against the pre-image (`-` side).
    pub line: Option<i64>,
    pub start_line: Option<i64>,
    pub original_line: Option<i64>,
    pub original_start_line: Option<i64>,
}

/// Result of compressing a single `diffHunk`.
pub struct CompressOutcome {
    /// Compressed diff text. Always ends with `\n`.
    pub text: String,
    /// True when the hunk header could not be parsed and the body was
    /// emitted verbatim (tail-truncated). Callers may use this to emit a
    /// CLI warning.
    pub parse_failed: bool,
}

/// Compress a `diffHunk` to the relevant window around the commented line.
pub fn compress_diff_hunk(input: &CompressInput) -> CompressOutcome {
    match try_compress(input) {
        Some(text) => CompressOutcome {
            text,
            parse_failed: false,
        },
        None => CompressOutcome {
            text: tail_only(input.diff_hunk),
            parse_failed: true,
        },
    }
}

fn try_compress(input: &CompressInput) -> Option<String> {
    let lines: Vec<&str> = input.diff_hunk.lines().collect();
    let header_idx = last_hunk_header(&lines)?;

    // Only keep content from the last hunk header onward — multi-hunk
    // diffHunks (rare, but possible after rebases) collapse to the most
    // recent one, which is the one that actually contains the comment.
    let header = lines[header_idx];
    let body: &[&str] = &lines[header_idx + 1..];

    let (new_start, old_start) = parse_hunk_header(header)?;

    let target_indices = locate_target(input, body, new_start, old_start);

    let (keep_start, keep_end) = match target_indices {
        Some(indices) => {
            // `indices` is non-empty by construction in `locate_target`.
            let min_idx = *indices.first()?;
            let max_idx = *indices.last()?;
            let keep_start = min_idx.saturating_sub(LINES_BEFORE);
            let keep_end = (max_idx + LINES_AFTER + 1).min(body.len());
            (keep_start, keep_end)
        }
        // Comment line falls outside the body (commonly: GitHub truncated
        // the hunk before the commented line, or the comment is not tied
        // to a specific line). Keep the header for function context and
        // surface the tail of whatever body we did receive.
        None => (body.len().saturating_sub(FALLBACK_TAIL_LINES), body.len()),
    };

    let mut out = String::new();
    out.push_str(header);
    out.push('\n');

    if keep_start > 0 {
        out.push_str(&format!("... [{keep_start} lines omitted] ...\n"));
    }

    for line in &body[keep_start..keep_end] {
        out.push_str(line);
        out.push('\n');
    }

    let omitted_after = body.len().saturating_sub(keep_end);
    if omitted_after > 0 {
        out.push_str(&format!("... [{omitted_after} lines omitted] ...\n"));
    }

    Some(out)
}

/// Locate the body indices that match the comment line range. Returns
/// `None` when no anchor exists (no `line` / `original_line`) or when the
/// comment line is not present in the body.
fn locate_target(
    input: &CompressInput,
    body: &[&str],
    new_start: i64,
    old_start: i64,
) -> Option<Vec<usize>> {
    // Decide which side anchors the comment. `line` is the post-image (`+`)
    // line and is canonical for live comments. `originalLine` appears on
    // resolved/outdated comments where `line` is null; it can refer to
    // either side depending on whether the comment was attached to an added
    // or deleted line, and the GraphQL response does not expose the side
    // directly. Try the new side first and fall back to the old side. If
    // both sides happen to contain the same line number on context lines,
    // the new side wins — acceptable since the chosen window will still
    // surround a real instance of that line in the hunk.
    let candidates: Vec<(i64, i64, bool)> = match (input.line, input.original_line) {
        (Some(end), _) => vec![(input.start_line.unwrap_or(end), end, true)],
        (None, Some(end)) => {
            let start = input.original_start_line.unwrap_or(end);
            vec![(start, end, true), (start, end, false)]
        }
        (None, None) => return None,
    };

    candidates
        .into_iter()
        .map(|(start, end, use_new)| {
            collect_target_indices(body, new_start, old_start, start, end, use_new)
        })
        .find(|hits| !hits.is_empty())
}

fn last_hunk_header(lines: &[&str]) -> Option<usize> {
    lines.iter().rposition(|l| l.starts_with("@@"))
}

/// Parse a hunk header `@@ -OLD,len +NEW,len @@ ...` into `(new_start, old_start)`.
fn parse_hunk_header(header: &str) -> Option<(i64, i64)> {
    let (_, old_start_str, new_start_str) =
        regex_captures!(r"@@\s+-(\d+)(?:,\d+)?\s+\+(\d+)(?:,\d+)?\s+@@", header)?;
    let old_start = old_start_str.parse().ok()?;
    let new_start = new_start_str.parse().ok()?;
    Some((new_start, old_start))
}

fn collect_target_indices(
    body: &[&str],
    new_start: i64,
    old_start: i64,
    target_start: i64,
    target_end: i64,
    use_new_side: bool,
) -> Vec<usize> {
    let mut new_line = new_start;
    let mut old_line = old_start;
    let mut hits = Vec::new();

    for (idx, line) in body.iter().enumerate() {
        // Diff prefixes are always single-byte ASCII (`+`, `-`, ` `, `\`),
        // so reading the first byte avoids UTF-8 decoding on every line.
        let prefix = line.as_bytes().first().copied().unwrap_or(b' ');
        let (current, advance_new, advance_old) = match prefix {
            b'+' => (new_line, true, false),
            b'-' => (old_line, false, true),
            // Treat anything else (space, '\', empty line) as context.
            _ => (if use_new_side { new_line } else { old_line }, true, true),
        };

        let on_target_side = match prefix {
            b'+' => use_new_side,
            b'-' => !use_new_side,
            _ => true,
        };

        if on_target_side && current >= target_start && current <= target_end {
            hits.push(idx);
        }

        if advance_new {
            new_line += 1;
        }
        if advance_old {
            old_line += 1;
        }
    }

    hits
}

/// Last-ditch rendering for hunks whose header could not be parsed:
/// emit at most the trailing `FALLBACK_TAIL_LINES` lines verbatim, with
/// an `omitted` marker when the body is longer. Always ends with `\n` so
/// the caller can append it inside a fenced code block without special
/// casing empty input.
fn tail_only(diff_hunk: &str) -> String {
    let lines: Vec<&str> = diff_hunk.lines().collect();
    let tail_start = lines.len().saturating_sub(FALLBACK_TAIL_LINES);

    let mut out = String::new();
    if tail_start > 0 {
        out.push_str(&format!("... [{tail_start} lines omitted] ...\n"));
    }
    for line in &lines[tail_start..] {
        out.push_str(line);
        out.push('\n');
    }
    if out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    fn input_for(diff_hunk: &str, line: i64) -> CompressInput<'_> {
        CompressInput {
            diff_hunk,
            line: Some(line),
            start_line: None,
            original_line: None,
            original_start_line: None,
        }
    }

    fn compress(input: &CompressInput) -> CompressOutcome {
        compress_diff_hunk(input)
    }

    #[rstest]
    fn test_keeps_window_around_target_line() {
        // Header says new side starts at 1. Lines 1..=20 are all context.
        // Comment is on new line 15.
        let mut hunk = String::from("@@ -1,20 +1,20 @@\n");
        for i in 1..=20 {
            hunk.push_str(&format!(" line {i}\n"));
        }

        let result = compress(&input_for(&hunk, 15));

        let expected = indoc! {"
            @@ -1,20 +1,20 @@
            ... [9 lines omitted] ...
             line 10
             line 11
             line 12
             line 13
             line 14
             line 15
             line 16
             line 17
             line 18
            ... [2 lines omitted] ...
        "};
        assert_eq!(result.text, expected);
        assert!(!result.parse_failed);
    }

    #[rstest]
    fn test_target_at_end_no_trailing_omission() {
        let mut hunk = String::from("@@ -1,5 +1,5 @@\n");
        for i in 1..=5 {
            hunk.push_str(&format!(" line {i}\n"));
        }

        let result = compress(&input_for(&hunk, 5));

        let expected = indoc! {"
            @@ -1,5 +1,5 @@
             line 1
             line 2
             line 3
             line 4
             line 5
        "};
        assert_eq!(result.text, expected);
        assert!(!result.parse_failed);
    }

    #[rstest]
    fn test_handles_added_lines() {
        let hunk = indoc! {"
            @@ -1,6 +1,8 @@
             ctx 1
             ctx 2
             ctx 3
            +added 1
            +added 2
             ctx 4
             ctx 5
             ctx 6
        "};

        let result = compress(&input_for(hunk, 4));

        let expected = indoc! {"
            @@ -1,6 +1,8 @@
             ctx 1
             ctx 2
             ctx 3
            +added 1
            +added 2
             ctx 4
             ctx 5
            ... [1 lines omitted] ...
        "};
        assert_eq!(result.text, expected);
    }

    #[rstest]
    fn test_multiline_range() {
        let mut hunk = String::from("@@ -1,30 +1,30 @@\n");
        for i in 1..=30 {
            hunk.push_str(&format!(" line {i}\n"));
        }

        let input = CompressInput {
            diff_hunk: &hunk,
            line: Some(20),
            start_line: Some(18),
            original_line: None,
            original_start_line: None,
        };
        let result = compress(&input);

        let expected = indoc! {"
            @@ -1,30 +1,30 @@
            ... [12 lines omitted] ...
             line 13
             line 14
             line 15
             line 16
             line 17
             line 18
             line 19
             line 20
             line 21
             line 22
             line 23
            ... [7 lines omitted] ...
        "};
        assert_eq!(result.text, expected);
    }

    #[rstest]
    fn test_outdated_comment_with_original_line_on_new_side() {
        let hunk = indoc! {"
            @@ -80,3 +80,5 @@
             ctx 80
             ctx 81
             ctx 82
            +added 83
            +added 84
        "};

        let input = CompressInput {
            diff_hunk: hunk,
            line: None,
            start_line: None,
            original_line: Some(84),
            original_start_line: None,
        };
        let result = compress(&input);

        assert_eq!(result.text, hunk);
    }

    #[rstest]
    fn test_outdated_comment_uses_original_line() {
        let hunk = indoc! {"
            @@ -10,5 +10,4 @@
             ctx 10
             ctx 11
            -deleted at old 12
             ctx 13
             ctx 14
        "};

        let input = CompressInput {
            diff_hunk: hunk,
            line: None,
            start_line: None,
            original_line: Some(12),
            original_start_line: None,
        };
        let result = compress(&input);

        assert_eq!(result.text, hunk);
    }

    #[rstest]
    fn test_multi_hunk_keeps_only_last() {
        let hunk = indoc! {"
            @@ -1,3 +1,3 @@
             top 1
             top 2
             top 3
            @@ -100,5 +100,5 @@
             bot 100
             bot 101
             bot 102
             bot 103
             bot 104
        "};

        let result = compress(&input_for(hunk, 102));

        let expected = indoc! {"
            @@ -100,5 +100,5 @@
             bot 100
             bot 101
             bot 102
             bot 103
             bot 104
        "};
        assert_eq!(result.text, expected);
    }

    #[rstest]
    fn test_target_outside_hunk_keeps_header_and_tails_body() {
        // GitHub truncates a long diffHunk before the comment line. We keep
        // the header (function context) and the tail of the body the API
        // did return — no warning embedded, the leading omitted marker
        // already conveys "something was cut".
        let body_len = 50usize;
        let mut hunk = String::from("@@ -0,0 +1,703 @@ fn truncated()\n");
        for i in 0..body_len {
            hunk.push_str(&format!("+body {i}\n"));
        }

        let result = compress(&input_for(&hunk, 300));

        let omitted = body_len - FALLBACK_TAIL_LINES;
        let mut expected = String::from("@@ -0,0 +1,703 @@ fn truncated()\n");
        expected.push_str(&format!("... [{omitted} lines omitted] ...\n"));
        for i in omitted..body_len {
            expected.push_str(&format!("+body {i}\n"));
        }
        assert_eq!(result.text, expected);
        assert!(!result.parse_failed);
    }

    #[rstest]
    fn test_target_outside_short_body_keeps_header_no_marker() {
        // Body shorter than FALLBACK_TAIL_LINES: header + body only, no
        // leading omitted marker.
        let hunk = indoc! {"
            @@ -1,5 +1,5 @@ fn foo()
             line 1
             line 2
             line 3
             line 4
             line 5
        "};
        let result = compress(&input_for(hunk, 999));

        assert_eq!(result.text, hunk);
        assert!(!result.parse_failed);
    }

    #[rstest]
    fn test_no_line_information_keeps_header_and_tails_body() {
        // No `line` / `original_line`: nothing to anchor on, but the header
        // is still useful — emit it followed by the tail.
        let hunk = indoc! {"
            @@ -1,3 +1,3 @@
             a
             b
             c
        "};
        let input = CompressInput {
            diff_hunk: hunk,
            line: None,
            start_line: None,
            original_line: None,
            original_start_line: None,
        };

        let result = compress(&input);

        assert_eq!(result.text, hunk);
        assert!(!result.parse_failed);
    }

    #[rstest]
    fn test_parse_failed_when_no_header() {
        let hunk = " line 1\n line 2\n";
        let result = compress(&input_for(hunk, 1));

        assert_eq!(result.text, hunk);
        assert!(result.parse_failed);
    }

    #[rstest]
    fn test_parse_failed_truncates_to_tail_window() {
        let total = 50usize;
        let mut hunk = String::new();
        for i in 0..total {
            hunk.push_str(&format!("garbage {i}\n"));
        }

        let result = compress(&input_for(&hunk, 1));

        let omitted = total - FALLBACK_TAIL_LINES;
        let mut expected = format!("... [{omitted} lines omitted] ...\n");
        for i in omitted..total {
            expected.push_str(&format!("garbage {i}\n"));
        }
        assert_eq!(result.text, expected);
        assert!(result.parse_failed);
    }

    #[rstest]
    fn test_parse_failed_on_empty_hunk() {
        // The real-world payload from a free-floating PR comment with no
        // anchored line: empty diffHunk. Result is the parse-failed branch;
        // we still emit a trailing newline so the fenced code block stays
        // well-formed.
        let result = compress(&input_for("", 1));
        assert_eq!(result.text, "\n");
        assert!(result.parse_failed);
    }

    #[rstest]
    #[case::with_explicit_new_len("@@ -1,3 +5,7 @@", Some((5, 1)))]
    #[case::omitted_new_len("@@ -1,3 +5 @@", Some((5, 1)))]
    #[case::omitted_old_len("@@ -1 +5,7 @@", Some((5, 1)))]
    #[case::empty_new_side("@@ -1,5 +0,0 @@", Some((0, 1)))]
    #[case::with_function_suffix("@@ -1,3 +5,7 @@ fn foo()", Some((5, 1)))]
    #[case::missing_at_signs("not a header", None)]
    fn test_parse_hunk_header(#[case] header: &str, #[case] expected: Option<(i64, i64)>) {
        assert_eq!(parse_hunk_header(header), expected);
    }

    #[rstest]
    fn test_header_with_function_context_preserved() {
        // Header includes a ` fn my_function() {` suffix that the compressor
        // must keep verbatim — terminals and reviewers rely on it for
        // orientation.
        let hunk = indoc! {"
            @@ -10,5 +10,5 @@ fn my_function() {
             a
             b
             c
             d
             e
        "};

        let result = compress(&input_for(hunk, 12));

        assert_eq!(result.text, hunk);
    }
}
