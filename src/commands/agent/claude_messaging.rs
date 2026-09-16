//! Sends a message directly to a Claude Code session's `SendMessage` Unix
//! domain socket, bypassing the `SendMessage` tool entirely.
//!
//! Claude Code's `SendMessage` tool works by writing one NDJSON line to a
//! per-session socket (`messagingSocketPath` in the session registry, see
//! `claude_registry`); nothing about the tool call itself is special. This
//! lets armyknife notify a session from contexts where no Claude Code
//! session is in the loop to call the tool -- e.g. a background process
//! reporting that a delegated PR was merged.
//!
//! This is an undocumented, internal protocol with a `peerProtocol` version
//! field in the registry, meaning it is expected to change without notice.
//! Every failure here is surfaced as an error rather than swallowed, so a
//! protocol break is visible instead of silently dropping messages.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Sends `content` as a user message to the session listening on
/// `socket_path`. `pid` is used to locate that session's peer-token key
/// file (`~/.claude/sessions/<pid>.*.key`) for authentication; if the key
/// file can't be found or read, the message is still sent without an auth
/// frame -- Claude Code only requires auth on Windows.
///
/// Sets neither `priority` nor `from`: this is a fire-and-forget
/// notification queued behind the receiving session's current turn, and
/// armyknife has no listening socket of its own to receive a delivery
/// receipt on.
pub fn send_message(socket_path: &str, pid: u32, content: &str) -> Result<()> {
    let mut stream = UnixStream::connect(socket_path)
        .with_context(|| format!("failed to connect to messaging socket {socket_path}"))?;

    if let Some(token) = read_peer_token(pid) {
        let auth_line = build_auth_line(&token)?;
        writeln!(stream, "{auth_line}").context("failed to write auth frame")?;
    }

    let user_line = build_user_line(content)?;
    writeln!(stream, "{user_line}").context("failed to write user frame")?;

    stream.flush().context("failed to flush messaging socket")?;

    Ok(())
}

#[derive(Serialize)]
struct AuthFrame<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    token: &'a str,
}

#[derive(Serialize)]
struct UserFrame<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    message: UserMessage<'a>,
}

#[derive(Serialize)]
struct UserMessage<'a> {
    role: &'static str,
    content: &'a str,
}

fn build_auth_line(token: &str) -> Result<String> {
    Ok(serde_json::to_string(&AuthFrame {
        kind: "auth",
        token,
    })?)
}

fn build_user_line(content: &str) -> Result<String> {
    Ok(serde_json::to_string(&UserFrame {
        kind: "user",
        message: UserMessage {
            role: "user",
            content,
        },
    })?)
}

#[derive(Deserialize)]
struct PeerKeyFile {
    #[serde(rename = "peerToken")]
    peer_token: String,
}

/// Reads the `peerToken` from `~/.claude/sessions/<pid>.*.key`. Returns
/// `None` on any failure (missing home dir, missing/unreadable/corrupt key
/// file) -- the caller falls back to sending without an auth frame rather
/// than treating a missing key as fatal (see `send_message`'s doc comment).
fn read_peer_token(pid: u32) -> Option<String> {
    let home = crate::shared::dirs::home_dir()?;
    let dir = home.join(".claude").join("sessions");
    let key_path = find_key_file(&dir, pid)?;

    let content = std::fs::read_to_string(key_path).ok()?;
    let key: PeerKeyFile = serde_json::from_str(&content).ok()?;
    Some(key.peer_token)
}

fn find_key_file(dir: &Path, pid: u32) -> Option<std::path::PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let pid_str = pid.to_string();

    entries.flatten().map(|e| e.path()).find(|path| {
        path.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| is_key_file_for_pid(name, &pid_str))
    })
}

/// Whether `file_name` is a peer-token key file (`<pid>.<sha256-hex>.key`)
/// belonging to `pid_str`. armyknife doesn't need to compute the sha256 of
/// the socket path itself -- the pid segment alone is enough to identify
/// the right file, since `pid` is unique among the entries that matter
/// (concurrently running processes).
fn is_key_file_for_pid(file_name: &str, pid_str: &str) -> bool {
    file_name
        .strip_suffix(".key")
        .and_then(|stem| stem.split_once('.'))
        .is_some_and(|(pid_part, _hash)| pid_part == pid_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn find_key_file_picks_the_entry_matching_pid_among_other_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("111.deadbeef0123456789.key"), "").unwrap();
        fs::write(dir.path().join("222.deadbeef0123456789.key"), "").unwrap();

        assert_eq!(
            find_key_file(dir.path(), 111),
            Some(dir.path().join("111.deadbeef0123456789.key"))
        );
    }

    #[test]
    fn find_key_file_returns_none_when_no_file_matches_pid() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("222.deadbeef0123456789.key"), "").unwrap();

        assert_eq!(find_key_file(dir.path(), 111), None);
    }

    #[test]
    fn find_key_file_returns_none_when_sessions_dir_is_missing() {
        let dir = TempDir::new().unwrap().path().join("missing");

        assert_eq!(find_key_file(&dir, 111), None);
    }

    #[test]
    fn build_auth_line_produces_the_documented_frame_shape() {
        assert_eq!(
            build_auth_line("token-123").unwrap(),
            r#"{"type":"auth","token":"token-123"}"#
        );
    }

    #[test]
    fn build_user_line_produces_the_documented_frame_shape_with_no_priority_or_from() {
        assert_eq!(
            build_user_line("hello there").unwrap(),
            r#"{"type":"user","message":{"role":"user","content":"hello there"}}"#
        );
    }

    #[rstest]
    #[case::exact_match("12345.abcdef0123456789.key", "12345", true)]
    #[case::pid_prefix_of_another_pid("123456.abcdef0123456789.key", "12345", false)]
    #[case::pid_suffix_of_another_pid("1.abcdef0123456789.key", "12345", false)]
    #[case::wrong_extension("12345.abcdef0123456789.json", "12345", false)]
    #[case::no_hash_segment("12345.key", "12345", false)]
    fn is_key_file_for_pid_cases(
        #[case] file_name: &str,
        #[case] pid_str: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(is_key_file_for_pid(file_name, pid_str), expected);
    }
}
