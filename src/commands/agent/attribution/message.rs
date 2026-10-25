use std::path::Path;

use anyhow::Result;

use super::context::{self, OperationContext};

const TOOL_TRAILER: &str = "AI-Tool";
const MODEL_TRAILER: &str = "AI-Model";

pub(super) fn prepare(path: &Path, message: &str) -> Result<String> {
    let context = context::claim(path, unix_timestamp()?)?;
    add_trailers(message, &context)
}

pub(super) fn verify_and_consume(path: &Path, message: &str) -> Result<()> {
    let check =
        context::claimed_context(path).and_then(|context| verify_trailers(message, &context));
    let cleanup = context::consume(path);
    match (check, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(check_error), Ok(())) => Err(check_error),
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(check_error), Err(cleanup_error)) => Err(anyhow::anyhow!(
            "{check_error:#}; failed to consume attribution context: {cleanup_error:#}"
        )),
    }
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

fn verify_trailers(message: &str, context: &OperationContext) -> Result<()> {
    let lines = normalized_lines(message);
    let Some(start) = trailer_block_start(&lines) else {
        anyhow::bail!("commit message has no managed attribution trailer block");
    };
    let managed: Vec<&str> = lines[start..]
        .iter()
        .filter(|line| !is_continuation(line))
        .filter(|line| is_managed_trailer(line))
        .map(String::as_str)
        .collect();
    let expected_tool = format!("{TOOL_TRAILER}: {}", context.tool());
    let expected_model = format!("{MODEL_TRAILER}: {}", context.model().trailer_value());
    let footer_end = lines[start..]
        .iter()
        .rposition(|line| !line.is_empty())
        .map(|offset| start + offset)
        .ok_or_else(|| anyhow::anyhow!("commit message has an empty trailer block"))?;
    if managed != [expected_tool.as_str(), expected_model.as_str()]
        || footer_end < start + 1
        || lines[footer_end - 1] != expected_tool
        || lines[footer_end] != expected_model
    {
        anyhow::bail!("commit message attribution does not match its operation context");
    }
    Ok(())
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

fn unix_timestamp() -> Result<i64> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| anyhow::anyhow!("system clock is before the Unix epoch: {error}"))?;
    i64::try_from(elapsed.as_secs())
        .map_err(|error| anyhow::anyhow!("invalid system timestamp: {error}"))
}

#[cfg(test)]
mod tests {
    use super::super::Provider;
    use super::{prepare, verify_and_consume};
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
            prepare(&path, "Fixture subject\n").expect("prepare unknown attribution"),
            indoc! {"
                Fixture subject

                AI-Tool: Codex
                AI-Model: unknown
            "},
        );
    }

    #[rstest]
    fn verify_accepts_generated_trailers_and_consumes_context(context_dir: TempDir) {
        let path = create_context(
            &context_dir,
            ModelIdentity::known("fixture-model-red".to_owned()).expect("valid model slug"),
            "fixture-tool-use-red",
        );
        let message = prepare(&path, "Fixture subject\n").expect("prepare commit message");
        let result = verify_and_consume(&path, &message).map_err(|error| error.to_string());

        assert_eq!((result, path.exists()), (Ok(()), false));
    }

    #[rstest]
    fn verify_rejects_changed_model_trailer_and_consumes_context(context_dir: TempDir) {
        let path = create_context(
            &context_dir,
            ModelIdentity::known("fixture-model-red".to_owned()).expect("valid model slug"),
            "fixture-tool-use-red",
        );
        let prepared = prepare(&path, "Fixture subject\n").expect("prepare commit message");
        let edited = prepared.replace("fixture-model-red", "fixture-model-blue");
        let result = verify_and_consume(&path, &edited).map_err(|error| error.to_string());

        assert_eq!(
            (result, path.exists()),
            (
                Err("commit message attribution does not match its operation context".to_owned()),
                false,
            ),
        );
    }
}
