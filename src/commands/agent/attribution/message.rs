use std::path::Path;

use anyhow::Result;

use super::context::{self, OperationContext, unix_timestamp};

const TOOL_TRAILER: &str = "AI-Tool";
const MODEL_TRAILER: &str = "AI-Model";

pub(super) fn prepare(path: &Path, message: &str) -> Result<String> {
    let context = context::claim(path, unix_timestamp()?)?;
    add_trailers(message, &context)
}

fn add_trailers(message: &str, context: &OperationContext) -> Result<String> {
    let mut lines = normalized_lines(message);
    trim_trailing_blank_lines(&mut lines);
    let footer = trailer_block_start(&lines);
    if let Some(start) = footer {
        let mut kept = Vec::with_capacity(lines.len());
        let mut remove_continuation = false;
        for (index, line) in lines.iter().enumerate() {
            if index < start {
                kept.push(line.clone());
                continue;
            }
            if is_managed_trailer(line) {
                remove_continuation = true;
                continue;
            }
            if is_continuation(line) && remove_continuation {
                continue;
            }
            remove_continuation = false;
            kept.push(line.clone());
        }
        lines = kept;
    }
    trim_trailing_blank_lines(&mut lines);

    let has_retained_footer = trailer_block_start(&lines).is_some();
    let mut output = lines.join("\n");
    if !output.is_empty() {
        output.push('\n');
        if !has_retained_footer {
            output.push('\n');
        }
    }
    output.push_str(TOOL_TRAILER);
    output.push_str(": ");
    output.push_str(context.tool());
    output.push('\n');
    output.push_str(MODEL_TRAILER);
    output.push_str(": ");
    output.push_str(context.model().trailer_value());
    output.push('\n');
    Ok(output)
}

fn normalized_lines(message: &str) -> Vec<String> {
    message
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_owned())
        .collect()
}

fn trim_trailing_blank_lines(lines: &mut Vec<String>) {
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
}

fn trailer_block_start(lines: &[String]) -> Option<usize> {
    let mut end = lines.len();
    while end > 0 && lines[end - 1].is_empty() {
        end -= 1;
    }
    if end == 0 {
        return None;
    }

    let mut start = end;
    while start > 0 && (is_trailer_line(&lines[start - 1]) || is_continuation(&lines[start - 1])) {
        start -= 1;
    }
    if start == end || (start > 0 && !lines[start - 1].is_empty()) {
        return None;
    }
    Some(start)
}

fn is_trailer_line(line: &str) -> bool {
    let Some((token, value)) = line.split_once(": ") else {
        return false;
    };
    !token.is_empty()
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && !value.is_empty()
}

fn is_continuation(line: &str) -> bool {
    line.starts_with(' ') || line.starts_with('\t')
}

fn is_managed_trailer(line: &str) -> bool {
    let Some((token, _)) = line.split_once(": ") else {
        return false;
    };
    token.eq_ignore_ascii_case(TOOL_TRAILER) || token.eq_ignore_ascii_case(MODEL_TRAILER)
}

#[cfg(test)]
mod tests {
    use super::super::Provider;
    use super::prepare;
    use crate::commands::agent::attribution::context::{self, ModelIdentity, OperationContext};
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    #[fixture]
    fn context_dir() -> TempDir {
        tempfile::tempdir().expect("create test context directory")
    }

    fn create_context(
        directory: &TempDir,
        model: ModelIdentity,
        tool_use_id: &str,
    ) -> std::path::PathBuf {
        let context = OperationContext::new(Provider::Codex, model, tool_use_id.to_owned())
            .expect("create fixture operation context");
        context::create(&context, directory.path()).expect("persist fixture operation context")
    }

    #[rstest]
    fn prepare_replaces_managed_trailers_and_preserves_other_authors(context_dir: TempDir) {
        let path = create_context(
            &context_dir,
            ModelIdentity::known("fixture-model-blue".to_owned()).expect("valid model slug"),
            "fixture-tool-use-blue",
        );
        let message = indoc! {"
            Fixture subject

            Fixture body.

            Co-authored-by: Example Person <person@example.invalid>
            AI-Tool: Old Tool
            AI-Model: fixture-model-red
        "};

        assert_eq!(
            prepare(&path, message).expect("prepare attributed message"),
            indoc! {"
                Fixture subject

                Fixture body.

                Co-authored-by: Example Person <person@example.invalid>
                AI-Tool: Codex
                AI-Model: fixture-model-blue
            "},
        );
    }

    #[rstest]
    fn explicit_unknown_reason_is_not_written_to_the_commit_message(context_dir: TempDir) {
        let path = create_context(
            &context_dir,
            ModelIdentity::Unknown {
                reason: "fixture transcript delay".to_owned(),
            },
            "fixture-tool-use-unknown",
        );

        assert_eq!(
            prepare(&path, indoc! {"Fixture subject"}).expect("prepare unknown attribution"),
            indoc! {"
                Fixture subject

                AI-Tool: Codex
                AI-Model: unknown
            "},
        );
    }

    #[rstest]
    fn operation_context_can_be_claimed_for_only_one_commit(context_dir: TempDir) {
        let path = create_context(
            &context_dir,
            ModelIdentity::known("fixture-model-red".to_owned()).expect("valid model slug"),
            "fixture-tool-use-red",
        );
        let first = prepare(&path, indoc! {"Fixture subject"}).map_err(|error| error.to_string());
        let second = prepare(&path, indoc! {"Fixture subject"}).map_err(|error| {
            error.to_string().replace(
                path.to_str().expect("context path is UTF-8"),
                "<context-file>",
            )
        });

        assert_eq!(
            (first, second),
            (
                Ok(indoc! {"
                    Fixture subject

                    AI-Tool: Codex
                    AI-Model: fixture-model-red
                "}
                .to_owned()),
                Err("attribution context was already claimed or could not be claimed: <context-file>"
                    .to_owned()),
            ),
        );
    }
}
