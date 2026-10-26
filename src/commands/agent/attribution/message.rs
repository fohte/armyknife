use std::path::Path;

use anyhow::Result;

use super::context::{self, unix_timestamp};

pub(super) fn prepare(path: &Path, message: &str) -> Result<String> {
    let context = context::claim(path, unix_timestamp()?)?;
    let Some(trailer) = context.coauthor_trailer() else {
        return Ok(message.to_owned());
    };
    add_coauthor_trailer(message, &trailer)
}

fn add_coauthor_trailer(message: &str, trailer: &str) -> Result<String> {
    let mut lines = normalized_lines(message);
    trim_trailing_blank_lines(&mut lines);
    let footer = trailer_block_start(&lines);
    if footer.is_some_and(|start| lines[start..].iter().any(|line| line == trailer)) {
        return Ok(message.to_owned());
    }

    let mut output = lines.join("\n");
    if !output.is_empty() {
        output.push('\n');
        if footer.is_none() {
            output.push('\n');
        }
    }
    output.push_str(trailer);
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
        provider: Provider,
        model: ModelIdentity,
        tool_use_id: &str,
    ) -> std::path::PathBuf {
        let context = OperationContext::new(provider, model, tool_use_id.to_owned())
            .expect("create fixture operation context");
        context::create(&context, directory.path()).expect("persist fixture operation context")
    }

    #[rstest]
    #[case::codex(
        Provider::Codex,
        "fixture-model-blue",
        "Co-authored-by: Codex (fixture-model-blue) <noreply@openai.com>"
    )]
    #[case::claude_code(
        Provider::ClaudeCode,
        "fixture-claude-model-blue",
        "Co-authored-by: Claude Code (fixture-claude-model-blue) <noreply@anthropic.com>"
    )]
    fn prepare_appends_provider_coauthor_and_preserves_existing_trailers(
        context_dir: TempDir,
        #[case] provider: Provider,
        #[case] model: &str,
        #[case] trailer: &str,
    ) {
        let path = create_context(
            &context_dir,
            provider,
            ModelIdentity::known(model.to_owned()).expect("valid model slug"),
            "fixture-tool-use-blue",
        );
        let message = indoc! {"
            Fixture subject

            Fixture body.

            Co-authored-by: Example Person <person@example.invalid>
            Co-authored-by: Example Tool (fixture-model-green) <tool@example.invalid>
            AI-Tool: Legacy Tool
            AI-Model: fixture-model-green
        "};
        let expected = format!("{message}{trailer}\n");

        assert_eq!(
            prepare(&path, message).expect("prepare attributed message"),
            expected
        );
    }

    #[rstest]
    #[case::codex(
        Provider::Codex,
        "fixture-model-blue",
        "Co-authored-by: Codex (fixture-model-blue) <noreply@openai.com>"
    )]
    #[case::claude_code(
        Provider::ClaudeCode,
        "fixture-claude-model-blue",
        "Co-authored-by: Claude Code (fixture-claude-model-blue) <noreply@anthropic.com>"
    )]
    fn existing_identical_coauthor_is_not_duplicated(
        context_dir: TempDir,
        #[case] provider: Provider,
        #[case] model: &str,
        #[case] trailer: &str,
    ) {
        let path = create_context(
            &context_dir,
            provider,
            ModelIdentity::known(model.to_owned()).expect("valid model slug"),
            "fixture-tool-use-existing-coauthor",
        );
        let mut message = indoc! {"
            Fixture subject

        "}
        .to_owned();
        message.push_str(trailer);
        message.push('\n');

        assert_eq!(
            prepare(&path, &message).expect("prepare attributed message"),
            message
        );
    }

    #[rstest]
    fn legacy_unknown_context_does_not_add_a_trailer(context_dir: TempDir) {
        let path = create_context(
            &context_dir,
            Provider::Codex,
            ModelIdentity::Unknown {
                reason: "fixture model was unavailable".to_owned(),
            },
            "fixture-tool-use-unknown",
        );
        let message = indoc! {"
            Fixture subject

            Co-authored-by: Example Person <person@example.invalid>
        "};

        assert_eq!(
            prepare(&path, message).expect("prepare message without model"),
            message
        );
    }

    #[rstest]
    fn operation_context_can_be_claimed_for_only_one_commit(context_dir: TempDir) {
        let path = create_context(
            &context_dir,
            Provider::Codex,
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

                    Co-authored-by: Codex (fixture-model-red) <noreply@openai.com>
                "}
                .to_owned()),
                Err("attribution context was already claimed or could not be claimed: <context-file>"
                    .to_owned()),
            ),
        );
    }
}
