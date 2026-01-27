//! Claude Code session information reader.
//!
//! This module reads session metadata from Claude Code's sessions-index.json files
//! located at ~/.claude/projects/{encoded-path}/sessions-index.json
//!
//! Falls back to reading the first user prompt from .jsonl files when
//! sessions-index.json is not available.

use serde::Deserialize;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
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

/// User message entry in .jsonl files
#[derive(Debug, Deserialize)]
struct JsonlEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    message: Option<JsonlMessage>,
}

/// Message content in .jsonl user entries
#[derive(Debug, Deserialize)]
struct JsonlMessage {
    content: Option<String>,
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

/// Returns the path to Claude Code's project directory.
///
/// Path format: ~/.claude/projects/{encoded-path}/
fn project_dir(project_path: &Path) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let encoded = encode_project_path(project_path);
    Some(home.join(".claude").join("projects").join(encoded))
}

/// Returns the path to Claude Code's sessions-index.json for a project.
///
/// Path format: ~/.claude/projects/{encoded-path}/sessions-index.json
pub fn sessions_index_path(project_path: &Path) -> Option<PathBuf> {
    Some(project_dir(project_path)?.join("sessions-index.json"))
}

/// Returns the path to a session's .jsonl file.
///
/// Path format: ~/.claude/projects/{encoded-path}/{session_id}.jsonl
fn session_jsonl_path(project_path: &Path, session_id: &str) -> Option<PathBuf> {
    Some(project_dir(project_path)?.join(format!("{session_id}.jsonl")))
}

/// Retrieves the session title from Claude Code's sessions-index.json.
///
/// Returns the summary if available, otherwise the first ~50 characters of firstPrompt.
/// Falls back to reading the first user prompt from the .jsonl file if sessions-index.json
/// doesn't exist or doesn't contain the session.
pub fn get_session_title(project_path: &Path, session_id: &str) -> Option<String> {
    // First, try to get from sessions-index.json
    if let Some(title) = get_title_from_index(project_path, session_id) {
        return Some(title);
    }

    // Fall back to reading from .jsonl file directly
    get_title_from_jsonl(project_path, session_id)
}

/// Tries to get session title from sessions-index.json.
fn get_title_from_index(project_path: &Path, session_id: &str) -> Option<String> {
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
                // Return full prompt; truncation is handled by the display layer
                return Some(first_prompt.trim().to_string());
            }
            return None;
        }
    }

    None
}

/// Reads the first user prompt from a session's .jsonl file.
fn get_title_from_jsonl(project_path: &Path, session_id: &str) -> Option<String> {
    let jsonl_path = session_jsonl_path(project_path, session_id)?;

    if !jsonl_path.exists() {
        return None;
    }

    let file = File::open(&jsonl_path).ok()?;
    let reader = BufReader::new(file);

    for line in reader.lines() {
        let line = line.ok()?;
        if line.is_empty() {
            continue;
        }

        let entry: JsonlEntry = serde_json::from_str(&line).ok()?;

        // Look for the first "user" type entry with message content
        if entry.entry_type.as_deref() == Some("user")
            && let Some(message) = entry.message
            && let Some(content) = message.content
        {
            // Return full content; truncation is handled by the display layer
            return Some(content.trim().to_string());
        }
    }

    None
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
