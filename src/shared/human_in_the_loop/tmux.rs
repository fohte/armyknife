use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// Get the tmux pane ID where this process is running.
///
/// Returns `Some("%pane_id")` if running inside tmux, `None` otherwise.
///
/// Uses `TMUX_PANE` environment variable which is set by tmux when the pane
/// is created. This identifies the actual pane where the command was executed,
/// not the currently focused pane.
pub fn get_tmux_target() -> Option<String> {
    crate::infra::tmux::current_pane_id_from_env()
}

/// Opens the review in a floating pane that attaches to the shpool session
/// `shpool_session`, so closing the pane detaches instead of ending the editor.
pub(super) fn open_review_pane(
    parent_pane_id: &str,
    title: &str,
    shpool_session: &str,
    executable: &OsStr,
    args: &[OsString],
    done_fifo_path: &Path,
) -> super::Result<String> {
    let environment = review_pane_environment();
    let script = review_pane_command(executable, args, done_fifo_path, &environment);
    let script_path = write_review_script(&std::env::temp_dir(), &script)?;
    let result = shpool_session_command(&script_path).and_then(|session_command| {
        crate::infra::tmux::open_editor_pane(crate::infra::tmux::EditorPaneSpec {
            parent_pane_id,
            title,
            shpool_session,
            command: Some(&session_command),
            force_shpool_attach: false,
        })
        .map_err(|error| super::HumanInTheLoopError::CommandFailed(format!("{error:#}")))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&script_path);
    }
    result
}

/// shpool parses its `-c` string as a template in which every `{` must open a
/// `{variable}` and cannot be escaped, so the script (with its `{ ...; }`
/// trap and arbitrary titles) is passed as a file instead. The script removes
/// itself on start.
fn write_review_script(dir: &Path, script: &OsStr) -> super::Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let path = dir.join(format!(
        "armyknife-review-{}.sh",
        uuid::Uuid::new_v4().simple()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)?;
    file.write_all(b"rm -f -- \"$0\"\n")?;
    file.write_all(script.as_bytes())?;
    file.write_all(b"\n")?;
    Ok(path)
}

/// shpool splits its command string with shell-words, which reads only single
/// quotes the way a POSIX shell does, so the path is quoted by hand.
fn shpool_session_command(script_path: &Path) -> super::Result<String> {
    let mut command = OsString::from("/bin/sh ");
    command.push(shell_quote(script_path.as_os_str()));
    let command = command.into_string().map_err(|_| {
        super::HumanInTheLoopError::CommandFailed(format!(
            "review script path is not valid UTF-8: {}",
            script_path.display()
        ))
    })?;
    if command.contains(['{', '}']) {
        return Err(super::HumanInTheLoopError::CommandFailed(format!(
            "review script path contains braces, which shpool treats as template variables: {}",
            script_path.display()
        )));
    }
    Ok(command)
}

fn review_pane_command(
    executable: &OsStr,
    args: &[OsString],
    done_fifo_path: &Path,
    environment: &[(OsString, OsString)],
) -> OsString {
    let mut command = OsString::new();
    for (name, value) in environment {
        command.push("export ");
        command.push(name);
        command.push("=");
        command.push(shell_quote(value));
        command.push("; ");
    }

    command.push("_ARMYKNIFE_DONE_FIFO=");
    command.push(shell_quote(done_fifo_path.as_os_str()));
    command.push(
        "; trap '_ARMYKNIFE_EXIT_STATUS=$?; [ -p \"$_ARMYKNIFE_DONE_FIFO\" ] && { exec 3<>\"$_ARMYKNIFE_DONE_FIFO\" && printf 0 >&3; } 2>/dev/null; exit \"$_ARMYKNIFE_EXIT_STATUS\"' EXIT; ",
    );
    command.push(shell_quote(executable));
    for arg in args {
        command.push(" ");
        command.push(shell_quote(arg));
    }
    command
}

fn review_pane_environment() -> Vec<(OsString, OsString)> {
    select_review_pane_environment(std::env::vars_os())
}

fn select_review_pane_environment(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    vars.into_iter()
        .filter(|(name, _)| {
            let name = name.as_bytes();
            valid_environment_name(name)
                // The shpool daemon's environment has no TMUX, which the editor
                // needs to reach the same tmux server as the floating pane.
                && (matches!(name, b"HOME" | b"PATH" | b"TMUX")
                    || name.starts_with(b"XDG_")
                    || (name.starts_with(b"ARMYKNIFE_")
                        && name.windows(2).any(|pair| pair == b"__")))
        })
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .collect()
}

