use crate::commands::cc::types::{Session, SessionStatus};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Dim foreground for idle/secondary text (idle status, repo column, section
/// headers, breadcrumbs, ...). A fixed 256-color grayscale index rather than
/// the ANSI-16 `Color::Gray`/`Color::DarkGray` names, whose actual rendered
/// brightness depends on the terminal's configurable palette.
pub(super) const DIM_FG: Color = Color::Indexed(245);

/// Returns the color for a session status icon.
///
/// Only 3 colors are used: amber for waiting-for-user, green for running,
/// and a single neutral color for every idle status (paused/stopped/ended).
pub(super) fn status_color(status: SessionStatus) -> Color {
    match status {
        SessionStatus::Running => Color::Green,
        SessionStatus::WaitingInput => Color::Yellow,
        SessionStatus::Paused | SessionStatus::Stopped | SessionStatus::Ended => DIM_FG,
    }
}

/// Counts sessions by status.
pub(super) fn count_statuses(sessions: &[Session]) -> (usize, usize, usize, usize) {
    let mut running = 0;
    let mut waiting = 0;
    let mut stopped = 0;
    let mut paused = 0;

    for session in sessions {
        match session.status {
            SessionStatus::Running => running += 1,
            SessionStatus::WaitingInput => waiting += 1,
            SessionStatus::Paused => paused += 1,
            SessionStatus::Stopped | SessionStatus::Ended => stopped += 1,
        }
    }

    (running, waiting, stopped, paused)
}

/// Gets the title display name for a session without external file I/O.
/// Used as fallback when title is not in cache.
/// All outputs are sanitized to strip ANSI escape sequences.
pub(super) fn get_title_display_name_fallback(session: &Session) -> String {
    use crate::commands::cc::claude_sessions;

    if let Some(ref label) = session.label {
        return claude_sessions::normalize_title(label);
    }

    let raw_title = session
        .cwd
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from)
        .unwrap_or_else(|| session.cwd.display().to_string());
    claude_sessions::normalize_title(&raw_title)
}

/// Gets the repo name rendered in the repo column. Falls back to the cwd
/// basename when the repo label is not yet cached (first frame before the
/// cache is populated, or a path outside any git repo).
/// All outputs are sanitized to strip ANSI escape sequences.
pub(super) fn get_session_info(session: &Session, repo: &str) -> String {
    use crate::commands::cc::claude_sessions;

    let raw = if !repo.is_empty() {
        repo.to_string()
    } else {
        session
            .cwd
            .file_name()
            .and_then(|n| n.to_str())
            .map(String::from)
            .unwrap_or_else(|| session.cwd.display().to_string())
    };
    claude_sessions::normalize_title(&raw)
}

