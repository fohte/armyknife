//! Claude Code session information reader.
//!
//! This module reads session metadata from Claude Code's sessions-index.json files
//! located at ~/.claude/projects/{encoded-path}/sessions-index.json
//!
//! Falls back to reading the first user prompt from .jsonl files when
//! sessions-index.json is not available.

use lazy_regex::regex_replace_all;
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

/// Assistant message entry in .jsonl files
#[derive(Debug, Deserialize)]
struct AssistantJsonlEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    message: Option<AssistantMessage>,
}

/// Message content in .jsonl assistant entries
#[derive(Debug, Deserialize)]
struct AssistantMessage {
    content: Option<Vec<MessageContent>>,
}

/// Content item in assistant message content array
#[derive(Debug, Deserialize)]
struct MessageContent {
    #[serde(rename = "type")]
    content_type: Option<String>,
    text: Option<String>,
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

/// Normalizes a title string for display.
///
/// - Strips ANSI escape sequences to prevent terminal injection
/// - Trims whitespace and replaces newlines with spaces to prevent
///   breaking table formatting
fn normalize_title(s: &str) -> String {
    // Strip ANSI escape sequences (CSI sequences like \x1b[...m)
    let stripped = regex_replace_all!(r"\x1b\[[0-9;]*[A-Za-z]", s, |_| "");
    stripped.trim().replace('\n', " ").replace('\r', "")
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
                return Some(normalize_title(&summary));
            }
            if let Some(first_prompt) = entry.first_prompt {
                // Truncation is handled by the display layer
                return Some(normalize_title(&first_prompt));
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
        // Skip lines that fail to read (continue to next line instead of early return)
        let Ok(line) = line else {
            continue;
        };
        if line.is_empty() {
            continue;
        }

        // Skip lines that fail to parse as JSON (may contain non-JSON data)
        let Ok(entry) = serde_json::from_str::<JsonlEntry>(&line) else {
            continue;
        };

        // Look for the first "user" type entry with message content
        if entry.entry_type.as_deref() == Some("user")
            && let Some(message) = entry.message
            && let Some(content) = message.content
        {
            // Truncation is handled by the display layer
            return Some(normalize_title(&content));
        }
    }

    None
}

/// Retrieves the last assistant text message from a session's .jsonl file.
///
/// Scans the file and returns the text content from the last assistant message
/// that contains a text element. Skips assistant messages that only have tool_use.
pub fn get_last_assistant_message(project_path: &Path, session_id: &str) -> Option<String> {
    let home = dirs::home_dir()?;
    get_last_assistant_message_in_home(&home, project_path, session_id)
}

/// Internal function for testing: allows overriding the home directory.
fn get_last_assistant_message_in_home(
    home: &Path,
    project_path: &Path,
    session_id: &str,
) -> Option<String> {
    let encoded = encode_project_path(project_path);
    let jsonl_path = home
        .join(".claude")
        .join("projects")
        .join(&encoded)
        .join(format!("{session_id}.jsonl"));

    if !jsonl_path.exists() {
        return None;
    }

    let file = File::open(&jsonl_path).ok()?;
    let reader = BufReader::new(file);

    let mut last_text: Option<String> = None;

    for line in reader.lines() {
        let Ok(line) = line else {
            continue;
        };
        if line.is_empty() {
            continue;
        }

        let Ok(entry) = serde_json::from_str::<AssistantJsonlEntry>(&line) else {
            continue;
        };

        // Look for "assistant" type entries
        if entry.entry_type.as_deref() != Some("assistant") {
            continue;
        }

        // Extract text from content array
        if let Some(message) = entry.message
            && let Some(contents) = message.content
        {
            for content in contents {
                if content.content_type.as_deref() == Some("text")
                    && let Some(text) = content.text
                    && !text.is_empty()
                {
                    last_text = Some(normalize_title(&text));
                    break; // Take the first text element in this message
                }
            }
        }
    }

    last_text
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;
    use std::io::Write;
    use tempfile::TempDir;

    /// Creates a test project directory structure with a .jsonl file.
    fn create_test_project_with_jsonl(
        home_dir: &Path,
        project_path: &str,
        session_id: &str,
        jsonl_content: &str,
    ) {
        let encoded = encode_project_path(Path::new(project_path));
        let project_dir = home_dir.join(".claude").join("projects").join(&encoded);
        fs::create_dir_all(&project_dir).unwrap();

        let jsonl_path = project_dir.join(format!("{session_id}.jsonl"));
        let mut file = File::create(&jsonl_path).unwrap();
        file.write_all(jsonl_content.as_bytes()).unwrap();
    }

    // =========================================================================
    // Tests for get_last_assistant_message
    // =========================================================================

    #[rstest]
    #[case::returns_last_text(
        indoc! {r#"
            {"type":"user","message":{"content":"Hello"}}
            {"type":"assistant","message":{"content":[{"type":"text","text":"Hi there!"}]}}
            {"type":"user","message":{"content":"How are you?"}}
            {"type":"assistant","message":{"content":[{"type":"text","text":"I'm doing well, thanks!"}]}}
        "#},
        Some("I'm doing well, thanks!")
    )]
    #[case::with_tool_use_and_text(
        indoc! {r#"
            {"type":"user","message":{"content":"Read the file"}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read"},{"type":"text","text":"I've read the file."}]}}
        "#},
        Some("I've read the file.")
    )]
    #[case::skips_tool_use_only(
        indoc! {r#"
            {"type":"user","message":{"content":"Hello"}}
            {"type":"assistant","message":{"content":[{"type":"text","text":"Here's my response"}]}}
            {"type":"user","message":{"content":"Do something"}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"}]}}
        "#},
        Some("Here's my response")
    )]
    #[case::normalizes_newlines(
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Line 1\nLine 2\nLine 3"}]}}"#,
        Some("Line 1 Line 2 Line 3")
    )]
    fn test_get_last_assistant_message(
        #[case] jsonl_content: &str,
        #[case] expected: Option<&str>,
    ) {
        let temp_dir = TempDir::new().unwrap();
        let home_dir = temp_dir.path();

        let project_path = "/test/project";
        let session_id = "test-session";

        create_test_project_with_jsonl(home_dir, project_path, session_id, jsonl_content);

        let result =
            get_last_assistant_message_in_home(home_dir, Path::new(project_path), session_id);

        assert_eq!(result, expected.map(String::from));
    }

    #[test]
    fn test_get_last_assistant_message_handles_nonexistent_file() {
        let temp_dir = TempDir::new().unwrap();
        let home_dir = temp_dir.path();

        let result = get_last_assistant_message_in_home(
            home_dir,
            Path::new("/nonexistent/path"),
            "test-session",
        );

        assert!(result.is_none());
    }

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

    #[rstest]
    #[case::simple("hello world", "hello world")]
    #[case::with_newline("hello\nworld", "hello world")]
    #[case::with_crlf("hello\r\nworld", "hello world")]
    #[case::with_multiple_newlines("line1\nline2\nline3", "line1 line2 line3")]
    #[case::with_leading_whitespace("  hello  ", "hello")]
    #[case::with_trailing_newline("hello\n", "hello")]
    #[case::with_ansi_color("\x1b[31mred text\x1b[0m", "red text")]
    #[case::with_ansi_bold("\x1b[1mbold\x1b[0m normal", "bold normal")]
    #[case::with_multiple_ansi("\x1b[32m\x1b[1mgreen bold\x1b[0m", "green bold")]
    fn test_normalize_title(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(normalize_title(input), expected);
    }
}