fn valid_environment_name(name: &[u8]) -> bool {
    let Some((first, rest)) = name.split_first() else {
        return false;
    };
    (first.is_ascii_alphabetic() || *first == b'_')
        && rest
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

fn shell_quote(value: &OsStr) -> OsString {
    let mut quoted = Vec::with_capacity(value.as_bytes().len() + 2);
    quoted.push(b'\'');
    for byte in value.as_bytes() {
        if *byte == b'\'' {
            quoted.extend_from_slice(b"'\\''");
        } else {
            quoted.push(*byte);
        }
    }
    quoted.push(b'\'');
    OsString::from_vec(quoted)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[test]
    fn review_pane_command_quotes_fifo_and_preserves_non_utf8_arguments() {
        let document_arg = OsString::from_vec(b"doc-\xff.md".to_vec());
        let args = vec![OsString::from("--document"), document_arg];
        let environment = vec![
            (OsString::from("HOME"), OsString::from("/tmp/sample home")),
            (OsString::from("PATH"), OsString::from("/tmp/sample bin")),
        ];
        let command = review_pane_command(
            OsStr::new("/tmp/sample app"),
            &args,
            Path::new("/tmp/done 'fifo'"),
            &environment,
        );
        let expected = [
            b"export HOME='/tmp/sample home'; export PATH='/tmp/sample bin'; _ARMYKNIFE_DONE_FIFO='/tmp/done '\\''fifo'\\'''; trap '_ARMYKNIFE_EXIT_STATUS=$?; [ -p \"$_ARMYKNIFE_DONE_FIFO\" ] && { exec 3<>\"$_ARMYKNIFE_DONE_FIFO\" && printf 0 >&3; } 2>/dev/null; exit \"$_ARMYKNIFE_EXIT_STATUS\"' EXIT; '/tmp/sample app' '--document' 'doc-".as_slice(),
            &[0xff],
            b".md'".as_slice(),
        ]
        .concat();

        assert_eq!(command.as_os_str().as_bytes(), expected.as_slice());
    }

    #[test]
    fn review_pane_environment_keeps_editor_and_terminal_configuration() {
        let vars = vec![
            (
                OsString::from("XDG_CONFIG_HOME"),
                OsString::from("/tmp/config"),
            ),
            (
                OsString::from("ARMYKNIFE_EDITOR__EDITOR_COMMAND"),
                OsString::from("nvim --clean"),
            ),
            (OsString::from("HOME"), OsString::from("/tmp/home")),
            (OsString::from("PATH"), OsString::from("/tmp/bin")),
            (OsString::from("TMUX"), OsString::from("/tmp/tmux")),
            (OsString::from("ARMYKNIFE_UNRELATED"), OsString::from("x")),
        ];

        assert_eq!(
            select_review_pane_environment(vars),
            vec![
                (
                    OsString::from("ARMYKNIFE_EDITOR__EDITOR_COMMAND"),
                    OsString::from("nvim --clean"),
                ),
                (OsString::from("HOME"), OsString::from("/tmp/home")),
                (OsString::from("PATH"), OsString::from("/tmp/bin")),
                (OsString::from("TMUX"), OsString::from("/tmp/tmux")),
                (
                    OsString::from("XDG_CONFIG_HOME"),
                    OsString::from("/tmp/config")
                ),
            ],
        );
    }

    #[rstest]
    #[case::plain_path(
        PathBuf::from("/tmp/review script.sh"),
        Ok("/bin/sh '/tmp/review script.sh'".to_string())
    )]
    #[case::path_with_quote(
        PathBuf::from("/tmp/it's.sh"),
        Ok("/bin/sh '/tmp/it'\\''s.sh'".to_string())
    )]
    #[case::path_with_braces(
        PathBuf::from("/tmp/{dir}/review.sh"),
        Err("Command failed: review script path contains braces, which shpool treats as template variables: /tmp/{dir}/review.sh".to_string())
    )]
    #[case::non_utf8_path(
        PathBuf::from(OsString::from_vec(b"/tmp/\xff.sh".to_vec())),
        Err("Command failed: review script path is not valid UTF-8: /tmp/\u{fffd}.sh".to_string())
    )]
    fn shpool_session_command_quotes_the_script_path(
        #[case] script_path: PathBuf,
        #[case] expected: Result<String, String>,
    ) {
        assert_eq!(
            shpool_session_command(&script_path).map_err(|error| error.to_string()),
            expected,
        );
    }

    #[test]
    fn write_review_script_prepends_self_removal_and_keeps_script_bytes() {
        let dir = tempfile::TempDir::new().expect("temp dir creation should succeed");
        let script = OsString::from_vec(b"trap '{ :; }' EXIT; run \xff".to_vec());

        let path = write_review_script(dir.path(), &script).expect("script should be written");
        let metadata = std::fs::metadata(&path).expect("script should exist");
        let expected_script = [
            indoc::indoc! {r#"
                rm -f -- "$0"
                trap '{ :; }' EXIT; run "#}
            .as_bytes(),
            b"\xff\n",
        ]
        .concat();

        assert_eq!(
            (
                std::fs::read(&path).expect("script should be readable"),
                std::os::unix::fs::PermissionsExt::mode(&metadata.permissions()) & 0o777,
                path.parent().map(Path::to_path_buf),
            ),
            (expected_script, 0o600, Some(dir.path().to_path_buf()),),
        );
    }
}
