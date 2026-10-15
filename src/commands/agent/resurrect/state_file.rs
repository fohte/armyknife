use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

use super::super::types::Engine;
use crate::shared::cache;
use anyhow::{Context, Result};
use clap::ValueEnum;

/// Directory name for storing resurrect state files.
const RESURRECT_STATE_DIR: &str = "resurrect";

/// File name for the resurrect state file.
const RESURRECT_STATE_FILE: &str = "pane_sessions.txt";

/// Session metadata captured for one pane during the pre-save hook.
type SavedPaneSession = (String, u32, u32, String, Vec<String>, Engine);

/// Session metadata restored by pane position from the state file.
type RestoredPaneSession = (String, Vec<String>, Option<Engine>);
/// Returns the path to the resurrect state file.
pub(super) fn state_file_path() -> Result<PathBuf> {
    let base_dir =
        cache::base_dir().ok_or_else(|| anyhow::anyhow!("Could not determine cache directory"))?;
    Ok(base_dir
        .join("cc")
        .join(RESURRECT_STATE_DIR)
        .join(RESURRECT_STATE_FILE))
}

/// Parses a state file line into (pane_position, session_id, ancestor_session_ids, engine).
/// Format: "session_name:window_index.pane_index<TAB>session_id<TAB>ancestor_session_ids<TAB>engine".
/// The ancestor and engine columns are optional for backwards compatibility;
/// state files without an engine column resolve it from the current store.
fn parse_state_line(line: &str) -> Option<(&str, &str, Vec<String>, Option<Engine>)> {
    if line.is_empty() {
        return None;
    }
    match line.split('\t').collect::<Vec<_>>().as_slice() {
        [pane_position, session_id] => Some((pane_position, session_id, Vec::new(), None)),
        [pane_position, session_id, ancestor_ids] => Some((
            pane_position,
            session_id,
            parse_ancestor_ids(ancestor_ids),
            None,
        )),
        [pane_position, session_id, ancestor_ids, engine] => Some((
            pane_position,
            session_id,
            parse_ancestor_ids(ancestor_ids),
            Some(Engine::from_str(engine, false).ok()?),
        )),
        _ => None,
    }
}

fn parse_ancestor_ids(s: &str) -> Vec<String> {
    if s.is_empty() {
        Vec::new()
    } else {
        s.split(',').map(String::from).collect()
    }
}

/// Formats a pane position for state file.
/// Format: "session_name:window_index.pane_index"
pub(super) fn format_pane_position(
    session_name: &str,
    window_index: u32,
    pane_index: u32,
) -> String {
    format!("{}:{}.{}", session_name, window_index, pane_index)
}

pub(super) fn format_engine(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "claude",
        Engine::Codex => "codex",
    }
}

/// Writes pane sessions to a state file.
pub(super) fn write_state_file(
    state_file: &PathBuf,
    pane_sessions: &[SavedPaneSession],
) -> Result<()> {
    // Ensure parent directory exists
    if let Some(parent) = state_file.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory: {}", parent.display()))?;
    }

    let mut file = fs::File::create(state_file)
        .with_context(|| format!("Failed to create state file: {}", state_file.display()))?;

    for (session_name, window_index, pane_index, session_id, ancestor_session_ids, engine) in
        pane_sessions
    {
        let pane_position = format_pane_position(session_name, *window_index, *pane_index);
        writeln!(
            file,
            "{}\t{}\t{}\t{}",
            pane_position,
            session_id,
            ancestor_session_ids.join(","),
            format_engine(*engine),
        )?;
    }

    Ok(())
}

