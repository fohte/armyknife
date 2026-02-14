//! Tmux-resurrect integration for Claude Code session restoration.
//!
//! This module provides commands to save and restore Claude Code session IDs
//! when tmux-resurrect saves/restores tmux sessions. Since tmux user options
//! are not automatically preserved by tmux-resurrect, we need to explicitly
//! save them to a state file and restore them after tmux-resurrect completes.

use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Subcommand};

use super::types::TMUX_SESSION_OPTION;
use crate::infra::tmux;
use crate::shared::cache;

/// Directory name for storing resurrect state files.
const RESURRECT_STATE_DIR: &str = "resurrect";

/// File name for the resurrect state file.
const RESURRECT_STATE_FILE: &str = "pane_sessions.txt";

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum ResurrectCommands {
    /// Save all pane session IDs (called from tmux-resurrect post-save hook)
    Save(SaveArgs),

    /// Restore pane session IDs (called from tmux-resurrect post-restore hook)
    Restore(RestoreArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SaveArgs {}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct RestoreArgs {}

/// Runs the resurrect subcommand.
pub fn run(cmd: &ResurrectCommands) -> Result<()> {
    match cmd {
        ResurrectCommands::Save(args) => run_save(args),
        ResurrectCommands::Restore(args) => run_restore(args),
    }
}

/// Returns the path to the resurrect state file.
fn state_file_path() -> Result<PathBuf> {
    let base_dir =
        cache::base_dir().ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
    Ok(base_dir
        .join("cc")
        .join(RESURRECT_STATE_DIR)
        .join(RESURRECT_STATE_FILE))
}

/// Parses a state file line into (pane_position, session_id).
/// Format: "session_name:window_index.pane_index<TAB>session_id"
fn parse_state_line(line: &str) -> Option<(&str, &str)> {
    if line.is_empty() {
        return None;
    }
    let parts: Vec<&str> = line.split('\t').collect();
    if parts.len() == 2 {
        Some((parts[0], parts[1]))
    } else {
        None
    }
}

/// Parses a pane position string into (session_name, window_index, pane_index).
/// Format: "session_name:window_index.pane_index"
fn parse_pane_position(position: &str) -> Option<(&str, u32, u32)> {
    let (session_name, rest) = position.split_once(':')?;
    let (window_index_str, pane_index_str) = rest.split_once('.')?;
    let window_index = window_index_str.parse::<u32>().ok()?;
    let pane_index = pane_index_str.parse::<u32>().ok()?;
    Some((session_name, window_index, pane_index))
}

/// Formats a pane position for state file.
/// Format: "session_name:window_index.pane_index"
fn format_pane_position(session_name: &str, window_index: u32, pane_index: u32) -> String {
    format!("{}:{}.{}", session_name, window_index, pane_index)
}

/// Writes pane sessions to a state file.
fn write_state_file(
    state_file: &PathBuf,
    pane_sessions: &[(String, u32, u32, String)], // (session_name, window_index, pane_index, session_id)
) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = state_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    let mut file = fs::File::create(state_file)
        .with_context(|| format!("Failed to create state file: {}", state_file.display()))?;

    for (session_name, window_index, pane_index, session_id) in pane_sessions {
        let pane_position = format_pane_position(session_name, *window_index, *pane_index);
        writeln!(file, "{}\t{}", pane_position, session_id)?;
    }

    Ok(())
}

/// Reads pane sessions from a state file.
/// Returns a map of pane_position -> session_id.
fn read_state_file(state_file: &PathBuf) -> Result<HashMap<String, String>> {
    let file = fs::File::open(state_file)
        .with_context(|| format!("Failed to open state file: {}", state_file.display()))?;
    let reader = BufReader::new(file);

    let mut pane_sessions = HashMap::new();
    for line in reader.lines() {
        let line = line?;
        if let Some((pane_position, session_id)) = parse_state_line(&line) {
            pane_sessions.insert(pane_position.to_string(), session_id.to_string());
        }
    }

    Ok(pane_sessions)
}

/// Saves all pane session IDs to the state file.
///
/// Format: session_name:window_index.pane_index<TAB>session_id
/// This format uses pane position (session:window.pane) rather than pane_id
/// because pane_id changes after tmux-resurrect restore.
fn run_save(_args: &SaveArgs) -> Result<()> {
    let panes = tmux::list_all_panes_with_option(TMUX_SESSION_OPTION);
    let state_file = state_file_path()?;

    if panes.is_empty() {
        // No panes with session IDs to save; clean up any stale state file
        let _ = fs::remove_file(&state_file);
        return Ok(());
    }

    // Convert panes to the format expected by write_state_file
    let pane_sessions: Vec<_> = panes
        .into_iter()
        .filter_map(|pane| {
            pane.option_value.map(|session_id| {
                (
                    pane.session_name,
                    pane.window_index,
                    pane.pane_index,
                    session_id,
                )
            })
        })
        .collect();

    write_state_file(&state_file, &pane_sessions)
}

/// Restores pane session IDs from the state file.
///
/// Reads the state file and sets the user option on each pane that still exists.
fn run_restore(_args: &RestoreArgs) -> Result<()> {
    let state_file = state_file_path()?;

    if !state_file.exists() {
        // No state file means nothing to restore
        return Ok(());
    }

    let pane_sessions = read_state_file(&state_file)?;

    if pane_sessions.is_empty() {
        return Ok(());
    }

    // Restore session IDs to panes
    let mut restore_count = 0;
    for (pane_position, session_id) in &pane_sessions {
        if let Some((session_name, window_index, pane_index)) = parse_pane_position(pane_position) {
            // Find the pane_id for this position
            if let Some(pane_id) =
                tmux::find_pane_id_by_position(session_name, window_index, pane_index)
            {
                // Set the user option on this pane
                if tmux::set_pane_option(&pane_id, TMUX_SESSION_OPTION, session_id).is_ok() {
                    restore_count += 1;
                }
            }
        }
    }

    // Clean up state file after successful restore
    if restore_count > 0 {
        let _ = fs::remove_file(&state_file);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;
    use tempfile::TempDir;

    #[test]
    fn state_file_path_is_under_cache_dir() {
        let path = state_file_path().expect("should return path");
        assert!(path.ends_with("cc/resurrect/pane_sessions.txt"));
    }

    #[rstest]
    #[case::valid_line("main:0.1\tabc-123", Some(("main:0.1", "abc-123")))]
    #[case::valid_uuid("work:2.0\t550e8400-e29b-41d4-a716-446655440000", Some(("work:2.0", "550e8400-e29b-41d4-a716-446655440000")))]
    #[case::session_with_slash("fohte/repo:1.2\txyz-456", Some(("fohte/repo:1.2", "xyz-456")))]
    #[case::missing_tab("main:0.1abc-123", None)]
    #[case::extra_tabs("main:0.1\tabc\t123", None)]
    #[case::empty_line("", None)]
    fn test_parse_state_line(#[case] line: &str, #[case] expected: Option<(&str, &str)>) {
        assert_eq!(parse_state_line(line), expected);
    }

    #[rstest]
    #[case::simple("main:0.1", Some(("main", 0, 1)))]
    #[case::high_indices("work:10.5", Some(("work", 10, 5)))]
    #[case::session_with_slash("fohte/repo:1.2", Some(("fohte/repo", 1, 2)))]
    #[case::missing_colon("main0.1", None)]
    #[case::missing_dot("main:01", None)]
    #[case::non_numeric_window("main:a.1", None)]
    #[case::non_numeric_pane("main:0.b", None)]
    fn test_parse_pane_position(
        #[case] position: &str,
        #[case] expected: Option<(&str, u32, u32)>,
    ) {
        assert_eq!(parse_pane_position(position), expected);
    }

    #[rstest]
    #[case::simple("main", 0, 1, "main:0.1")]
    #[case::high_indices("work", 10, 5, "work:10.5")]
    #[case::session_with_slash("fohte/repo", 1, 2, "fohte/repo:1.2")]
    fn test_format_pane_position(
        #[case] session_name: &str,
        #[case] window_index: u32,
        #[case] pane_index: u32,
        #[case] expected: &str,
    ) {
        assert_eq!(
            format_pane_position(session_name, window_index, pane_index),
            expected
        );
    }

    #[test]
    fn write_and_read_state_file_roundtrip() {
        let temp_dir = TempDir::new().expect("should create temp dir");
        let state_file = temp_dir.path().join("pane_sessions.txt");

        let pane_sessions = vec![
            ("main".to_string(), 0, 1, "abc-123".to_string()),
            ("work".to_string(), 2, 0, "def-456".to_string()),
            (
                "fohte/repo".to_string(),
                1,
                2,
                "550e8400-e29b-41d4-a716-446655440000".to_string(),
            ),
        ];

        write_state_file(&state_file, &pane_sessions).expect("should write state file");

        assert!(state_file.exists());

        let read_sessions = read_state_file(&state_file).expect("should read state file");

        assert_eq!(read_sessions.len(), 3);
        assert_eq!(read_sessions.get("main:0.1"), Some(&"abc-123".to_string()));
        assert_eq!(read_sessions.get("work:2.0"), Some(&"def-456".to_string()));
        assert_eq!(
            read_sessions.get("fohte/repo:1.2"),
            Some(&"550e8400-e29b-41d4-a716-446655440000".to_string())
        );
    }

    #[test]
    fn write_state_file_creates_parent_directories() {
        let temp_dir = TempDir::new().expect("should create temp dir");
        let state_file = temp_dir.path().join("nested").join("dir").join("state.txt");

        let pane_sessions = vec![("main".to_string(), 0, 0, "session-id".to_string())];

        write_state_file(&state_file, &pane_sessions).expect("should write state file");

        assert!(state_file.exists());
    }

    #[test]
    fn read_state_file_skips_malformed_lines() {
        let temp_dir = TempDir::new().expect("should create temp dir");
        let state_file = temp_dir.path().join("state.txt");

        // Write a file with some malformed lines
        fs::write(
            &state_file,
            indoc! {"
                main:0.1\tabc-123
                malformed line

                work:2.0\tdef-456
                extra\ttabs\there
            "},
        )
        .expect("should write file");

        let sessions = read_state_file(&state_file).expect("should read state file");

        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions.get("main:0.1"), Some(&"abc-123".to_string()));
        assert_eq!(sessions.get("work:2.0"), Some(&"def-456".to_string()));
    }

    #[test]
    fn read_state_file_returns_empty_map_for_empty_file() {
        let temp_dir = TempDir::new().expect("should create temp dir");
        let state_file = temp_dir.path().join("state.txt");

        fs::write(&state_file, "").expect("should write file");

        let sessions = read_state_file(&state_file).expect("should read state file");

        assert!(sessions.is_empty());
    }
}
