//! Claude Code session information reader.
//!
//! Reads session metadata from Claude Code's .jsonl transcript files
//! located at ~/.claude/projects/{encoded-path}/{session-id}.jsonl

use lazy_regex::regex_replace_all;
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Initial buffer size for reading from end of file (8KB)
const INITIAL_READ_SIZE: usize = 8 * 1024;

/// Maximum buffer size to prevent reading too much data (64KB)
const MAX_READ_SIZE: usize = 64 * 1024;

/// Maximum number of lines to scan from the end before giving up
const MAX_LINES_TO_SCAN: usize = 20;

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
    let home = crate::shared::dirs::home_dir()?;
    let encoded = encode_project_path(project_path);
    Some(home.join(".claude").join("projects").join(encoded))
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
pub fn normalize_title(s: &str) -> String {
    // Strip ANSI escape sequences (CSI sequences like \x1b[...m)
    let stripped = regex_replace_all!(r"\x1b\[[0-9;]*[A-Za-z]", s, |_| "");
    stripped.trim().replace('\n', " ").replace('\r', "")
}

/// Retrieves the session title by reading the first user prompt from the .jsonl file.
pub fn get_session_title(project_path: &Path, session_id: &str) -> Option<String> {
    get_title_from_jsonl(project_path, session_id)
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
    let home = crate::shared::dirs::home_dir()?;
    get_last_assistant_message_in_home(&home, project_path, session_id)
}

/// Retrieves all conversation text from a session's .jsonl file for search.
///
/// Returns a concatenated string of all user messages and assistant text responses.
/// Excludes tool outputs (like Bash, Read, etc.) to focus on conversation content.
pub fn get_conversation_text(project_path: &Path, session_id: &str) -> Option<String> {
    let home = crate::shared::dirs::home_dir()?;
    get_conversation_text_in_home(&home, project_path, session_id)
}

/// Internal function for testing: allows overriding the home directory.
fn get_conversation_text_in_home(
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

    let mut texts: Vec<String> = Vec::new();

    for line in reader.lines() {
        let Ok(line) = line else {
            continue;
        };
        if line.is_empty() {
            continue;
        }

        // Try to parse as user message
        if let Ok(entry) = serde_json::from_str::<JsonlEntry>(&line)
            && entry.entry_type.as_deref() == Some("user")
            && let Some(message) = entry.message
            && let Some(content) = message.content
            && !content.is_empty()
        {
            texts.push(normalize_title(&content));
            continue;
        }

        // Try to parse as assistant message
        if let Ok(entry) = serde_json::from_str::<AssistantJsonlEntry>(&line)
            && entry.entry_type.as_deref() == Some("assistant")
            && let Some(message) = entry.message
            && let Some(contents) = message.content
        {
            for content in contents {
                if content.content_type.as_deref() == Some("text")
                    && let Some(text) = content.text
                    && !text.is_empty()
                {
                    texts.push(normalize_title(&text));
                }
            }
        }
    }

    if texts.is_empty() {
        None
    } else {
        Some(texts.join(" "))
    }
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

    // Try reverse reading first for large files
    if let Some(text) = read_last_assistant_message_reverse(&file) {
        return Some(text);
    }

    // Fallback to forward reading for small files or edge cases
    read_last_assistant_message_forward(&file)
}

/// Reads the last assistant message by scanning from the end of file.
/// Uses progressive buffer expansion: starts with a small buffer and doubles
/// if no text message is found (e.g., when last entries are tool_use only).
fn read_last_assistant_message_reverse(file: &File) -> Option<String> {
    let metadata = file.metadata().ok()?;
    let file_size = metadata.len();

    // For very small files, use forward reading
    if file_size < INITIAL_READ_SIZE as u64 {
        return None;
    }

    let mut read_size = INITIAL_READ_SIZE;

    // Progressive expansion: try small buffer first, expand if needed
    while read_size <= MAX_READ_SIZE {
        let actual_read = std::cmp::min(read_size as u64, file_size) as usize;

        if let Some(text) = try_read_last_lines(file, actual_read, file_size) {
            return Some(text);
        }

        // Double the buffer size and try again
        read_size *= 2;
    }

    None
}

