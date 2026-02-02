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

/// Saves all pane session IDs to the state file.
///
/// Format: session_name:window_index.pane_index<TAB>session_id
/// This format uses pane position (session:window.pane) rather than pane_id
/// because pane_id changes after tmux-resurrect restore.
fn run_save(_args: &SaveArgs) -> Result<()> {
    let panes = tmux::list_all_panes_with_option(TMUX_SESSION_OPTION);

    if panes.is_empty() {
        // No panes with session IDs to save, but this is not an error
        return Ok(());
    }

    let state_file = state_file_path()?;

    // Ensure parent directory exists
    if let Some(parent) = state_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    // Write state file
    let mut file = fs::File::create(&state_file)
        .with_context(|| format!("Failed to create state file: {}", state_file.display()))?;

    for pane in panes {
        if let Some(session_id) = pane.option_value {
            // Format: session_name:window_index.pane_index<TAB>session_id
            let pane_position = format!(
                "{}:{}.{}",
                pane.session_name, pane.window_index, pane.pane_index
            );
            writeln!(file, "{}\t{}", pane_position, session_id)?;
        }
    }

    Ok(())
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

    let file = fs::File::open(&state_file)
        .with_context(|| format!("Failed to open state file: {}", state_file.display()))?;
    let reader = BufReader::new(file);

    // Parse state file into a map of pane_position -> session_id
    let mut pane_sessions: HashMap<String, String> = HashMap::new();
    for line in reader.lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() != 2 {
            // Skip malformed lines
            continue;
        }

        let pane_position = parts[0].to_string();
        let session_id = parts[1].to_string();
        pane_sessions.insert(pane_position, session_id);
    }

    if pane_sessions.is_empty() {
        return Ok(());
    }

    // Restore session IDs to panes
    let mut restore_count = 0;
    for (pane_position, session_id) in &pane_sessions {
        // Parse pane_position: "session_name:window_index.pane_index"
        if let Some((session_name, rest)) = pane_position.split_once(':')
            && let Some((window_index_str, pane_index_str)) = rest.split_once('.')
            && let (Ok(window_index), Ok(pane_index)) = (
                window_index_str.parse::<u32>(),
                pane_index_str.parse::<u32>(),
            )
        {
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
    use rstest::rstest;

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
        let parts: Vec<&str> = line.split('\t').collect();
        let result = if parts.len() == 2 {
            Some((parts[0], parts[1]))
        } else {
            None
        };
        assert_eq!(result, expected);
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
        let result = if let Some((session_name, rest)) = position.split_once(':') {
            if let Some((window_index_str, pane_index_str)) = rest.split_once('.') {
                if let (Ok(window_index), Ok(pane_index)) = (
                    window_index_str.parse::<u32>(),
                    pane_index_str.parse::<u32>(),
                ) {
                    Some((session_name, window_index, pane_index))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        assert_eq!(result, expected);
    }
}
