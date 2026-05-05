//! Reads the user-visible text inside the Claude Code TUI input box for a
//! given tmux pane.
//!
//! Used by `auto_compact` and `sweep` as a "did the user type something"
//! probe: comparing two captures over time tells us whether a Stopped
//! session is being touched without depending on the terminal cursor
//! (which moves with every TUI redraw and is structurally tied to layout
//! rather than user input) or pty atime (which drifted on macOS devfs in
//! ways unrelated to keystrokes).
//!
//! The Claude Code TUI draws an input region delimited by two horizontal
//! rules of `─` (U+2500). We capture the pane via `tmux capture-pane -p`,
//! find the bottom-most pair of rule lines, and return the body lines
//! between them with the prompt decoration stripped. When the bottom of
//! the pane shows something other than the input box (permission prompt,
//! mode picker, startup splash) no rule pair is present and we return
//! `None` — callers must treat that as "no observation" rather than
//! "input is empty".

use crate::infra::tmux;

/// Returns the text the user has typed into the Claude Code TUI input
/// box for `pane_id`, or `None` if no input box is currently rendered.
pub fn get_pane_input_text(pane_id: &str) -> Option<String> {
    let raw = tmux::capture_pane(pane_id)?;
    extract_input_text(&raw)
}

fn extract_input_text(raw: &str) -> Option<String> {
    let lines: Vec<&str> = raw.lines().collect();

    let bottom_rule = lines.iter().rposition(|l| is_horizontal_rule(l))?;
    let top_rule = lines[..bottom_rule]
        .iter()
        .rposition(|l| is_horizontal_rule(l))?;

    // Lines between the rules are the input box body. The first row
    // begins with `❯ ` (prompt), continuation rows with two leading
    // spaces; both pieces of decoration are layout rather than content,
    // so dropping them lets a one-line message and the same message
    // wrapped across rows compare equal as long as the wrap point
    // matches. We also drop trailing whitespace per row so that a pane
    // resize between two captures (which can change tmux's right-pad
    // length on some terminal/version combinations) doesn't flip an
    // unchanged prompt into a UserTyping verdict.
    let body: Vec<String> = lines[top_rule + 1..bottom_rule]
        .iter()
        .map(|l| strip_decoration(l).trim_end().to_string())
        .collect();
    Some(body.join("\n"))
}

/// `─` (U+2500) is the only legitimate character in a Claude Code input
/// box rule. Allow trailing whitespace because tmux capture-pane on some
/// configurations right-pads short rules to the pane width with spaces;
/// require at least a few rule characters so a stray box-drawing glyph
/// in normal output (e.g. inside an assistant code block) doesn't get
/// mistaken for the input box.
fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim_end();
    let count = trimmed.chars().filter(|c| *c == '─').count();
    count >= 8 && trimmed.chars().all(|c| c == '─')
}

/// Strips the prompt marker `❯ ` from the first line of an input box body
/// and the two-space continuation indent from subsequent lines. Accepts
/// `❯` without a trailing space too, because tmux capture-pane (and some
/// processing pipelines) right-trim lines down to bare `❯` when the input
/// is empty.
fn strip_decoration(line: &str) -> &str {
    line.strip_prefix("❯ ")
        .or_else(|| line.strip_prefix("❯"))
        .or_else(|| line.strip_prefix("  "))
        .unwrap_or(line)
}

#[cfg(test)]
mod tests {
    //! Samples below mirror real `tmux capture-pane -p` output from
    //! Claude Code TUI v2.1.x; the rule character is U+2500 BOX DRAWINGS
    //! LIGHT HORIZONTAL and rule width follows pane width.
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    // 60-char rule to mimic a moderate-width pane without making the
    // literal unwieldy.
    const RULE: &str = "────────────────────────────────────────────────────────────";

