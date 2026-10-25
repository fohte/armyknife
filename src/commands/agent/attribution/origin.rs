use anyhow::Result;

use super::PROVIDER_ORIGIN_ENV;
use super::command::{
    executable_name, git_subcommand_index, is_shell_assignment, runok_command_index,
    split_shell_control_operators,
};

pub(super) fn requires_marker(command: &str) -> Result<bool> {
    requires_marker_at_depth(command, 0)
}

pub(super) fn inject_marker(command: &str) -> Result<String> {
    let (segments, has_control_operator) = split_shell_control_operators(command);
    if has_control_operator {
        for segment in segments {
            if requires_marker(segment)? {
                anyhow::bail!(
                    "provider-origin markers cannot be carried through compound commands"
                );
            }
        }
        return Ok(command.to_owned());
    }

    let words = shlex::split(command)
        .ok_or_else(|| anyhow::anyhow!("provider command has invalid shell quoting"))?;
    inject_marker_into_words(command, words, 0)
}

fn is_known_git_subcommand(subcommand: &str) -> bool {
    matches!(
        subcommand,
        "add"
            | "am"
            | "annotate"
            | "apply"
            | "archive"
            | "bisect"
            | "blame"
            | "branch"
            | "bundle"
            | "cat-file"
            | "check-attr"
            | "check-ignore"
            | "check-mailmap"
            | "check-ref-format"
            | "checkout"
            | "cherry"
            | "cherry-pick"
            | "clean"
            | "clone"
            | "commit"
            | "commit-graph"
            | "commit-tree"
            | "config"
            | "count-objects"
            | "credential"
            | "describe"
            | "diff"
            | "diff-files"
            | "diff-index"
            | "diff-tree"
            | "fetch"
            | "for-each-ref"
            | "format-patch"
            | "fsck"
            | "gc"
            | "grep"
            | "help"
            | "index-pack"
            | "init"
            | "interpret-trailers"
            | "log"
            | "ls-files"
            | "ls-remote"
            | "ls-tree"
            | "mailinfo"
            | "mailsplit"
            | "merge"
            | "merge-base"
            | "merge-file"
            | "merge-index"
            | "merge-tree"
            | "mktag"
            | "mktree"
            | "mv"
            | "name-rev"
            | "notes"
            | "pack-refs"
            | "patch-id"
            | "prune"
            | "pull"
            | "push"
            | "range-diff"
            | "read-tree"
            | "rebase"
            | "reflog"
            | "remote"
            | "repack"
            | "replace"
            | "request-pull"
            | "rerere"
            | "reset"
            | "restore"
            | "revert"
            | "rm"
            | "send-email"
            | "shortlog"
            | "show"
            | "show-branch"
            | "show-ref"
            | "sparse-checkout"
            | "stash"
            | "status"
            | "submodule"
            | "switch"
            | "symbolic-ref"
            | "tag"
            | "unpack-objects"
            | "update-index"
            | "update-ref"
            | "verify-commit"
            | "verify-pack"
            | "verify-tag"
            | "version"
            | "whatchanged"
            | "worktree"
            | "write-tree"
    )
}

fn requires_marker_at_depth(command: &str, depth: usize) -> Result<bool> {
    let (segments, has_control_operator) = split_shell_control_operators(command);
    if has_control_operator {
        for segment in segments {
            if requires_marker_at_depth(segment, depth + 1)? {
                return Ok(true);
            }
        }
        return Ok(false);
    }

    let words = shlex::split(command)
        .ok_or_else(|| anyhow::anyhow!("provider command has invalid shell quoting"))?;
    requires_marker_in_words(&words, depth)
}

