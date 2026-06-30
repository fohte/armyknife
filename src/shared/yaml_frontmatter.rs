//! YAML frontmatter helpers shared across HITL review flows.
//!
//! All helpers operate on the leading `---\n...\n---\n?` block only and
//! tolerate both LF and CRLF on the delimiters.

use lazy_regex::regex_captures;
use regex::Regex;

/// Split content into `(frontmatter_block, yaml_body, body_offset)`.
/// `body_offset` is the byte index where the post-frontmatter body starts in
/// the original `content`. Returns `None` if no leading `---` block is found.
pub fn split_frontmatter(content: &str) -> Option<(&str, &str, usize)> {
    let (whole, yaml) = regex_captures!(r"^---\r?\n([\s\S]*?)\r?\n---\r?\n?", content)?;
    Some((whole, yaml, whole.len()))
}

/// Rewrite every YAML boolean field listed in `fields` to `false` inside the
/// leading frontmatter block. Field names are matched case-sensitively;
/// values are matched case-insensitively (`True`/`TRUE`/`Yes`/`On` count as
/// truthy). Trailing inline comments and optional CR are preserved.
///
/// Indentation is `\s*` so the same call works for both top-level (`submit:`)
/// and nested (`  submit:` under `steps:`) fields. Returns the original
/// string unchanged when no frontmatter is present.
pub fn reset_bool_fields(content: &str, fields: &[&str]) -> String {
    let Some((fm_block, _yaml, body_offset)) = split_frontmatter(content) else {
        return content.to_string();
    };

    let alternation = fields
        .iter()
        .map(|f| regex::escape(f))
        .collect::<Vec<_>>()
        .join("|");
    let pattern =
        format!(r"(?m)^(\s*(?:{alternation}):[ \t]*)(?i:true|yes|on)([ \t]*(?:#[^\r\n]*)?\r?)$");
    #[expect(
        clippy::unwrap_used,
        reason = "pattern is built from escaped literals and a known-valid regex skeleton"
    )]
    let re = Regex::new(&pattern).unwrap();
    let new_fm = re.replace_all(fm_block, "${1}false${2}");

    let mut result = String::with_capacity(content.len());
    result.push_str(&new_fm);
    result.push_str(&content[body_offset..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    #[rstest]
    #[case::lf(
        indoc! {"
            ---
            title: T
            steps:
              submit: true
              ready-for-translation: true
            ---
            body
        "},
        &["submit", "ready-for-translation"],
        indoc! {"
            ---
            title: T
            steps:
              submit: false
              ready-for-translation: false
            ---
            body
        "}
    )]
    #[case::top_level_only(
        indoc! {"
            ---
            title: T
            submit: true
            readonly:
              number: 1
            ---
            body
        "},
        &["submit"],
        indoc! {"
            ---
            title: T
            submit: false
            readonly:
              number: 1
            ---
            body
        "}
    )]
    #[case::crlf(
        concat!("---\r\n", "submit: true\r\n", "---\r\n", "body\r\n"),
        &["submit"],
        concat!("---\r\n", "submit: false\r\n", "---\r\n", "body\r\n")
    )]
    #[case::uppercase(
        indoc! {"
            ---
            submit: True
            ---
            body
        "},
        &["submit"],
        indoc! {"
            ---
            submit: false
            ---
            body
        "}
    )]
    #[case::trailing_comment(
        indoc! {"
            ---
            submit: true  # approved
            ---
            body
        "},
        &["submit"],
        indoc! {"
            ---
            submit: false  # approved
            ---
            body
        "}
    )]
    #[case::no_frontmatter(
        "body only\n",
        &["submit"],
        "body only\n"
    )]
    #[case::body_mentions_field(
        indoc! {"
            ---
            submit: true
            ---
            Body mentions submit: true on its own line
            submit: true
        "},
        &["submit"],
        indoc! {"
            ---
            submit: false
            ---
            Body mentions submit: true on its own line
            submit: true
        "}
    )]
    #[case::field_already_false(
        indoc! {"
            ---
            submit: false
            ---
            body
        "},
        &["submit"],
        indoc! {"
            ---
            submit: false
            ---
            body
        "}
    )]
    fn test_reset_bool_fields(
        #[case] input: &str,
        #[case] fields: &[&str],
        #[case] expected: &str,
    ) {
        assert_eq!(reset_bool_fields(input, fields), expected);
    }
}
