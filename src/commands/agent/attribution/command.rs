use std::path::Path;

use super::{CONTEXT_ENV, FINAL_OBJECT_REQUIRED_ENV, REQUIRED_ENV};
use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GitOperation {
    Commit,
    CherryPick,
}

pub(super) fn inject_context(command: &str, context_path: &Path) -> Result<String> {
    inject_context_with_final_object(command, context_path, false)
}

pub(super) fn inject_context_with_final_object(
    command: &str,
    context_path: &Path,
    final_object: bool,
) -> Result<String> {
    let context_path = context_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("attribution context path is not valid UTF-8"))?;
    let context_assignment = format!("{CONTEXT_ENV}={}", shlex::try_quote(context_path)?);
    let required_assignment = format!("{REQUIRED_ENV}=1");
    let final_object_assignment = final_object.then(|| format!("{FINAL_OBJECT_REQUIRED_ENV}=1"));
    let assignments = std::iter::once(context_assignment.as_str())
        .chain(std::iter::once(required_assignment.as_str()))
        .chain(final_object_assignment.as_deref())
        .collect::<Vec<_>>()
        .join(" ");
    let words = shlex::split(command)
        .ok_or_else(|| anyhow::anyhow!("commit command has invalid shell quoting"))?;

    if let Some(index) = runok_command_index(&words) {
        let delimiter = words[index + 2..]
            .iter()
            .position(|word| word == "--")
            .map(|offset| index + 2 + offset)
            .ok_or_else(|| anyhow::anyhow!("runok command has no command delimiter"))?;
        let target = &words[delimiter + 1..];
        if target.len() != 1 {
            anyhow::bail!("attribution supports runok wrappers with one shell-command argument");
        }
        let wrapped = format!("{assignments} {}", target[0]);
        let mut rewritten = words;
        rewritten[delimiter + 1] = wrapped;
        return shlex::try_join(rewritten.iter().map(String::as_str))
            .map_err(|_| anyhow::anyhow!("failed to quote the transformed runok command"));
    }

    Ok(format!("{assignments} {command}"))
}

pub(super) fn classify_command(command: &str) -> Result<Option<GitOperation>> {
    classify_command_at_depth(command, 0)
}

pub(super) fn runok_command_index(words: &[String]) -> Option<usize> {
    let index = if words
        .first()
        .is_some_and(|word| executable_name(word) == "runok")
    {
        0
    } else if words.len() > 1
        && executable_name(&words[0]) == "a"
        && executable_name(&words[1]) == "runok"
    {
        1
    } else {
        return None;
    };
    (words.get(index + 1).map(String::as_str) == Some("exec")).then_some(index)
}

fn classify_command_at_depth(command: &str, depth: usize) -> Result<Option<GitOperation>> {
    if depth > 4 {
        return Ok(None);
    }
    let (segments, has_control_operator) = split_shell_control_operators(command);
    if has_control_operator {
        let mut contains_commit = false;
        for segment in segments {
            if classify_shell_segment(segment, depth + 1)?.is_some() {
                contains_commit = true;
            }
        }
        return if contains_commit {
            anyhow::bail!("compound commit commands are not supported by attribution")
        } else {
            Ok(None)
        };
    }
    let words = shlex::split(command)
        .ok_or_else(|| anyhow::anyhow!("commit command has invalid shell quoting"))?;
    classify_words(&words, depth)
}

fn classify_shell_segment(segment: &str, depth: usize) -> Result<Option<GitOperation>> {
    let words = shlex::split(segment)
        .ok_or_else(|| anyhow::anyhow!("commit command has invalid shell quoting"))?;
    let words = match words.first().map(String::as_str) {
        Some("if" | "then" | "elif" | "else" | "while" | "until" | "do") => &words[1..],
        Some("fi" | "done" | "esac") => return Ok(None),
        _ => &words,
    };
    classify_words(words, depth)
}