/// Splits text into spans, highlighting portions that match any of the search words.
///
/// Uses case-insensitive matching consistent with the existing search logic.
/// When multiple words produce overlapping match ranges, they are merged.
/// Highlighted spans receive `fg(Color::Yellow)` and `BOLD` on top of `base_style`.
pub(super) fn highlight_matches<'a>(text: &str, query: &str, base_style: Style) -> Vec<Span<'a>> {
    let words: Vec<&str> = query.split_whitespace().collect();
    if words.is_empty() || text.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    // Build a mapping from lowercased byte offsets to original byte offsets.
    // to_lowercase() can change byte length (e.g. 'İ' -> "i\u{307}"),
    // so we must map match positions in the lowercased string back to the original.
    let mut text_lower = String::new();
    let mut lower_to_orig: Vec<usize> = Vec::new();
    for (orig_offset, ch) in text.char_indices() {
        for lower_ch in ch.to_lowercase() {
            let lower_start = text_lower.len();
            text_lower.push(lower_ch);
            // Map each byte of the lowercased char to the original char's byte offset
            for _ in lower_start..text_lower.len() {
                lower_to_orig.push(orig_offset);
            }
        }
    }
    // Sentinel: map end-of-lowered-string to end-of-original-string
    lower_to_orig.push(text.len());

    // Collect all match ranges (byte offsets in the original string)
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    for word in &words {
        let word_lower = word.to_lowercase();
        let mut start = 0;
        while let Some(pos) = text_lower[start..].find(&word_lower) {
            let lower_start = start + pos;
            let lower_end = lower_start + word_lower.len();
            let orig_start = lower_to_orig[lower_start];
            let mut orig_end = lower_to_orig[lower_end];
            // When lowercasing expands bytes (e.g. İ -> i\u{307}), a match
            // within the expansion maps start and end to the same original
            // offset. Extend to cover the full original character.
            if orig_end <= orig_start {
                orig_end = text[orig_start..]
                    .chars()
                    .next()
                    .map_or(text.len(), |c| orig_start + c.len_utf8());
            }
            ranges.push((orig_start, orig_end));
            // Advance by one character (not one byte) to stay on a char boundary
            start = lower_start
                + text_lower[lower_start..]
                    .chars()
                    .next()
                    .map_or(1, |c| c.len_utf8());
        }
    }

    if ranges.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    // Sort by start position, then by end position descending to prefer longer matches
    ranges.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));

    // Merge overlapping ranges
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for range in ranges {
        if let Some(last) = merged.last_mut()
            && range.0 <= last.1
        {
            last.1 = last.1.max(range.1);
            continue;
        }
        merged.push(range);
    }

    // Build spans from merged ranges
    let highlight_style = base_style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    let mut cursor = 0;

    for (start, end) in merged {
        if cursor < start {
            spans.push(Span::styled(text[cursor..start].to_string(), base_style));
        }
        spans.push(Span::styled(text[start..end].to_string(), highlight_style));
        cursor = end;
    }

    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_string(), base_style));
    }

    spans
}

/// Truncates a string to fit within the specified display width.
/// Uses unicode display width for proper handling of CJK characters.
pub(super) fn truncate(s: &str, max_width: usize) -> String {
    let display_width = s.width();
    if display_width <= max_width {
        s.to_string()
    } else if max_width < 3 {
        truncate_to_width(s, max_width)
    } else {
        let truncated = truncate_to_width(s, max_width - 3);
        format!("{}...", truncated)
    }
}