/// Reads pane sessions from a state file.
/// Returns a map of pane_position -> (session_id, ancestor_session_ids, engine).
pub(super) fn read_state_file(
    state_file: &PathBuf,
) -> Result<HashMap<String, RestoredPaneSession>> {
    let file = fs::File::open(state_file)
        .with_context(|| format!("Failed to open state file: {}", state_file.display()))?;
    let reader = BufReader::new(file);

    let mut pane_sessions = HashMap::new();
    for line in reader.lines() {
        let line = line?;
        if let Some((pane_position, session_id, ancestor_session_ids, engine)) =
            parse_state_line(&line)
        {
            pane_sessions.insert(
                pane_position.to_string(),
                (session_id.to_string(), ancestor_session_ids, engine),
            );
        }
    }

    Ok(pane_sessions)
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
    #[case::valid_line_no_ancestors(
        "main:0.1\tabc-123",
        Some(("main:0.1", "abc-123", Vec::new(), None))
    )]
    #[case::valid_uuid(
        "work:2.0\t550e8400-e29b-41d4-a716-446655440000",
        Some((
            "work:2.0",
            "550e8400-e29b-41d4-a716-446655440000",
            Vec::new(),
            None
        ))
    )]
    #[case::session_with_slash(
        "fohte/repo:1.2\txyz-456",
        Some(("fohte/repo:1.2", "xyz-456", Vec::new(), None))
    )]
    #[case::with_ancestors(
        "main:0.1\tabc-123\troot-1,parent-1",
        Some((
            "main:0.1",
            "abc-123",
            vec!["root-1".to_string(), "parent-1".to_string()],
            None
        ))
    )]
    #[case::with_empty_ancestors_field(
        "main:0.1\tabc-123\t",
        Some(("main:0.1", "abc-123", Vec::new(), None))
    )]
    #[case::with_codex_engine(
        "main:0.1\tabc-123\troot-1\tcodex",
        Some((
            "main:0.1",
            "abc-123",
            vec!["root-1".to_string()],
            Some(Engine::Codex)
        ))
    )]
    #[case::missing_tab("main:0.1abc-123", None)]
    #[case::unknown_engine("main:0.1\tabc-123\t\tunknown", None)]
    #[case::too_many_tabs("main:0.1\tabc\t123\textra", None)]
    #[case::empty_line("", None)]
    fn test_parse_state_line(
        #[case] line: &str,
        #[case] expected: Option<(&str, &str, Vec<String>, Option<Engine>)>,
    ) {
        assert_eq!(parse_state_line(line), expected);
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
            (
                "main".to_string(),
                0,
                1,
                "abc-123".to_string(),
                Vec::new(),
                Engine::Claude,
            ),
            (
                "work".to_string(),
                2,
                0,
                "def-456".to_string(),
                vec!["root-1".to_string(), "parent-1".to_string()],
                Engine::Codex,
            ),
            (
                "fohte/repo".to_string(),
                1,
                2,
                "550e8400-e29b-41d4-a716-446655440000".to_string(),
                Vec::new(),
                Engine::Claude,
            ),
        ];

        write_state_file(&state_file, &pane_sessions).expect("should write state file");

        let read_sessions = read_state_file(&state_file).expect("should read state file");

        assert_eq!(
            read_sessions,
            HashMap::from([
                (
                    "main:0.1".to_string(),
                    ("abc-123".to_string(), Vec::new(), Some(Engine::Claude))
                ),
                (
                    "work:2.0".to_string(),
                    (
                        "def-456".to_string(),
                        vec!["root-1".to_string(), "parent-1".to_string()],
                        Some(Engine::Codex)
                    )
                ),
                (
                    "fohte/repo:1.2".to_string(),
                    (
                        "550e8400-e29b-41d4-a716-446655440000".to_string(),
                        Vec::new(),
                        Some(Engine::Claude)
                    )
                ),
            ])
        );
    }

    #[test]
    fn write_state_file_creates_parent_directories() {
        let temp_dir = TempDir::new().expect("should create temp dir");
        let state_file = temp_dir.path().join("nested").join("dir").join("state.txt");

        let pane_sessions = vec![(
            "main".to_string(),
            0,
            0,
            "session-id".to_string(),
            Vec::new(),
            Engine::Claude,
        )];

        write_state_file(&state_file, &pane_sessions).expect("should write state file");
        assert_eq!(
            read_state_file(&state_file).expect("should read state file"),
            HashMap::from([(
                "main:0.0".to_string(),
                ("session-id".to_string(), Vec::new(), Some(Engine::Claude),),
            )])
        );
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

                work:2.0\tdef-456\troot-1
                work-codex:3.0\tghi-789\troot-2\tcodex
                extra\ttabs\there\ttoo\tmany
            "},
        )
        .expect("should write file");

        let sessions = read_state_file(&state_file).expect("should read state file");

        assert_eq!(
            sessions,
            HashMap::from([
                (
                    "main:0.1".to_string(),
                    ("abc-123".to_string(), Vec::new(), None)
                ),
                (
                    "work:2.0".to_string(),
                    ("def-456".to_string(), vec!["root-1".to_string()], None)
                ),
                (
                    "work-codex:3.0".to_string(),
                    (
                        "ghi-789".to_string(),
                        vec!["root-2".to_string()],
                        Some(Engine::Codex)
                    )
                ),
            ])
        );
    }

    #[test]
    fn read_state_file_returns_empty_map_for_empty_file() {
        let temp_dir = TempDir::new().expect("should create temp dir");
        let state_file = temp_dir.path().join("state.txt");

        fs::write(&state_file, "").expect("should write file");

        let sessions = read_state_file(&state_file).expect("should read state file");

        assert_eq!(sessions, HashMap::new());
    }
}
