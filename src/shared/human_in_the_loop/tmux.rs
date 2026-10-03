use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;

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

pub(super) fn open_review_pane(
    parent_pane_id: &str,
    title: &str,
    executable: &OsStr,
    args: &[OsString],
    done_fifo_path: &Path,
) -> super::Result<()> {
    let environment = review_pane_environment();
    let command = review_pane_command(executable, args, done_fifo_path, &environment);
    crate::infra::tmux::open_floating_pane(crate::infra::tmux::FloatingPaneSpec {
        parent_pane_id,
        title,
        command: &command,
    })
    .map(|_| ())
    .map_err(|error| super::HumanInTheLoopError::CommandFailed(error.to_string()))
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
    command.push("; trap 'printf 0 2>/dev/null > \"$_ARMYKNIFE_DONE_FIFO\"' EXIT; ");
    command.push(shell_quote(executable));
    for arg in args {
        command.push(" ");
        command.push(shell_quote(arg));
    }
    command
}

fn review_pane_environment() -> Vec<(OsString, OsString)> {
    std::env::vars_os()
        .filter(|(name, _)| {
            let name = name.as_bytes();
            valid_environment_name(name)
                && (matches!(name, b"HOME" | b"PATH")
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
            b"export HOME='/tmp/sample home'; export PATH='/tmp/sample bin'; _ARMYKNIFE_DONE_FIFO='/tmp/done '\\''fifo'\\'''; trap 'printf 0 2>/dev/null > \"$_ARMYKNIFE_DONE_FIFO\"' EXIT; '/tmp/sample app' '--document' 'doc-".as_slice(),
            &[0xff],
            b".md'".as_slice(),
        ]
        .concat();

        assert_eq!(command.as_os_str().as_bytes(), expected.as_slice());
    }
}
