use std::path::Path;

use super::CONTEXT_ENV;
use anyhow::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GitOperation {
    Commit,
    CherryPick,
}

pub(super) fn inject_context(command: &str, context_path: &Path) -> Result<String> {
    let context_path = context_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("attribution context path is not valid UTF-8"))?;
    let context_assignment = format!("{CONTEXT_ENV}={}", shlex::try_quote(context_path)?);
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
        let wrapped = format!("{context_assignment} {}", target[0]);
        let mut rewritten = words;
        rewritten[delimiter + 1] = wrapped;
        return shlex::try_join(rewritten.iter().map(String::as_str))
            .map_err(|_| anyhow::anyhow!("failed to quote the transformed runok command"));
    }

    Ok(format!("{context_assignment} {command}"))
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
    if has_shell_control_operator(command) {
        return Ok(None);
    }
    let words = shlex::split(command)
        .ok_or_else(|| anyhow::anyhow!("commit command has invalid shell quoting"))?;
    classify_words(&words, depth)
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
        if operation.is_some() && (assigned_hook_override || assigned_context_override) {
            return Ok(None);
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
                    return Ok(None);
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
                    return Ok(None);
                }
            }
            None
        }
        "git" => classify_git(words)?,
        _ => None,
    };
    if operation.is_some() && (assigned_hook_override || assigned_context_override) {
        return Ok(None);
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
        return Ok(None);
    }
    if operation.is_some() && changes_attribution_context {
        return Ok(None);
    }
    Ok(operation)
}

fn is_attribution_assignment(word: &str) -> bool {
    word.split_once('=')
        .is_some_and(|(name, _)| is_attribution_variable(name))
}

fn is_attribution_variable(name: &str) -> bool {
    name == super::CONTEXT_ENV
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
                return Ok(None);
            }
            Ok(Some(GitOperation::Commit))
        }
        "cherry-pick" => {
            let operation = classify_cherry_pick(&words[index + 1..])?;
            if operation.is_some() && hooks_path_override {
                return Ok(None);
            }
            Ok(operation)
        }
        "commit-tree" | "rebase" | "am" | "merge" | "revert" | "pull" => Ok(None),
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
        return Ok(None);
    }
    if arguments.iter().any(|word| word == "--continue") {
        return Ok(None);
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
        _ => Ok(None),
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

fn has_shell_control_operator(command: &str) -> bool {
    let bytes = command.as_bytes();
    let mut quote = None;
    let mut escaped = false;
    for byte in bytes.iter().copied() {
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
            b';' | b'&' | b'|' | b'(' | b')' | b'{' | b'}' | b'\n' => return true,
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{GitOperation, classify_command, inject_context};
    use rstest::rstest;

    #[rstest]
    #[case::amend("git commit --amend -m 'Fixture subject'", Some(GitOperation::Commit))]
    #[case::squash_commit("git commit -m 'Fixture squash subject'", Some(GitOperation::Commit))]
    #[case::single_cherry_pick("git cherry-pick fixture-revision", Some(GitOperation::CherryPick))]
    #[case::env_split_string("env -S 'git commit -m Fixture subject'", Some(GitOperation::Commit))]
    #[case::squash_merge("git merge --squash fixture-branch", None)]
    #[case::pull("git pull origin fixture-branch", None)]
    #[case::rebase("git rebase fixture-base", None)]
    #[case::cherry_pick_multiple("git cherry-pick fixture-one fixture-two", None)]
    #[case::cherry_pick_continue("git cherry-pick --continue", None)]
    #[case::compound_shell("git commit -m 'Fixture subject' && git status", None)]
    #[case::unknown_alias("git ci -m 'Fixture subject'", None)]
    fn classifies_supported_commit_operations_without_rejecting_other_git_commands(
        #[case] command: &str,
        #[case] expected: Option<GitOperation>,
    ) {
        assert_eq!(
            classify_command(command).expect("classify command"),
            expected
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
    fn context_injection_preserves_the_runok_ask_wrapper() {
        let command =
            r#"runok exec --ask --hook-origin fixture-token -- 'git commit -m "Fixture subject"'"#;
        let injected = inject_context(command, Path::new("/fixture/context"))
            .expect("inject attribution context");

        assert_eq!(
            injected,
            r#"runok exec --ask --hook-origin fixture-token -- 'ARMYKNIFE_AGENT_ATTRIBUTION_CONTEXT=/fixture/context git commit -m "Fixture subject"'"#,
        );
    }

    #[rstest]
    #[case::git_config_environment(
        "GIT_CONFIG_KEY_0=core.hooksPath GIT_CONFIG_VALUE_0=/fixture/hooks git commit -m 'Fixture subject'"
    )]
    #[case::attribution_context_environment(
        "ARMYKNIFE_AGENT_ATTRIBUTION_CONTEXT=/fixture/context git commit -m 'Fixture subject'"
    )]
    #[case::git_config_option("git -c core.hooksPath=/fixture/hooks cherry-pick fixture-revision")]
    fn command_overrides_skip_context_attribution(#[case] command: &str) {
        assert_eq!(classify_command(command).expect("classify command"), None);
    }
}
