//! Reads the user-visible text inside the Claude Code or Codex composer for
//! a given tmux pane.
//!
//! The Claude Code input extractor also serves `auto_compact`.
//!
//! The Claude Code TUI draws an input region delimited by two horizontal
//! rules of `─` (U+2500). We capture the pane via `tmux capture-pane -p`,
//! find the bottom-most pair of rule lines, and return the body lines
//! between them with the prompt decoration stripped. When the bottom of
//! the pane shows something other than the input box (permission prompt,
//! mode picker, startup splash) no rule pair is present and we return
//! `None` — callers must treat that as "no observation" rather than
//! "input is empty".

use crate::commands::agent::types::Engine;
use crate::infra::tmux;

/// Returns the text the user has typed into the Claude Code TUI input
/// box for `pane_id`, or `None` if no input box is currently rendered.
pub fn get_pane_input_text(pane_id: &str) -> Option<String> {
    let raw = tmux::capture_pane(pane_id)?;
    extract_input_text(&raw)
}

/// Returns whether `pane_id` contains an unsent composer draft for `engine`.
/// Returns `None` when the composer cannot be recognized.
pub fn pane_has_draft(pane_id: &str, engine: Engine) -> Option<bool> {
    match engine {
        Engine::Claude => get_pane_input_text(pane_id).map(|text| !text.is_empty()),
        Engine::Codex => {
            let raw = tmux::capture_pane_with_escapes(pane_id)?;
            extract_codex_composer_has_draft(&raw)
        }
    }
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

fn extract_codex_composer_has_draft(raw: &str) -> Option<bool> {
    let line = raw.lines().rev().find(|line| line.contains('›'))?;
    parse_codex_composer_line(line)
}

fn parse_codex_composer_line(line: &str) -> Option<bool> {
    let mut visible = Vec::new();
    let mut dim = false;
    let mut saw_sgr = false;
    let mut chars = line.chars().peekable();

    while let Some(character) = chars.next() {
        if character != '\u{1b}' {
            visible.push((character, dim, saw_sgr));
            continue;
        }

        if chars.next()? != '[' {
            return None;
        }

        let mut parameters = String::new();
        loop {
            let character = chars.next()?;
            if character == 'm' {
                saw_sgr = true;
                for parameter in parameters.split(';') {
                    let parameter = if parameter.is_empty() { "0" } else { parameter };
                    match parameter.parse::<u16>().ok()? {
                        0 => dim = false,
                        2 => dim = true,
                        22 => dim = false,
                        _ => {}
                    }
                }
                break;
            }
            if !character.is_ascii_digit() && character != ';' {
                return None;
            }
            parameters.push(character);
        }
    }

    // Placeholder wording can change or be localized; the dim SGR style is
    // the signal that distinguishes it from user-entered text.
    let prompt_index = visible
        .iter()
        .position(|(character, _, _)| *character == '›')?;
    if !visible[prompt_index].2 {
        return None;
    }
    if visible[..prompt_index]
        .iter()
        .any(|(character, _, _)| !character.is_whitespace())
    {
        return None;
    }

    let (_, is_dim, _) = visible[prompt_index + 1..]
        .iter()
        .find(|(character, _, _)| !character.is_whitespace())?;
    Some(!is_dim)
}

#[cfg(test)]
mod tests {
    //! Claude fixtures mirror `tmux capture-pane -p` output from v2.1.x;
    //! Codex fixtures model the composer styling returned by `capture-pane -e`.
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    // 60-char rule to mimic a moderate-width pane without making the
    // literal unwieldy.
    const RULE: &str = "────────────────────────────────────────────────────────────";

    const ESC: char = '\u{1b}';

    /// Builds a capture body by substituting `{rule}` with the test's
    /// horizontal-rule fixture. Lets each parameterised case stay readable
    /// without re-typing 60 box-drawing chars in every literal.
    fn render(template: &str) -> String {
        template.replace("{rule}", RULE)
    }

    #[rstest]
    // The input box is a `─`-rule pair; an empty `❯` line between them
    // must extract to the empty string (not to None, which is reserved
    // for "no input box visible").
    #[case::empty_input_box(
        indoc! {"
            Some earlier output line.
            Another earlier line.

            {rule}
            ❯
            {rule}
              Opus 4.7 (1M context) | 100k tok (12%)
              -- INSERT -- ⏵⏵ accept edits on
        "},
        Some(""),
    )]
    #[case::single_line_input(
        indoc! {"
            {rule}
            ❯ hello world
            {rule}
              -- INSERT --
        "},
        Some("hello world"),
    )]
    // Continuation rows are indented with two spaces in the capture;
    // both Shift+Enter inserts and soft-wraps look the same to
    // capture-pane, so the parser does not need to tell them apart.
    #[case::multi_line_input(
        indoc! {"
            {rule}
            ❯ first line
              second line
              third line
            {rule}
              -- INSERT --
        "},
        Some(indoc! {"
            first line
            second line
            third line"}),
    )]
    // Permission prompt mode and similar overlays don't draw the input
    // box; callers must treat None as "no observation".
    #[case::no_rules_at_all(
        indoc! {"
            Do you want to proceed?
            ❯ 1. Yes
              2. No

            Esc to cancel · Tab to amend
        "},
        None,
    )]
    #[case::only_one_rule(
        indoc! {"
            {rule}
            ❯ orphan
        "},
        None,
    )]
    // capture-pane outputs the visible pane only by default, but a deep
    // capture (-S) can include prior turns whose rendered output
    // happened to contain rule-like sequences. The probe anchors on the
    // bottom-most pair so the live input box is what gets compared.
    #[case::history_contains_earlier_rules(
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
        Some("live text"),
    )]
    fn extracts_input_text(#[case] template: &str, #[case] expected: Option<&str>) {
        let raw = render(template);
        assert_eq!(extract_input_text(&raw), expected.map(|s| s.to_string()),);
    }

    #[rstest]
    // tmux capture-pane on some terminal/tmux combinations right-pads
    // each line to pane width. Two captures of the same prompt taken at
    // different pane widths must extract to the same string, otherwise
    // a window resize between arm and wake would falsely trip
    // UserTyping. The pad is applied by the test runner so editors
    // don't strip the trailing whitespace from the source.
    #[case::different_padding_widths("", "              ")]
    #[case::same_padding_either_side("    ", "    ")]
    fn capture_padding_does_not_affect_extracted_text(#[case] pad_a: &str, #[case] pad_b: &str) {
        let raw_a = render(&format!(
            indoc! {"
                {{rule}}
                ❯ hi{pad}
                {{rule}}
            "},
            pad = pad_a,
        ));
        let raw_b = render(&format!(
            indoc! {"
                {{rule}}
                ❯ hi{pad}
                {{rule}}
            "},
            pad = pad_b,
        ));
        assert_eq!(extract_input_text(&raw_a), extract_input_text(&raw_b));
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

    #[rstest]
    #[case::dim_placeholder_is_empty(
        format!("{ESC}[1m›{ESC}[0m {ESC}[2mplaceholder text{ESC}[0m"),
        Some(false),
    )]
    #[case::normal_text_is_a_draft(
        format!("{ESC}[1m›{ESC}[0m draft text"),
        Some(true),
    )]
    #[case::last_composer_is_used(
        format!("{ESC}[1m›{ESC}[0m stale text\n{ESC}[1m›{ESC}[0m {ESC}[2mplaceholder{ESC}[0m"),
        Some(false),
    )]
    #[case::no_prompt_is_unknown("ordinary output".to_string(), None)]
    #[case::missing_body_is_unknown(format!("{ESC}[1m›{ESC}[0m"), None)]
    #[case::unstyled_prompt_is_unknown("› placeholder text".to_string(), None)]
    #[case::malformed_escape_is_unknown(format!("{ESC}[1m›{ESC}[2"), None)]
    fn detects_codex_composer_draft(#[case] raw: String, #[case] expected: Option<bool>) {
        assert_eq!(extract_codex_composer_has_draft(&raw), expected);
    }
}
