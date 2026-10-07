/// Escapes a value that tmux will expand as a format string and removes
/// controls that cannot be displayed safely in a pane title.
pub(crate) fn escape_format_value(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>()
        .replace('#', "##")
}
