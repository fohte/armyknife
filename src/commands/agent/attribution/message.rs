use anyhow::Result;

pub(super) fn prepare_with_trailer(message: &str, trailer: &str) -> Result<String> {
    add_coauthor_trailer(message, trailer)
}

fn add_coauthor_trailer(message: &str, trailer: &str) -> Result<String> {
    let mut lines = normalized_lines(message);
    trim_trailing_blank_lines(&mut lines);
    let mut removed_legacy_trailer = false;
    if let Some(start) = trailer_block_start(&lines) {
        let mut kept = Vec::with_capacity(lines.len());
        let mut remove_continuation = false;
        for (index, line) in lines.iter().enumerate() {
            if index < start {
                kept.push(line.clone());
                continue;
            }
            if is_legacy_attribution_trailer(line) {
                removed_legacy_trailer = true;
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
        trim_trailing_blank_lines(&mut lines);
    }

    let footer = trailer_block_start(&lines);
    if footer.is_some_and(|start| lines[start..].iter().any(|line| line == trailer)) {
        if !removed_legacy_trailer {
            return Ok(message.to_owned());
        }
        let mut output = lines.join("\n");
        if !output.is_empty() {
            output.push('\n');
        }
        return Ok(output);
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

fn is_legacy_attribution_trailer(line: &str) -> bool {
    let Some((token, _)) = line.split_once(": ") else {
        return false;
    };
    ["AI-Tool", "AI-Model", "AI-Model-Reason"]
        .iter()
        .any(|legacy| token.eq_ignore_ascii_case(legacy))
}

#[cfg(test)]
mod tests {
    use super::prepare_with_trailer;
    use indoc::indoc;
    use rstest::rstest;

    #[rstest]
    #[case::codex("Co-authored-by: Codex (fixture-model-blue) <noreply@openai.com>")]
    #[case::claude_code("Co-authored-by: Claude Code (fixture-model-blue) <noreply@anthropic.com>")]
    fn prepare_appends_provider_coauthor_and_cleans_legacy_trailers(#[case] trailer: &str) {
        let message = indoc! {"
            Fixture subject

            Fixture body.

            Co-authored-by: Example Person <person@example.invalid>
            AI-Tool: Legacy Tool
             Legacy continuation.
            AI-Model: fixture-model-green
            AI-Model-Reason: legacy fixture
            Signed-off-by: Example Maintainer <maintainer@example.invalid>
        "};
        let expected = format!(
            "{}{}\n",
            indoc! {"
                Fixture subject

                Fixture body.

                Co-authored-by: Example Person <person@example.invalid>
                Signed-off-by: Example Maintainer <maintainer@example.invalid>
            "},
            trailer,
        );

        assert_eq!(
            prepare_with_trailer(message, trailer).expect("prepare attributed message"),
            expected
        );
    }

    #[rstest]
    #[case::codex("Co-authored-by: Codex (fixture-model-blue) <noreply@openai.com>")]
    #[case::claude_code("Co-authored-by: Claude Code (fixture-model-blue) <noreply@anthropic.com>")]
    fn existing_identical_coauthor_is_not_duplicated(#[case] trailer: &str) {
        let message = format!(
            indoc! {"
                Fixture subject

                {}
            "},
            trailer,
        );

        assert_eq!(
            prepare_with_trailer(&message, trailer).expect("prepare attributed message"),
            message
        );
    }
}