fn classify_words(words: &[String], depth: usize) -> Result<Option<GitOperation>> {
    if depth > 4 || words.is_empty() {
        return Ok(None);
    }
    let assignments = words
        .iter()
        .take_while(|word| is_shell_assignment(word))
        .count();
    let assigned_hook_override = words[..assignments]
        .iter()
        .any(|word| is_git_config_assignment(word));
    let assigned_context_override = words[..assignments]
        .iter()
        .any(|word| is_attribution_assignment(word));
    let words = &words[assignments..];
    if words.is_empty() {
        return Ok(None);
    }
    if let Some(index) = runok_command_index(words) {
        let delimiter = words[index + 2..]
            .iter()
            .position(|word| word == "--")
            .map(|offset| index + 2 + offset)
            .ok_or_else(|| anyhow::anyhow!("runok command has no command delimiter"))?;
        let target = &words[delimiter + 1..];
        let operation = if target.len() == 1 {
            classify_command_at_depth(&target[0], depth + 1)
        } else {
            classify_words(target, depth + 1)
        }?;
        if operation.is_some() && assigned_hook_override {
            anyhow::bail!("commit command overrides Git configuration for its hooks");
        }
        if operation.is_some() && assigned_context_override {
            anyhow::bail!("commit command overrides its attribution context");
        }
        return Ok(operation);
    }

    let operation = match executable_name(&words[0]) {
        "a" if words.get(1).is_some_and(|word| word == "runok") => {
            classify_words(&words[1..], depth + 1)?
        }
        "rtk" => classify_words(&words[1..], depth + 1)?,
        "sh" | "bash" | "zsh" => {
            if let Some(index) = words.iter().position(|word| word == "-c" || word == "-lc") {
                if words[..index]
                    .iter()
                    .any(|word| word == "-i" || word == "-li")
                {
                    anyhow::bail!("interactive shells cannot carry an operation context");
                }
                words.get(index + 1).map_or(Ok(None), |command| {
                    classify_command_at_depth(command, depth + 1)
                })?
            } else {
                None
            }
        }
        "env" => classify_env(words, depth + 1)?,
        "sudo" => {
            let command = words.iter().position(|word| executable_name(word) == "git");
            if let Some(index) = command {
                let operation = classify_git(&words[index..])?;
                if operation.is_some() {
                    anyhow::bail!(
                        "sudo commit commands cannot preserve scoped attribution context"
                    );
                }
            }
            None
        }
        "git" => classify_git(words)?,
        _ => None,
    };
    if operation.is_some() && assigned_hook_override {
        anyhow::bail!("commit command overrides Git configuration for its hooks");
    }
    if operation.is_some() && assigned_context_override {
        anyhow::bail!("commit command overrides its attribution context");
    }
    Ok(operation)
}

fn classify_env(words: &[String], depth: usize) -> Result<Option<GitOperation>> {
    let mut index = 1;
    let mut overrides_hooks = false;
    let mut changes_attribution_context = false;
    while let Some(argument) = words.get(index).map(String::as_str) {
        match argument {
            "--" => {
                let operation = classify_words(&words[index + 1..], depth)?;
                return validate_env_operation(
                    operation,
                    overrides_hooks,
                    changes_attribution_context,
                );
            }
            "-i" | "--ignore-environment" => {
                changes_attribution_context = true;
                index += 1;
            }
            "-0" | "--null" => index += 1,
            "-u" | "--unset" => {
                changes_attribution_context |= words
                    .get(index + 1)
                    .is_some_and(|name| is_attribution_variable(name));
                index += 2;
            }
            "-C" | "--chdir" => index += 2,
            "-S" | "--split-string" => {
                let operation = words.get(index + 1).map_or(Ok(None), |command| {
                    classify_command_at_depth(command, depth)
                })?;
                return validate_env_operation(
                    operation,
                    overrides_hooks,
                    changes_attribution_context,
                );
            }
            _ if let Some(name) = argument.strip_prefix("--unset=") => {
                changes_attribution_context |= is_attribution_variable(name);
                index += 1;
            }
            _ if argument.starts_with("--chdir=") => index += 1,
            _ if let Some(command) = argument.strip_prefix("--split-string=") => {
                let operation = classify_command_at_depth(command, depth)?;
                return validate_env_operation(
                    operation,
                    overrides_hooks,
                    changes_attribution_context,
                );
            }
            _ if argument.starts_with("-u") && argument.len() > 2 => {
                changes_attribution_context |= is_attribution_variable(&argument[2..]);
                index += 1;
            }
            _ if is_shell_assignment(argument) => {
                overrides_hooks |= is_git_config_assignment(argument);
                changes_attribution_context |= is_attribution_assignment(argument);
                index += 1;
            }
            _ if argument.starts_with('-') => index += 1,
            _ => {
                let operation = classify_words(&words[index..], depth)?;
                return validate_env_operation(
                    operation,
                    overrides_hooks,
                    changes_attribution_context,
                );
            }
        }
    }
    Ok(None)
}

