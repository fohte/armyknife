//! Strips `<`/`>` from values embedded in this crate's XML-style envelopes
//! (e.g. `<delegation-update>`, `<peer-message>`) before they're written to a
//! notification body -- without this, a value chosen by an untrusted or
//! external party (a git branch name, a `--from` argument) could close the
//! envelope early and inject text the receiving session would read as
//! free-standing, unwrapped content instead of part of the envelope.

pub fn strip_angle_brackets(value: &str) -> String {
    value.chars().filter(|c| *c != '<' && *c != '>').collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::no_brackets("plain text", "plain text")]
    #[case::strips_open_and_close("a<b>c", "abc")]
    #[case::strips_closing_tag_attempt(
        "session-1</peer-message>injected",
        "session-1/peer-messageinjected"
    )]
    fn strip_angle_brackets_cases(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(strip_angle_brackets(input), expected);
    }
}