fn requires_marker_in_words(words: &[String], depth: usize) -> Result<bool> {
    if words.is_empty() {
        return Ok(false);
    }
    if depth > 4 {
        anyhow::bail!("provider-origin command nesting exceeds the supported limit");
    }

    let assignments = words
        .iter()
        .take_while(|word| is_shell_assignment(word))
        .count();
    let words = &words[assignments..];
    if words.is_empty() {
        return Ok(false);
    }
    if let Some(index) = runok_command_index(words) {
        let delimiter = words[index + 2..]
            .iter()
            .position(|word| word == "--")
            .map(|offset| index + 2 + offset)
            .ok_or_else(|| anyhow::anyhow!("runok command has no command delimiter"))?;
        let target = &words[delimiter + 1..];
        return if target.len() == 1 {
            requires_marker_at_depth(&target[0], depth + 1)
        } else {
            requires_marker_in_words(target, depth + 1)
        };
    }

    match executable_name(&words[0]) {
        "a" if words.get(1).is_some_and(|word| word == "runok") => {
            requires_marker_in_words(&words[1..], depth + 1)
        }
        "rtk" => requires_marker_in_words(&words[1..], depth + 1),
        "env" => requires_marker_in_env(words, depth + 1),
        "sh" | "bash" | "zsh" => {
            let command_index = words.iter().position(|word| word == "-c" || word == "-lc");
            if words.iter().any(|word| word == "-i" || word == "-li") || command_index.is_none() {
                return Ok(true);
            }
            match command_index.and_then(|index| words.get(index + 1)) {
                Some(command) => requires_marker_at_depth(command, depth + 1),
                None => Ok(false),
            }
        }
        "sudo" => {
            let Some(index) = words.iter().position(|word| executable_name(word) == "git") else {
                return Ok(false);
            };
            let git_words = &words[index..];
            let subcommand_index = git_subcommand_index(git_words);
            Ok(git_words
                .get(subcommand_index)
                .is_some_and(|subcommand| !is_known_git_subcommand(subcommand)))
        }
        "git" => {
            let subcommand_index = git_subcommand_index(words);
            Ok(words
                .get(subcommand_index)
                .is_some_and(|subcommand| !is_known_git_subcommand(subcommand)))
        }
        _ => Ok(false),
    }
}

fn requires_marker_in_env(words: &[String], depth: usize) -> Result<bool> {
    let mut index = 1;
    while let Some(argument) = words.get(index).map(String::as_str) {
        match argument {
            "--" => return requires_marker_in_words(&words[index + 1..], depth),
            "-S" | "--split-string" => {
                return words.get(index + 1).map_or(Ok(false), |command| {
                    requires_marker_at_depth(command, depth + 1)
                });
            }
            "-u" | "--unset" | "-C" | "--chdir" => index += 2,
            "-i" | "--ignore-environment" | "-0" | "--null" => index += 1,
            _ if argument.starts_with("--split-string=") => {
                return requires_marker_at_depth(
                    argument.trim_start_matches("--split-string="),
                    depth + 1,
                );
            }
            _ if argument.starts_with("--unset=") || argument.starts_with("--chdir=") => {
                index += 1;
            }
            _ if argument.starts_with("-u") && argument.len() > 2 => index += 1,
            _ if is_shell_assignment(argument) || argument.starts_with('-') => index += 1,
            _ => return requires_marker_in_words(&words[index..], depth),
        }
    }
    Ok(false)
}

