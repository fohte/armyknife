//! Claude Code session information reader.
//!
//! This module reads session metadata from Claude Code's sessions-index.json files
//! located at ~/.claude/projects/{encoded-path}/sessions-index.json

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

/// Claude Code sessions-index.json structure
#[derive(Debug, Deserialize)]
pub struct SessionsIndex {
    #[expect(dead_code, reason = "field exists in JSON but not used")]
    pub version: u32,
    pub entries: Vec<SessionEntry>,
}

/// Individual session entry in sessions-index.json
#[derive(Debug, Deserialize)]
pub struct SessionEntry {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "firstPrompt")]
    pub first_prompt: Option<String>,
    pub summary: Option<String>,
}

/// Encodes a project path to Claude Code's directory format.
///
/// Claude Code encodes paths by replacing '/' and '.' with '-'.
/// Example: /Users/fohte/project -> -Users-fohte-project
pub fn encode_project_path(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| if c == '/' || c == '.' { '-' } else { c })
        .collect()
}

/// Returns the path to Claude Code's sessions-index.json for a project.
///
/// Path format: ~/.claude/projects/{encoded-path}/sessions-index.json
pub fn sessions_index_path(project_path: &Path) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let encoded = encode_project_path(project_path);
    Some(
        home.join(".claude")
            .join("projects")
            .join(encoded)
            .join("sessions-index.json"),
    )
}

/// Retrieves the session title from Claude Code's sessions-index.json.
///
/// Returns the summary if available, otherwise the first ~50 characters of firstPrompt.
/// Returns None if the session cannot be found or the file doesn't exist.
pub fn get_session_title(project_path: &Path, session_id: &str) -> Option<String> {
    let index_path = sessions_index_path(project_path)?;

    if !index_path.exists() {
        return None;
    }

    let content = fs::read_to_string(&index_path).ok()?;
    let index: SessionsIndex = serde_json::from_str(&content).ok()?;

    for entry in index.entries {
        if entry.session_id == session_id {
            // Prefer summary over firstPrompt
            if let Some(summary) = entry.summary
                && !summary.is_empty()
            {
                return Some(summary);
            }
            if let Some(first_prompt) = entry.first_prompt {
                return Some(truncate_first_prompt(&first_prompt, 50));
            }
            return None;
        }
    }

    None
}

/// Truncates the first prompt to a reasonable display length.
fn truncate_first_prompt(prompt: &str, max_chars: usize) -> String {
    let trimmed = prompt.trim();
    let char_count = trimmed.chars().count();

    if char_count <= max_chars {
        trimmed.to_string()
    } else {
        let truncated: String = trimmed.chars().take(max_chars - 3).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::simple_path("/Users/fohte/project", "-Users-fohte-project")]
    #[case::with_github(
        "/Users/fohte/ghq/github.com/fohte/armyknife",
        "-Users-fohte-ghq-github-com-fohte-armyknife"
    )]
    #[case::with_dots("/Users/fohte/project.name/src", "-Users-fohte-project-name-src")]
    #[case::worktree(
        "/Users/fohte/armyknife/.worktrees/claude-title",
        "-Users-fohte-armyknife--worktrees-claude-title"
    )]
    fn test_encode_project_path(#[case] input: &str, #[case] expected: &str) {
        let path = Path::new(input);
        assert_eq!(encode_project_path(path), expected);
    }

    #[rstest]
    #[case::short("hello", 10, "hello")]
    #[case::exact("hello", 5, "hello")]
    #[case::with_whitespace("  hello  ", 10, "hello")]
    #[case::truncate_long("this is a very long prompt", 15, "this is a ve...")]
    #[case::truncate_exact_boundary("hello world", 11, "hello world")]
    #[case::truncate_one_over("hello world!", 11, "hello wo...")]
    fn test_truncate_first_prompt(#[case] input: &str, #[case] max: usize, #[case] expected: &str) {
        assert_eq!(truncate_first_prompt(input, max), expected);
    }

    #[test]
    fn test_sessions_index_path() {
        let path = Path::new("/Users/test/project");
        let result = sessions_index_path(path);

        assert!(result.is_some());
        let result_path = result.unwrap();
        assert!(result_path.to_string_lossy().contains(".claude/projects"));
        assert!(
            result_path
                .to_string_lossy()
                .ends_with("sessions-index.json")
        );
    }

    #[test]
    fn test_get_session_title_nonexistent_file() {
        let path = Path::new("/nonexistent/path/that/does/not/exist");
        let result = get_session_title(path, "test-session-id");
        assert!(result.is_none());
    }
}