    #[test]
    fn empty_input_box_returns_empty_string() {
        let raw = format!(
            indoc! {"
                Some earlier output line.
                Another earlier line.

                {rule}
                ❯
                {rule}
                  Opus 4.7 (1M context) | 100k tok (12%)
                  -- INSERT -- ⏵⏵ accept edits on
            "},
            rule = RULE,
        );
        assert_eq!(extract_input_text(&raw), Some(String::new()));
    }

    #[test]
    fn single_line_input_returns_typed_text() {
        let raw = format!(
            indoc! {"
                {rule}
                ❯ hello world
                {rule}
                  -- INSERT --
            "},
            rule = RULE,
        );
        assert_eq!(extract_input_text(&raw), Some("hello world".to_string()));
    }

    #[test]
    fn multi_line_input_joins_continuation_rows() {
        // Continuation rows are indented with two spaces in the capture;
        // both Shift+Enter inserts and soft-wraps look the same to
        // capture-pane, so the parser does not need to tell them apart.
        let raw = format!(
            indoc! {"
                {rule}
                ❯ first line
                  second line
                  third line
                {rule}
                  -- INSERT --
            "},
            rule = RULE,
        );
        assert_eq!(
            extract_input_text(&raw),
            Some(
                indoc! {"
                    first line
                    second line
                    third line
                "}
                .trim_end_matches('\n')
                .to_string(),
            ),
        );
    }

    #[test]
    fn typing_changes_the_extracted_string() {
        // Documents the comparison the activity probe will make: arm and
        // wake captures are extracted then compared as strings.
        let arm = format!(
            indoc! {"
                {rule}
                ❯
                {rule}
            "},
            rule = RULE,
        );
        let wake = format!(
            indoc! {"
                {rule}
                ❯ hi
                {rule}
            "},
            rule = RULE,
        );
        assert_ne!(extract_input_text(&arm), extract_input_text(&wake));
    }

    #[test]
    fn no_rules_returns_none() {
        // Permission prompt mode and similar overlays don't draw the
        // input box; callers must treat None as "no observation".
        let raw = indoc! {"
            Do you want to proceed?
            ❯ 1. Yes
              2. No

            Esc to cancel · Tab to amend
        "};
        assert_eq!(extract_input_text(raw), None);
    }

    #[test]
    fn one_rule_only_returns_none() {
        let raw = format!(
            indoc! {"
                {rule}
                ❯ orphan
            "},
            rule = RULE,
        );
        assert_eq!(extract_input_text(&raw), None);
    }

    #[test]
    fn picks_last_pair_when_capture_history_has_earlier_rules() {
        // capture-pane outputs the visible pane only by default, but a
        // deep capture (-S) can include prior turns whose rendered
        // output happened to contain rule-like sequences. The probe
        // anchors on the bottom-most pair so the live input box is what
        // gets compared.
        let raw = format!(
            indoc! {"
                Earlier transcript chunk
                {rule}
                ❯ stale text from history
                {rule}
                Some assistant output between turns.
                {rule}
                ❯ live text
                {rule}
                  -- INSERT --
            "},
            rule = RULE,
        );
        assert_eq!(extract_input_text(&raw), Some("live text".to_string()));
    }

    #[test]
    fn trailing_whitespace_is_ignored() {
        // tmux capture-pane on some terminal/tmux combinations right-pads
        // each line to pane width. The same prompt captured under two
        // different pane widths must still extract to the same string,
        // otherwise a window resize between arm and wake would falsely
        // trip UserTyping. We produce trailing whitespace via a `{pad}`
        // substitution so editors don't strip it from the source.
        let narrow = format!(
            indoc! {"
                {rule}
                ❯ hi{pad}
                {rule}
            "},
            rule = RULE,
            pad = "",
        );
        let padded = format!(
            indoc! {"
                {rule}
                ❯ hi{pad}
                {rule}
            "},
            rule = RULE,
            pad = "              ",
        );
        assert_eq!(extract_input_text(&narrow), extract_input_text(&padded));
    }

    #[rstest]
    #[case::all_rules("──────────────────")]
    // tmux capture-pane on some configurations right-pads short rules
    // to the pane width with spaces; the detector must accept that.
    #[case::trailing_whitespace("──────────────────   ")]
    fn rule_detector_accepts(#[case] line: &str) {
        assert!(is_horizontal_rule(line));
    }

    #[rstest]
    #[case::too_short("─────")]
    #[case::contains_text("──── input ───────")]
    #[case::leading_text("hi ──────────────")]
    #[case::heavy_rule("━━━━━━━━━━━━━━━━━━")]
    #[case::empty("")]
    fn rule_detector_rejects(#[case] line: &str) {
        assert!(!is_horizontal_rule(line));
    }
}