fn validate_env_operation(
    operation: Option<GitOperation>,
    overrides_hooks: bool,
    changes_attribution_context: bool,
) -> Result<Option<GitOperation>> {
    if operation.is_some() && overrides_hooks {
        anyhow::bail!("commit command overrides Git configuration for its hooks");
    }
    if operation.is_some() && changes_attribution_context {
        anyhow::bail!("commit command overrides its attribution context");
    }
    Ok(operation)
}

fn is_attribution_assignment(word: &str) -> bool {
    word.split_once('=')
        .is_some_and(|(name, _)| is_attribution_variable(name))
}

fn is_attribution_variable(name: &str) -> bool {
    matches!(
        name,
        super::CONTEXT_ENV | super::REQUIRED_ENV | super::FINAL_OBJECT_REQUIRED_ENV
    )
}

pub(super) fn git_subcommand_index(words: &[String]) -> usize {
    let mut index = 1;
    while let Some(option) = words.get(index).map(String::as_str) {
        if !option.starts_with('-') {
            break;
        }
        match option {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path"
            | "--super-prefix" | "--config-env" => index += 2,
            _ if option.starts_with("--git-dir=")
                || option.starts_with("--work-tree=")
                || option.starts_with("--namespace=")
                || option.starts_with("--exec-path=") =>
            {
                index += 1;
            }
            _ if option.starts_with("-c") && option.len() > 2 => index += 1,
            _ if option.starts_with("--config-env=") => index += 1,
            "--no-pager"
            | "--paginate"
            | "--no-optional-locks"
            | "--literal-pathspecs"
            | "--glob-pathspecs"
            | "--noglob-pathspecs" => index += 1,
            _ => break,
        }
    }
    index
}

fn classify_git(words: &[String]) -> Result<Option<GitOperation>> {
    let index = git_subcommand_index(words);
    let hooks_path_override = git_hooks_path_override(words, index);
    let Some(subcommand) = words.get(index).map(String::as_str) else {
        return Ok(None);
    };
    match subcommand {
        "commit" => {
            if hooks_path_override
                || words[index + 1..]
                    .iter()
                    .any(|word| word == "--no-verify" || word == "-n")
            {
                anyhow::bail!("commit command bypasses the attribution integrity hook");
            }
            Ok(Some(GitOperation::Commit))
        }
        "cherry-pick" => {
            let operation = classify_cherry_pick(&words[index + 1..])?;
            if operation.is_some() && hooks_path_override {
                anyhow::bail!("commit command overrides Git configuration for its hooks");
            }
            Ok(operation)
        }
        "commit-tree" | "rebase" | "am" | "merge" | "revert" | "pull" => {
            if subcommand == "merge" && words[index + 1..].iter().any(|word| word == "--squash") {
                return Ok(None);
            }
            anyhow::bail!(
                "git {subcommand} may create commits outside the supported single-commit hook path"
            )
        }
        _ => Ok(None),
    }
}

fn git_hooks_path_override(words: &[String], end_index: usize) -> bool {
    let mut index = 1;
    let mut overridden = false;
    while index < end_index {
        match words[index].as_str() {
            "-c" | "--config-env" => {
                let value = words.get(index + 1).map(String::as_str).unwrap_or_default();
                overridden |= is_hooks_path_assignment(value);
                index += 2;
            }
            option if option.starts_with("-c") && option.len() > 2 => {
                overridden |= is_hooks_path_assignment(&option[2..]);
                index += 1;
            }
            option if option.starts_with("--config-env=") => {
                overridden |= is_hooks_path_assignment(&option["--config-env=".len()..]);
                index += 1;
            }
            "-C" | "--git-dir" | "--work-tree" | "--namespace" | "--exec-path"
            | "--super-prefix" => index += 2,
            _ => index += 1,
        }
    }
    overridden
}

