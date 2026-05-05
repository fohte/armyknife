use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::diff_compress::{CompressInput, compress_diff_hunk};
use super::markers;
use crate::commands::gh::pr_review::details::{clean_review_body, strip_noise_keep_details};
use crate::commands::gh::pr_review::models::{PrData, Review, ReviewThread};
use crate::shared::human_in_the_loop::DocumentSchema;

/// Output options for `MarkdownSerializer`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SerializeOptions {
    /// When `true`, leave `<details>` blocks intact. Otherwise collapse them
    /// to a single-line marker so bot reviews don't dominate the file.
    pub open_details: bool,
}

/// YAML frontmatter for the threads.md file.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThreadsFrontmatter {
    pub pr: u64,
    pub repo: String,
    pub pulled_at: String,
    /// Set to `true` to mark replies as approved after editor close.
    #[serde(default)]
    pub submit: bool,
}

impl DocumentSchema for ThreadsFrontmatter {
    fn is_approved(&self) -> bool {
        self.submit
    }
}

pub struct MarkdownSerializer;

/// Result of a serialize call. `parse_failed_threads` lists the location
/// (e.g. `src/foo.rs:42`) of threads whose `diffHunk` could not be parsed,
/// so the caller can surface a CLI warning.
pub struct SerializeOutcome {
    pub text: String,
    pub parse_failed_threads: Vec<String>,
}

impl MarkdownSerializer {
    /// Generate a Markdown string from PrData and frontmatter metadata.
    pub fn serialize(pr_data: &PrData, frontmatter: &ThreadsFrontmatter) -> String {
        Self::serialize_with_drafts(pr_data, frontmatter, &HashMap::new())
    }

    /// Generate a Markdown string, preserving existing draft replies keyed by thread node ID.
    pub fn serialize_with_drafts(
        pr_data: &PrData,
        frontmatter: &ThreadsFrontmatter,
        existing_drafts: &HashMap<String, String>,
    ) -> String {
        Self::serialize_with_options(
            pr_data,
            frontmatter,
            existing_drafts,
            &SerializeOptions::default(),
        )
        .text
    }

    /// Full-form serializer used by `reply pull`. `existing_drafts` preserves
    /// the user's in-progress reply text across re-pulls.
    pub fn serialize_with_options(
        pr_data: &PrData,
        frontmatter: &ThreadsFrontmatter,
        existing_drafts: &HashMap<String, String>,
        options: &SerializeOptions,
    ) -> SerializeOutcome {
        let mut output = String::new();
        let mut parse_failed_threads = Vec::new();

        // YAML frontmatter
        output.push_str("---\n");
        output.push_str(&format!("pr: {}\n", frontmatter.pr));
        output.push_str(&format!("repo: \"{}\"\n", frontmatter.repo));
        output.push_str(&format!("pulled_at: \"{}\"\n", frontmatter.pulled_at));
        output.push_str(&format!("submit: {}\n", frontmatter.submit));
        output.push_str("---\n");

        if let Some(section) = serialize_reviews_section(&pr_data.reviews, options) {
            output.push('\n');
            output.push_str(&section);
        }

        for thread in &pr_data.threads {
            output.push('\n');
            output.push_str(&serialize_thread(
                thread,
                existing_drafts,
                options,
                &mut parse_failed_threads,
            ));
        }

        SerializeOutcome {
            text: output,
            parse_failed_threads,
        }
    }
}

/// Serialize the reviews section. Returns `None` when no review has a non-empty body
/// (empty-body reviews are skipped, since GitHub creates one for every approve/comment).
fn serialize_reviews_section(reviews: &[Review], options: &SerializeOptions) -> Option<String> {
    let mut sorted: Vec<&Review> = reviews
        .iter()
        .filter(|r| !r.body.trim().is_empty())
        .collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    let mut output = String::new();
    output.push_str(markers::REVIEWS_OPEN);
    output.push('\n');
    for review in sorted {
        let author = review
            .author
            .as_ref()
            .map(|a| a.login.as_str())
            .unwrap_or("ghost");
        let state = review.state.as_graphql_str();
        let timestamp = &review.created_at;
        output.push_str(&format!(
            "{prefix}@{author} state={state} {timestamp} -->\n",
            prefix = markers::REVIEW_OPEN_PREFIX,
        ));
        let body = if options.open_details {
            strip_noise_keep_details(&review.body)
        } else {
            clean_review_body(&review.body)
        };
        output.push_str(&body);
        if !body.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(markers::REVIEW_CLOSE);
        output.push('\n');
    }
    output.push_str(markers::REVIEWS_CLOSE);
    output.push('\n');
    Some(output)
}

