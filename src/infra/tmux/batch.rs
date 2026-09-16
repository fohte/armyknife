//! Batches multiple tmux commands into a single `tmux` invocation.
//!
//! Forking a `tmux` client costs ~50ms for the client/server handshake
//! alone, so issuing one command per pane doesn't scale to real
//! tmux-resurrect pane counts (dozens to ~100).

use std::io::Write;

use super::{Result, TmuxError, run_tmux, run_tmux_output};

/// Runs multiple tmux commands in a single `tmux source-file` invocation.
///
/// Each command line is evaluated independently, so a stale target in one
/// command (e.g. a pane closed since it was resolved) doesn't block the
/// rest of the batch, unlike joining commands with `;` on one command line.
///
/// Each element of `commands` is one already-tokenized tmux command, e.g.
/// `["set-option", "-p", "-t", "%1", "@opt", "value"]`. Tokens are quoted
/// per tmux's config-file syntax so arbitrary content passes through
/// literally.
pub fn run_batch(commands: &[Vec<String>]) -> Result<()> {
    if commands.is_empty() {
        return Ok(());
    }

    let script = commands
        .iter()
        .map(|tokens| {
            tokens
                .iter()
                .map(|t| quote_tmux_config_token(t))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut file = tempfile::NamedTempFile::new()
        .map_err(|e| TmuxError::Internal(format!("failed to create batch file: {e}")))?;
    file.write_all(script.as_bytes())
        .map_err(|e| TmuxError::Internal(format!("failed to write batch file: {e}")))?;

    run_tmux(&["source-file", &file.path().to_string_lossy()])
}

/// Quotes a single token for tmux config-file syntax so whitespace and
/// special characters are preserved as one literal argument.
///
/// `$` must be escaped too, not just `\` and `"`: inside a double-quoted
/// tmux config string, `$name` expands to tmux's `name` environment
/// variable (verified empirically), which plain `Command` argv never does.
fn quote_tmux_config_token(token: &str) -> String {
    let escaped = token
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$");
    format!("\"{escaped}\"")
}

/// Information about a tmux pane's position together with its pane_id and
/// the PID of the process running in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfoWithPid {
    pub session_name: String,
    pub window_index: u32,
    pub pane_index: u32,
    pub pane_id: String,
    pub pane_pid: u32,
}

/// Lists every currently existing pane's position, pane_id, and pane_pid in
/// one `list-panes -a` call, for resolving many panes by position (e.g.
/// restoring tmux-resurrect state) without forking a `tmux` client per pane.
pub fn list_all_panes() -> Result<Vec<PaneInfoWithPid>> {
    let output = run_tmux_output(&[
        "list-panes",
        "-a",
        "-F",
        "#{session_name}\t#{window_index}\t#{pane_index}\t#{pane_id}\t#{pane_pid}",
    ])?;

    Ok(output
        .lines()
        .filter_map(parse_pane_with_pid_line)
        .collect())
}

/// Parses a single line from tmux list-panes output with pane_pid.
/// Format: "#{session_name}\t#{window_index}\t#{pane_index}\t#{pane_id}\t#{pane_pid}"
fn parse_pane_with_pid_line(line: &str) -> Option<PaneInfoWithPid> {
    let mut parts = line.split('\t');
    Some(PaneInfoWithPid {
        session_name: parts.next()?.to_string(),
        window_index: parts.next()?.parse().ok()?,
        pane_index: parts.next()?.parse().ok()?,
        pane_id: parts.next()?.to_string(),
        pane_pid: parts.next()?.parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::standard_line(
        "main\t0\t1\t%5\t12345",
        Some(PaneInfoWithPid {
            session_name: "main".to_string(),
            window_index: 0,
            pane_index: 1,
            pane_id: "%5".to_string(),
            pane_pid: 12345,
        })
    )]
    #[case::session_with_slash(
        "fohte/repo\t1\t2\t%3\t99999",
        Some(PaneInfoWithPid {
            session_name: "fohte/repo".to_string(),
            window_index: 1,
            pane_index: 2,
            pane_id: "%3".to_string(),
            pane_pid: 99999,
        })
    )]
    #[case::missing_pid_field("main\t0\t1\t%5", None)]
    #[case::insufficient_parts("main\t0\t1", None)]
    #[case::empty_line("", None)]
    #[case::invalid_pid("main\t0\t1\t%5\tabc", None)]
    #[case::invalid_window_index("main\tabc\t1\t%5\t12345", None)]
    fn test_parse_pane_with_pid_line(
        #[case] line: &str,
        #[case] expected: Option<PaneInfoWithPid>,
    ) {
        assert_eq!(parse_pane_with_pid_line(line), expected);
    }

    #[rstest]
    #[case::plain("set-option", "\"set-option\"")]
    #[case::with_space("hello world", "\"hello world\"")]
    #[case::with_double_quote("say \"hi\"", "\"say \\\"hi\\\"\"")]
    #[case::with_backslash("a\\b", "\"a\\\\b\"")]
    #[case::with_dollar("$HOME", "\"\\$HOME\"")]
    #[case::with_semicolon("a; rm -rf /", "\"a; rm -rf /\"")]
    #[case::with_hash("value # not a comment", "\"value # not a comment\"")]
    #[case::empty("", "\"\"")]
    fn test_quote_tmux_config_token(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(quote_tmux_config_token(input), expected);
    }
}