fn classify_cherry_pick(arguments: &[String]) -> Result<Option<GitOperation>> {
    if arguments
        .iter()
        .any(|word| word == "--no-commit" || word == "-n")
    {
        return Ok(None);
    }
    if arguments
        .iter()
        .any(|word| word == "--abort" || word == "--skip")
    {
        anyhow::bail!("cherry-pick continuation state is not a single verifiable commit operation");
    }
    if arguments.iter().any(|word| word == "--continue") {
        anyhow::bail!("cherry-pick --continue may create multiple commits and is not supported");
    }
    let mut operands = 0usize;
    let mut skip_next = false;
    let mut after_separator = false;
    for argument in arguments {
        if skip_next {
            skip_next = false;
            continue;
        }
        if argument == "--" {
            after_separator = true;
            continue;
        }
        if !after_separator && argument.starts_with('-') {
            if matches!(
                argument.as_str(),
                "-m" | "--mainline" | "--strategy" | "--strategy-option"
            ) {
                skip_next = true;
            }
            continue;
        }
        operands += 1;
    }
    match operands {
        0 => Ok(None),
        1 => Ok(Some(GitOperation::CherryPick)),
        _ => anyhow::bail!("multiple cherry-pick commits require separate attributed operations"),
    }
}

pub(super) fn executable_name(word: &str) -> &str {
    Path::new(word)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(word)
}

pub(super) fn is_shell_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut characters = name.bytes();
    characters
        .next()
        .is_some_and(|character| character == b'_' || character.is_ascii_alphabetic())
        && characters.all(|character| character == b'_' || character.is_ascii_alphanumeric())
}

fn is_git_config_assignment(word: &str) -> bool {
    word.split_once('=')
        .is_some_and(|(name, _)| name.starts_with("GIT_CONFIG_"))
}

fn is_hooks_path_assignment(value: &str) -> bool {
    value
        .split_once('=')
        .is_some_and(|(key, _)| key.eq_ignore_ascii_case("core.hooksPath"))
}