fn inject_marker_into_words(command: &str, mut words: Vec<String>, depth: usize) -> Result<String> {
    if words.is_empty() {
        anyhow::bail!("provider-origin marker has no command to wrap");
    }
    if depth > 4 {
        anyhow::bail!("provider-origin command nesting exceeds the supported limit");
    }

    let assignments = words
        .iter()
        .take_while(|word| is_shell_assignment(word))
        .count();
    let executable_index = assignments;
    let executable = words
        .get(executable_index)
        .map(|word| executable_name(word).to_owned())
        .unwrap_or_default();

    if let Some(index) = runok_command_index(&words[executable_index..]) {
        let index = executable_index + index;
        let delimiter = words[index + 2..]
            .iter()
            .position(|word| word == "--")
            .map(|offset| index + 2 + offset)
            .ok_or_else(|| anyhow::anyhow!("runok command has no command delimiter"))?;
        let target = &mut words[delimiter + 1..];
        if target.len() != 1 {
            anyhow::bail!("provider-origin markers support runok wrappers with one shell command");
        }
        target[0] = inject_marker(&target[0])?;
        return shlex::try_join(words.iter().map(String::as_str))
            .map_err(|_| anyhow::anyhow!("failed to quote provider-origin command"));
    }

    if executable == "env" {
        return inject_marker_into_env(words, executable_index);
    }
    if executable == "a"
        && words
            .get(executable_index + 1)
            .is_some_and(|word| word == "runok")
    {
        let inner = shlex::try_join(words[executable_index + 1..].iter().map(String::as_str))
            .map_err(|_| anyhow::anyhow!("failed to quote nested runok command"))?;
        let rewritten = inject_marker(&inner)?;
        let inner_words = shlex::split(&rewritten)
            .ok_or_else(|| anyhow::anyhow!("provider-origin runok command has invalid quoting"))?;
        words.splice(executable_index + 1.., inner_words);
        return shlex::try_join(words.iter().map(String::as_str))
            .map_err(|_| anyhow::anyhow!("failed to quote provider-origin command"));
    }
    if matches!(executable.as_str(), "sh" | "bash" | "zsh")
        && let Some(index) = words[executable_index..]
            .iter()
            .position(|word| word == "-c" || word == "-lc")
    {
        let index = executable_index + index;
        if let Some(script) = words.get_mut(index + 1) {
            *script = inject_marker(script)?;
            return shlex::try_join(words.iter().map(String::as_str))
                .map_err(|_| anyhow::anyhow!("failed to quote provider-origin command"));
        }
    }
    if executable == "sudo" {
        anyhow::bail!("provider-origin markers cannot be carried through sudo");
    }

    Ok(format!("{PROVIDER_ORIGIN_ENV}=1 {command}"))
}

fn inject_marker_into_env(mut words: Vec<String>, env_index: usize) -> Result<String> {
    let mut index = env_index + 1;
    while let Some(argument) = words.get(index).map(String::as_str) {
        match argument {
            "--" => {
                words.insert(index + 1, format!("{PROVIDER_ORIGIN_ENV}=1"));
                return shlex::try_join(words.iter().map(String::as_str))
                    .map_err(|_| anyhow::anyhow!("failed to quote provider-origin command"));
            }
            "-S" | "--split-string" => {
                let script = words
                    .get_mut(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("env split-string has no command"))?;
                *script = inject_marker(script)?;
                return shlex::try_join(words.iter().map(String::as_str))
                    .map_err(|_| anyhow::anyhow!("failed to quote provider-origin command"));
            }
            "-u" | "--unset" | "-C" | "--chdir" => index += 2,
            "-i" | "--ignore-environment" | "-0" | "--null" => index += 1,
            _ if argument.starts_with("--split-string=") => {
                let script = argument.trim_start_matches("--split-string=");
                words[index] = format!("--split-string={}", inject_marker(script)?);
                return shlex::try_join(words.iter().map(String::as_str))
                    .map_err(|_| anyhow::anyhow!("failed to quote provider-origin command"));
            }
            _ if argument.starts_with("--unset=") || argument.starts_with("--chdir=") => {
                index += 1;
            }
            _ if argument.starts_with("-u") && argument.len() > 2 => index += 1,
            _ if is_shell_assignment(argument) || argument.starts_with('-') => index += 1,
            _ => {
                words.insert(index, format!("{PROVIDER_ORIGIN_ENV}=1"));
                return shlex::try_join(words.iter().map(String::as_str))
                    .map_err(|_| anyhow::anyhow!("failed to quote provider-origin command"));
            }
        }
    }
    anyhow::bail!("env command has no command to wrap with a provider-origin marker")
}