/// Truncates a string to fit within the specified display width.
fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut current_width = 0;

    for c in s.chars() {
        let char_width = c.width().unwrap_or(0);
        if current_width + char_width > max_width {
            break;
        }
        result.push(c);
        current_width += char_width;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::tui::ui::test_support::create_test_session;
    use crate::commands::cc::types::TmuxInfo;
    use rstest::rstest;

    #[test]
    fn test_count_statuses() {
        let sessions = vec![
            {
                let mut s = create_test_session("1");
                s.status = SessionStatus::Running;
                s
            },
            {
                let mut s = create_test_session("2");
                s.status = SessionStatus::Running;
                s
            },
            {
                let mut s = create_test_session("3");
                s.status = SessionStatus::WaitingInput;
                s
            },
            {
                let mut s = create_test_session("4");
                s.status = SessionStatus::Stopped;
                s
            },
        ];

        let (running, waiting, stopped, paused) = count_statuses(&sessions);
        assert_eq!(running, 2);
        assert_eq!(waiting, 1);
        assert_eq!(stopped, 1);
        assert_eq!(paused, 0);
    }

    #[test]
    fn test_count_statuses_with_paused() {
        let sessions = vec![
            {
                let mut s = create_test_session("1");
                s.status = SessionStatus::Running;
                s
            },
            {
                let mut s = create_test_session("2");
                s.status = SessionStatus::Paused;
                s
            },
            {
                let mut s = create_test_session("3");
                s.status = SessionStatus::Paused;
                s
            },
        ];

        let (running, waiting, stopped, paused) = count_statuses(&sessions);
        assert_eq!(running, 1);
        assert_eq!(waiting, 0);
        assert_eq!(stopped, 0);
        assert_eq!(paused, 2);
    }

    #[test]
    fn test_get_title_display_name_fallback_ignores_tmux() {
        // Tmux session:window is no longer used as a label fallback; the
        // row already shows the repo name instead. Without an explicit
        // label, the fallback is the cwd basename.
        let mut session = create_test_session("test");
        session.tmux_info = Some(TmuxInfo {
            session_name: "dev".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%0".to_string(),
        });
        assert_eq!(get_title_display_name_fallback(&session), "project");
    }

    #[test]
    fn test_get_title_display_name_fallback_to_cwd() {
        // When cache is not available and no tmux, falls back to cwd
        let session = create_test_session("test");
        assert_eq!(get_title_display_name_fallback(&session), "project");
    }

    #[rstest]
    #[case::repo("armyknife", "armyknife")]
    #[case::empty_falls_back_to_cwd_basename("", "project")]
    fn test_get_session_info_formats_repo(#[case] repo: &str, #[case] expected: &str) {
        let session = create_test_session("test");
        assert_eq!(get_session_info(&session, repo), expected);
    }

    #[rstest]
    #[case::short("hello", 10, "hello")]
    #[case::exact("hello", 5, "hello")]
    #[case::truncate("hello world", 8, "hello...")]
    #[case::cjk_short("日本語", 10, "日本語")]
    #[case::cjk_exact("日本語", 6, "日本語")]
    #[case::cjk_truncate("日本語テスト", 8, "日本...")]
    fn test_truncate(#[case] input: &str, #[case] max_width: usize, #[case] expected: &str) {
        assert_eq!(truncate(input, max_width), expected);
    }

    #[rstest]
    #[case::running(SessionStatus::Running, Color::Green)]
    #[case::waiting_input(SessionStatus::WaitingInput, Color::Yellow)]
    #[case::paused(SessionStatus::Paused, DIM_FG)]
    #[case::stopped(SessionStatus::Stopped, DIM_FG)]
    #[case::ended(SessionStatus::Ended, DIM_FG)]
    fn test_status_color(#[case] status: SessionStatus, #[case] expected: Color) {
        assert_eq!(status_color(status), expected);
    }

    // =========================================================================
    // highlight_matches tests
    // =========================================================================

    /// Each expected element is (content, is_highlighted).
    #[rstest]
    #[case::empty_query("webapp:dev", "", &[("webapp:dev", false)])]
    #[case::no_match("webapp:dev", "xyz", &[("webapp:dev", false)])]
    #[case::single_word("webapp:dev", "web", &[("web", true), ("app:dev", false)])]
    #[case::case_insensitive("WebApp", "web", &[("Web", true), ("App", false)])]
    #[case::multiple_words("webapp:dev", "web dev", &[("web", true), ("app:", false), ("dev", true)])]
    #[case::overlapping_ranges("abcd", "ab bc", &[("abc", true), ("d", false)])]
    #[case::multiple_occurrences("abcabc", "ab", &[("ab", true), ("c", false), ("ab", true), ("c", false)])]
    #[case::empty_text("", "web", &[("", false)])]
    #[case::unicode_byte_length_increase("İstanbul City", "city", &[("İstanbul ", false), ("City", true)])]
    #[case::unicode_byte_length_decrease("\u{212A}elvin", "kelvin", &[("\u{212A}elvin", true)])]
    #[case::multibyte_overlapping_match("ああいああ", "ああ", &[("ああ", true), ("い", false), ("ああ", true)])]
    fn test_highlight_matches(
        #[case] text: &str,
        #[case] query: &str,
        #[case] expected: &[(&str, bool)],
    ) {
        let base = Style::default().add_modifier(Modifier::BOLD);
        let highlight = base.fg(Color::Yellow).add_modifier(Modifier::BOLD);

        let spans = highlight_matches(text, query, base);

        assert_eq!(spans.len(), expected.len(), "span count mismatch");
        for (span, &(content, is_highlighted)) in spans.iter().zip(expected) {
            assert_eq!(span.content, content);
            assert_eq!(span.style, if is_highlighted { highlight } else { base });
        }
    }
}