pub(super) fn split_shell_control_operators(command: &str) -> (Vec<&str>, bool) {
    let bytes = command.as_bytes();
    let mut segments = Vec::new();
    let mut start = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut found_operator = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == Some(b'\'') {
            if byte == b'\'' {
                quote = None;
            }
            continue;
        }
        if quote == Some(b'"') {
            if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quote = None;
            }
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'\'' | b'"' => quote = Some(byte),
            b';' | b'&' | b'|' | b'(' | b')' | b'{' | b'}' | b'\n' => {
                segments.push(&command[start..index]);
                start = index + 1;
                found_operator = true;
            }
            _ => {}
        }
    }
    segments.push(&command[start..]);
    (segments, found_operator)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{GitOperation, classify_command, inject_context_with_final_object};
    use rstest::rstest;

    #[rstest]
    #[case::amend("git commit --amend -m 'Fixture subject'", Some(GitOperation::Commit))]
    #[case::squash_commit("git commit -m 'Fixture squash subject'", Some(GitOperation::Commit))]
    #[case::single_cherry_pick("git cherry-pick fixture-revision", Some(GitOperation::CherryPick))]
    #[case::env_split_string("env -S 'git commit -m Fixture subject'", Some(GitOperation::Commit))]
    #[case::squash_merge_only("git merge --squash fixture-branch", None)]
    fn classify_supported_commit_operations(
        #[case] command: &str,
        #[case] expected: Option<GitOperation>,
    ) {
        assert_eq!(
            classify_command(command).expect("classify command"),
            expected
        );
    }

    #[rstest]
    fn multi_commit_cherry_pick_continuation_is_rejected() {
        let error = classify_command("git cherry-pick --continue")
            .expect_err("continuation may create multiple commits");

        assert_eq!(
            error.to_string(),
            "cherry-pick --continue may create multiple commits and is not supported",
        );
    }

    #[rstest]
    #[case::attached_semicolon("git commit -m 'Fixture subject';")]
    #[case::inline_chain("git commit -m 'Fixture subject' && git status")]
    #[case::if_then("if true; then git commit -m 'Fixture subject'; fi")]
    #[case::if_condition("if git commit -m 'Fixture subject'; then true; fi")]
    fn compound_commit_commands_are_rejected(#[case] command: &str) {
        let error = classify_command(command).expect_err("compound commit must fail closed");

        assert_eq!(
            error.to_string(),
            "compound commit commands are not supported by attribution",
        );
    }

    #[rstest]
    fn shell_punctuation_inside_a_quoted_commit_subject_is_not_an_operator() {
        assert_eq!(
            classify_command("git commit -m 'Fixture; subject'").expect("classify commit"),
            Some(GitOperation::Commit),
        );
    }

    #[rstest]
    fn final_object_marker_stays_inside_the_runok_git_command() {
        let command =
            r#"runok exec --ask --hook-origin fixture-token -- 'git commit -m "Fixture subject"'"#;
        let injected =
            inject_context_with_final_object(command, Path::new("/fixture/context"), true)
                .expect("inject final-object context");

        assert_eq!(
            injected,
            r#"runok exec --ask --hook-origin fixture-token -- 'ARMYKNIFE_AGENT_ATTRIBUTION_CONTEXT=/fixture/context ARMYKNIFE_AGENT_ATTRIBUTION_REQUIRED=1 ARMYKNIFE_AGENT_ATTRIBUTION_FINAL_OBJECT_REQUIRED=1 git commit -m "Fixture subject"'"#,
        );
    }

    #[rstest]
    #[case::inline_assignment(
        "GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=/fixture/hooks git commit -m 'Fixture subject'"
    )]
    #[case::env_assignment(
        "env GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=/fixture/hooks git commit -m 'Fixture subject'"
    )]
    #[case::cherry_pick("git -c core.hooksPath=/fixture/hooks cherry-pick fixture-revision")]
    fn hook_path_environment_overrides_are_rejected(#[case] command: &str) {
        let error = classify_command(command).expect_err("Git config override must fail closed");

        assert_eq!(
            error.to_string(),
            "commit command overrides Git configuration for its hooks",
        );
    }

    #[rstest]
    #[case::merge("git pull origin fixture-branch")]
    #[case::fast_forward_only("git pull --ff-only origin fixture-branch")]
    fn pull_is_rejected_as_a_commit_producing_operation(#[case] command: &str) {
        let error = classify_command(command).expect_err("pull may create a commit");

        assert_eq!(
            error.to_string(),
            "git pull may create commits outside the supported single-commit hook path",
        );
    }

    #[rstest]
    #[case::inline_assignment(
        "ARMYKNIFE_AGENT_ATTRIBUTION_CONTEXT=/fixture/context git commit -m 'Fixture subject'"
    )]
    #[case::env_assignment(
        "env ARMYKNIFE_AGENT_ATTRIBUTION_REQUIRED=1 git commit -m 'Fixture subject'"
    )]
    #[case::final_object_assignment(
        "ARMYKNIFE_AGENT_ATTRIBUTION_FINAL_OBJECT_REQUIRED=1 git commit -m 'Fixture subject'"
    )]
    #[case::ignore_environment("env -i git commit -m 'Fixture subject'")]
    #[case::unset_context(
        "env -u ARMYKNIFE_AGENT_ATTRIBUTION_CONTEXT git commit -m 'Fixture subject'"
    )]
    #[case::unset_final_object_marker(
        "env -u ARMYKNIFE_AGENT_ATTRIBUTION_FINAL_OBJECT_REQUIRED git commit -m 'Fixture subject'"
    )]
    fn context_overrides_are_rejected(#[case] command: &str) {
        let error = classify_command(command).expect_err("context override must fail closed");

        assert_eq!(
            error.to_string(),
            "commit command overrides its attribution context",
        );
    }
}