/// Attempts to read the last lines from a file and find an assistant text message.
fn try_read_last_lines(file: &File, read_size: usize, file_size: u64) -> Option<String> {
    let mut reader = BufReader::new(file);

    // Seek to near the end
    reader.seek(SeekFrom::End(-(read_size as i64))).ok()?;

    let mut buffer = vec![0u8; read_size];
    reader.read_exact(&mut buffer).ok()?;

    // Find the last complete lines by locating newlines
    let content = String::from_utf8_lossy(&buffer);

    // Skip the first line only when reading from middle of file (might be partial).
    // When read_size >= file_size, we're reading from the start, so all lines are complete.
    let complete_content = if read_size as u64 >= file_size {
        &content[..]
    } else {
        let first_newline = content.find('\n')?;
        &content[first_newline + 1..]
    };

    // Collect lines and scan from the end
    let lines: Vec<&str> = complete_content.lines().collect();

    for line in lines.iter().rev().take(MAX_LINES_TO_SCAN) {
        if line.is_empty() {
            continue;
        }

        let Ok(entry) = serde_json::from_str::<AssistantJsonlEntry>(line) else {
            continue;
        };

        if entry.entry_type.as_deref() != Some("assistant") {
            continue;
        }

        if let Some(message) = entry.message
            && let Some(contents) = message.content
        {
            for content in contents {
                if content.content_type.as_deref() == Some("text")
                    && let Some(text) = content.text
                    && !text.is_empty()
                {
                    return Some(normalize_title(&text));
                }
            }
        }
    }

    None
}