fn serialize_thread(
    thread: &ReviewThread,
    existing_drafts: &HashMap<String, String>,
    options: &SerializeOptions,
    parse_failed_threads: &mut Vec<String>,
) -> String {
    let mut output = String::new();

    let root = match thread.root_comment() {
        Some(c) => c,
        None => return output,
    };

    // Thread header
    let node_id = thread.id.as_deref().unwrap_or("unknown");
    let path = root.path.as_deref().unwrap_or("unknown");
    let line = root
        .effective_line()
        .map(|l| l.to_string())
        .unwrap_or_default();

    let location = if line.is_empty() {
        path.to_string()
    } else {
        format!("{path}:{line}")
    };

    let author = root.author_login();
    output.push_str(&format!(
        "<!-- thread: {node_id} path: {location} author: @{author} -->\n"
    ));

    // Resolve checkbox
    if thread.is_resolved {
        output.push_str("- [x] resolve\n");
    } else {
        output.push_str("- [ ] resolve\n");
    }

    // Diff hunk from root comment, compressed around the commented region.
    if let Some(diff_hunk) = &root.diff_hunk {
        let compressed = compress_diff_hunk(&CompressInput {
            diff_hunk,
            line: root.line,
            start_line: root.start_line,
            original_line: root.original_line,
            original_start_line: root.original_start_line,
        });
        if compressed.parse_failed {
            parse_failed_threads.push(location.clone());
        }
        output.push_str("<!-- diff -->\n");
        output.push_str("```diff\n");
        output.push_str(&compressed.text);
        output.push_str("```\n");
        output.push_str("<!-- /diff -->\n");
    }

    // All comments in chronological order
    for comment in &thread.comments.nodes {
        let author = comment.author_login();
        let timestamp = &comment.created_at;
        let body = if options.open_details {
            strip_noise_keep_details(&comment.body)
        } else {
            clean_review_body(&comment.body)
        };
        output.push_str(&format!("<!-- comment: @{author} {timestamp} -->\n"));
        output.push_str(&body);
        if !body.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("<!-- /comment -->\n");
    }

    // Existing draft reply (preserved from previous pull)
    if let Some(draft) = existing_drafts.get(node_id)
        && !draft.trim().is_empty()
    {
        output.push_str(draft);
        if !draft.ends_with('\n') {
            output.push('\n');
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::gh::pr_review::models::{
        Comment, ReviewState,
        comment::{Author, PullRequestReview, ReplyTo},
        thread::CommentsNode,
    };
    use indoc::indoc;
    use rstest::rstest;

    fn make_review(author: &str, state: ReviewState, body: &str, created_at: &str) -> Review {
        Review {
            database_id: 1,
            author: Some(Author {
                login: author.to_string(),
            }),
            body: body.to_string(),
            state,
            created_at: created_at.to_string(),
        }
    }

    fn make_comment(id: i64, author: &str, body: &str) -> Comment {
        Comment {
            database_id: id,
            author: Some(Author {
                login: author.to_string(),
            }),
            body: body.to_string(),
            created_at: "2024-01-15T10:30:00Z".to_string(),
            path: None,
            line: None,
            start_line: None,
            original_line: None,
            original_start_line: None,
            diff_hunk: None,
            reply_to: None,
            pull_request_review: None,
        }
    }

    fn make_thread(
        node_id: Option<&str>,
        comments: Vec<Comment>,
        is_resolved: bool,
    ) -> ReviewThread {
        ReviewThread {
            id: node_id.map(|s| s.to_string()),
            is_resolved,
            comments: CommentsNode { nodes: comments },
        }
    }

    fn default_frontmatter() -> ThreadsFrontmatter {
        ThreadsFrontmatter {
            pr: 42,
            repo: "fohte/armyknife".to_string(),
            pulled_at: "2024-01-15T10:00:00Z".to_string(),
            submit: false,
        }
    }

    #[rstest]
    fn test_serialize_empty_threads() {
        let pr_data = PrData {
            reviews: vec![],
            threads: vec![],
        };
        let result = MarkdownSerializer::serialize(&pr_data, &default_frontmatter());
        let expected = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
            ---
        "#};
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_serialize_single_comment_thread() {
        let pr_data = PrData {
            reviews: vec![],
            threads: vec![make_thread(
                Some("RT_abc123"),
                vec![Comment {
                    path: Some("src/main.rs".to_string()),
                    line: Some(41),
                    diff_hunk: Some("@@ -40,3 +40,3 @@\n context\n-old\n+new".to_string()),
                    pull_request_review: Some(PullRequestReview { database_id: 100 }),
                    ..make_comment(1, "reviewer", "Fix this bug")
                }],
                false,
            )],
        };
        let result = MarkdownSerializer::serialize(&pr_data, &default_frontmatter());
        let expected = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
            ---

            <!-- thread: RT_abc123 path: src/main.rs:41 author: @reviewer -->
            - [ ] resolve
            <!-- diff -->
            ```diff
            @@ -40,3 +40,3 @@
             context
            -old
            +new
            ```
            <!-- /diff -->
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Fix this bug
            <!-- /comment -->
        "#};
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_serialize_thread_with_replies() {
        let pr_data = PrData {
            reviews: vec![],
            threads: vec![make_thread(
                Some("RT_def456"),
                vec![
                    Comment {
                        path: Some("lib.rs".to_string()),
                        line: Some(10),
                        pull_request_review: Some(PullRequestReview { database_id: 100 }),
                        ..make_comment(1, "reviewer", "Please fix")
                    },
                    Comment {
                        reply_to: Some(ReplyTo {}),
                        pull_request_review: Some(PullRequestReview { database_id: 100 }),
                        ..make_comment(2, "author", "Done!")
                    },
                ],
                false,
            )],
        };
        let result = MarkdownSerializer::serialize(&pr_data, &default_frontmatter());
        let expected = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
            ---

            <!-- thread: RT_def456 path: lib.rs:10 author: @reviewer -->
            - [ ] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Please fix
            <!-- /comment -->
            <!-- comment: @author 2024-01-15T10:30:00Z -->
            Done!
            <!-- /comment -->
        "#};
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_serialize_resolved_thread() {
        let pr_data = PrData {
            reviews: vec![],
            threads: vec![make_thread(
                Some("RT_resolved"),
                vec![Comment {
                    path: Some("a.rs".to_string()),
                    line: Some(1),
                    ..make_comment(1, "reviewer", "Nit")
                }],
                true,
            )],
        };
        let result = MarkdownSerializer::serialize(&pr_data, &default_frontmatter());
        let expected = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
            ---

            <!-- thread: RT_resolved path: a.rs:1 author: @reviewer -->
            - [x] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Nit
            <!-- /comment -->
        "#};
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_serialize_with_drafts_preserves_existing() {
        let pr_data = PrData {
            reviews: vec![],
            threads: vec![make_thread(
                Some("RT_abc123"),
                vec![Comment {
                    path: Some("a.rs".to_string()),
                    line: Some(1),
                    ..make_comment(1, "reviewer", "Fix this")
                }],
                false,
            )],
        };
        let mut drafts = HashMap::new();
        drafts.insert("RT_abc123".to_string(), "My draft reply\n".to_string());

        let result =
            MarkdownSerializer::serialize_with_drafts(&pr_data, &default_frontmatter(), &drafts);
        let expected = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
            ---

            <!-- thread: RT_abc123 path: a.rs:1 author: @reviewer -->
            - [ ] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Fix this
            <!-- /comment -->
            My draft reply
        "#};
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_serialize_compresses_long_diff_hunk() {
        // Build a 30-line context-only hunk so the compressor has predictable
        // arithmetic: comment on new line 20 → keep idx 14..=22 (5 before,
        // 3 after), drop the rest with omission markers.
        let mut hunk = String::from("@@ -1,30 +1,30 @@\n");
        for i in 1..=30 {
            hunk.push_str(&format!(" line {i}\n"));
        }

        let pr_data = PrData {
            reviews: vec![],
            threads: vec![make_thread(
                Some("RT_long"),
                vec![Comment {
                    path: Some("a.rs".to_string()),
                    line: Some(20),
                    diff_hunk: Some(hunk),
                    ..make_comment(1, "reviewer", "Fix")
                }],
                false,
            )],
        };

        let result = MarkdownSerializer::serialize(&pr_data, &default_frontmatter());
        let expected = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
            ---

            <!-- thread: RT_long path: a.rs:20 author: @reviewer -->
            - [ ] resolve
            <!-- diff -->
            ```diff
            @@ -1,30 +1,30 @@
            ... [14 lines omitted] ...
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
            ```
            <!-- /diff -->
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Fix
            <!-- /comment -->
        "#};
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case::no_diff_hunk(None, None, vec![])]
    #[case::header_present(
        Some("@@ -1,1 +1,1 @@\n a\n".to_string()),
        Some(1),
        vec![],
    )]
    #[case::header_unparseable_with_line(
        Some("garbage line\n".to_string()),
        Some(7),
        vec!["src/foo.rs:7".to_string()],
    )]
    #[case::header_unparseable_without_line(
        Some("garbage line\n".to_string()),
        None,
        vec!["src/foo.rs".to_string()],
    )]
    fn test_serialize_records_parse_failed_thread_locations(
        #[case] diff_hunk: Option<String>,
        #[case] line: Option<i64>,
        #[case] expected_locations: Vec<String>,
    ) {
        let pr_data = PrData {
            reviews: vec![],
            threads: vec![make_thread(
                Some("RT_pf"),
                vec![Comment {
                    path: Some("src/foo.rs".to_string()),
                    line,
                    diff_hunk,
                    ..make_comment(1, "reviewer", "Fix")
                }],
                false,
            )],
        };

        let outcome = MarkdownSerializer::serialize_with_options(
            &pr_data,
            &default_frontmatter(),
            &HashMap::new(),
            &SerializeOptions::default(),
        );

        assert_eq!(outcome.parse_failed_threads, expected_locations);
    }

    #[rstest]
    fn test_serialize_collapses_details_in_comment_body() {
        let body = "<details><summary>Prompt for agents</summary>secret</details>";
        let pr_data = PrData {
            reviews: vec![],
            threads: vec![make_thread(
                Some("RT_d"),
                vec![Comment {
                    path: Some("a.rs".to_string()),
                    line: Some(1),
                    ..make_comment(1, "bot", body)
                }],
                false,
            )],
        };

        let collapsed_default = MarkdownSerializer::serialize(&pr_data, &default_frontmatter());
        let expected_collapsed = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
            ---

            <!-- thread: RT_d path: a.rs:1 author: @bot -->
            - [ ] resolve
            <!-- comment: @bot 2024-01-15T10:30:00Z -->
            [▶ Prompt for agents]
            <!-- /comment -->
        "#};
        assert_eq!(collapsed_default, expected_collapsed);

        let opts = SerializeOptions { open_details: true };
        let preserved = MarkdownSerializer::serialize_with_options(
            &pr_data,
            &default_frontmatter(),
            &HashMap::new(),
            &opts,
        );
        let expected_preserved = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
            ---

            <!-- thread: RT_d path: a.rs:1 author: @bot -->
            - [ ] resolve
            <!-- comment: @bot 2024-01-15T10:30:00Z -->
            <details><summary>Prompt for agents</summary>secret</details>
            <!-- /comment -->
        "#};
        assert_eq!(preserved.text, expected_preserved);
    }

    #[rstest]
    fn test_serialize_reviews_with_body() {
        let pr_data = PrData {
            reviews: vec![
                make_review(
                    "alice",
                    ReviewState::Approved,
                    "LGTM!",
                    "2024-01-15T10:00:00Z",
                ),
                make_review(
                    "gemini-code-assist",
                    ReviewState::Commented,
                    indoc! {"
                        Looks fine overall.
                        One nit below."},
                    "2024-01-15T11:00:00Z",
                ),
            ],
            threads: vec![],
        };
        let result = MarkdownSerializer::serialize(&pr_data, &default_frontmatter());
        let expected = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
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
        "#};
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_serialize_skips_empty_review_bodies() {
        let pr_data = PrData {
            reviews: vec![
                make_review("alice", ReviewState::Approved, "", "2024-01-15T10:00:00Z"),
                make_review(
                    "bob",
                    ReviewState::Commented,
                    "   \n  ",
                    "2024-01-15T10:30:00Z",
                ),
            ],
            threads: vec![],
        };
        let result = MarkdownSerializer::serialize(&pr_data, &default_frontmatter());
        let expected = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
            ---
        "#};
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_serialize_reviews_sorted_by_created_at() {
        let pr_data = PrData {
            reviews: vec![
                make_review(
                    "later",
                    ReviewState::Commented,
                    "second",
                    "2024-01-15T12:00:00Z",
                ),
                make_review(
                    "earlier",
                    ReviewState::Commented,
                    "first",
                    "2024-01-15T10:00:00Z",
                ),
            ],
            threads: vec![],
        };
        let result = MarkdownSerializer::serialize(&pr_data, &default_frontmatter());
        let expected = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
            ---

            <!-- reviews -->
            <!-- review: @earlier state=COMMENTED 2024-01-15T10:00:00Z -->
            first
            <!-- /review -->
            <!-- review: @later state=COMMENTED 2024-01-15T12:00:00Z -->
            second
            <!-- /review -->
            <!-- /reviews -->
        "#};
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_serialize_reviews_before_threads() {
        let pr_data = PrData {
            reviews: vec![make_review(
                "alice",
                ReviewState::ChangesRequested,
                "Please address.",
                "2024-01-15T09:00:00Z",
            )],
            threads: vec![make_thread(
                Some("RT_xyz"),
                vec![Comment {
                    path: Some("a.rs".to_string()),
                    line: Some(1),
                    ..make_comment(1, "alice", "nit")
                }],
                false,
            )],
        };
        let result = MarkdownSerializer::serialize(&pr_data, &default_frontmatter());
        let expected = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
            ---

            <!-- reviews -->
            <!-- review: @alice state=CHANGES_REQUESTED 2024-01-15T09:00:00Z -->
            Please address.
            <!-- /review -->
            <!-- /reviews -->

            <!-- thread: RT_xyz path: a.rs:1 author: @alice -->
            - [ ] resolve
            <!-- comment: @alice 2024-01-15T10:30:00Z -->
            nit
            <!-- /comment -->
        "#};
        assert_eq!(result, expected);
    }

    #[rstest]
    fn test_serialize_no_line_number() {
        let pr_data = PrData {
            reviews: vec![],
            threads: vec![make_thread(
                Some("RT_noline"),
                vec![Comment {
                    path: Some("file.rs".to_string()),
                    ..make_comment(1, "reviewer", "Comment")
                }],
                false,
            )],
        };
        let result = MarkdownSerializer::serialize(&pr_data, &default_frontmatter());
        let expected = indoc! {r#"
            ---
            pr: 42
            repo: "fohte/armyknife"
            pulled_at: "2024-01-15T10:00:00Z"
            submit: false
            ---

            <!-- thread: RT_noline path: file.rs author: @reviewer -->
            - [ ] resolve
            <!-- comment: @reviewer 2024-01-15T10:30:00Z -->
            Comment
            <!-- /comment -->
        "#};
        assert_eq!(result, expected);
    }
}