/// Fallback: reads the entire file forward (original implementation).
fn read_last_assistant_message_forward(file: &File) -> Option<String> {
    let mut file = BufReader::new(file);
    // Reset to beginning in case it was seeked
    file.seek(SeekFrom::Start(0)).ok()?;

    let mut last_text: Option<String> = None;

    for line in file.lines() {
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
        std::fs::create_dir_all(&project_dir).unwrap();

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

    // =========================================================================
    // Tests for get_conversation_text
    // =========================================================================

    #[rstest]
    #[case::user_and_assistant(
        indoc! {r#"
            {"type":"user","message":{"content":"Hello"}}
            {"type":"assistant","message":{"content":[{"type":"text","text":"Hi there!"}]}}
            {"type":"user","message":{"content":"How are you?"}}
            {"type":"assistant","message":{"content":[{"type":"text","text":"I'm doing well!"}]}}
        "#},
        Some("Hello Hi there! How are you? I'm doing well!")
    )]
    #[case::excludes_tool_use(
        indoc! {r#"
            {"type":"user","message":{"content":"Read file"}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read"}]}}
            {"type":"assistant","message":{"content":[{"type":"text","text":"I've read it."}]}}
        "#},
        Some("Read file I've read it.")
    )]
    #[case::includes_text_with_tool_use(
        indoc! {r#"
            {"type":"user","message":{"content":"Check this"}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"},{"type":"text","text":"Running command..."}]}}
        "#},
        Some("Check this Running command...")
    )]
    fn test_get_conversation_text(#[case] jsonl_content: &str, #[case] expected: Option<&str>) {
        let temp_dir = TempDir::new().unwrap();
        let home_dir = temp_dir.path();

        let project_path = "/test/project";
        let session_id = "test-session";

        create_test_project_with_jsonl(home_dir, project_path, session_id, jsonl_content);

        let result = get_conversation_text_in_home(home_dir, Path::new(project_path), session_id);

        assert_eq!(result, expected.map(String::from));
    }

    #[test]
    fn test_get_conversation_text_handles_nonexistent_file() {
        let temp_dir = TempDir::new().unwrap();
        let home_dir = temp_dir.path();

        let result =
            get_conversation_text_in_home(home_dir, Path::new("/nonexistent/path"), "test-session");

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
    fn test_get_session_title_nonexistent_file() {
        let path = Path::new("/nonexistent/path/that/does/not/exist");
        let result = get_session_title(path, "test-session-id");
        assert!(result.is_none());
    }

    #[rstest]
    #[case::simple("hello world", "hello world")]
    #[case::with_newline("hello\nworld", "hello world")]
    #[case::with_crlf("hello\r\nworld", "hello world")]
    #[case::with_multiple_newlines(indoc! {"
        line1
        line2
        line3"}, "line1 line2 line3")]
    #[case::with_leading_whitespace("  hello  ", "hello")]
    #[case::with_trailing_newline("hello\n", "hello")]
    #[case::with_ansi_color("\x1b[31mred text\x1b[0m", "red text")]
    #[case::with_ansi_bold("\x1b[1mbold\x1b[0m normal", "bold normal")]
    #[case::with_multiple_ansi("\x1b[32m\x1b[1mgreen bold\x1b[0m", "green bold")]
    fn test_normalize_title(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(normalize_title(input), expected);
    }

    // =========================================================================
    // Tests for reverse reading optimization
    // =========================================================================

    #[test]
    fn test_reverse_read_finds_last_message_in_large_file() {
        let temp_dir = TempDir::new().unwrap();
        let home_dir = temp_dir.path();

        let project_path = "/test/project";
        let session_id = "large-session";

        // Create a file larger than REVERSE_READ_BUFFER_SIZE (64KB)
        let mut content = String::new();
        for i in 0..500 {
            content.push_str(&format!(
                r#"{{"type":"user","message":{{"content":"Question {i}"}}}}"#
            ));
            content.push('\n');
            content.push_str(&format!(
                r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"Answer {i}"}}]}}}}"#
            ));
            content.push('\n');
        }

        create_test_project_with_jsonl(home_dir, project_path, session_id, &content);

        let result =
            get_last_assistant_message_in_home(home_dir, Path::new(project_path), session_id);

        // Should find the last message (Answer 499)
        assert_eq!(result, Some("Answer 499".to_string()));
    }

    #[test]
    fn test_small_file_uses_forward_read() {
        let temp_dir = TempDir::new().unwrap();
        let home_dir = temp_dir.path();

        let project_path = "/test/project";
        let session_id = "small-session";

        // Create a small file (less than 64KB)
        let content = indoc! {r#"
            {"type":"user","message":{"content":"Hello"}}
            {"type":"assistant","message":{"content":[{"type":"text","text":"First response"}]}}
            {"type":"user","message":{"content":"More"}}
            {"type":"assistant","message":{"content":[{"type":"text","text":"Last response"}]}}
        "#};

        create_test_project_with_jsonl(home_dir, project_path, session_id, content);

        let result =
            get_last_assistant_message_in_home(home_dir, Path::new(project_path), session_id);

        // Should still find the last message via forward read
        assert_eq!(result, Some("Last response".to_string()));
    }

    #[test]
    fn test_reverse_read_skips_tool_use_only_messages() {
        let temp_dir = TempDir::new().unwrap();
        let home_dir = temp_dir.path();

        let project_path = "/test/project";
        let session_id = "tool-use-session";

        // Create a large file where the last assistant message is tool_use only
        let mut content = String::new();
        // Add padding to exceed 64KB
        for i in 0..400 {
            content.push_str(&format!(
                r#"{{"type":"user","message":{{"content":"Padding question {i}"}}}}"#
            ));
            content.push('\n');
            content.push_str(&format!(
                r#"{{"type":"assistant","message":{{"content":[{{"type":"text","text":"Padding answer {i}"}}]}}}}"#
            ));
            content.push('\n');
        }
        // Add a text message followed by tool_use only messages
        content.push_str(
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"This is the real last text"}]}}"#,
        );
        content.push('\n');
        content.push_str(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"}]}}"#,
        );
        content.push('\n');
        content.push_str(
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read"}]}}"#,
        );
        content.push('\n');

        create_test_project_with_jsonl(home_dir, project_path, session_id, &content);

        let result =
            get_last_assistant_message_in_home(home_dir, Path::new(project_path), session_id);

        // Should find the last message with text content, not tool_use
        assert_eq!(result, Some("This is the real last text".to_string()));
    }
}
