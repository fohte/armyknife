use super::{EditorConfig, Terminal};

impl EditorConfig {
    /// Builds the shell command that brings the configured terminal app to the foreground.
    pub(crate) fn focus_app_command(&self) -> String {
        build_focus_app_command(
            &self.terminal,
            self.focus_app.as_deref(),
            cfg!(target_os = "macos"),
        )
    }
}

fn build_focus_app_command(terminal: &Terminal, focus_app: Option<&str>, is_macos: bool) -> String {
    const GHOSTTY_DEFAULT_TITLE: &str = "👻";

    // Ghostty exposes no per-window tty metadata, so its default title selects the main window.
    if is_macos && *terminal == Terminal::Ghostty && focus_app.is_none() {
        format!(
            "osascript -e 'tell application \"Ghostty\"' -e 'activate (first window whose name is \"{GHOSTTY_DEFAULT_TITLE}\")' -e 'activate' -e 'end tell'"
        )
    } else {
        let focus_app = focus_app.unwrap_or_else(|| terminal.default_focus_app());
        let focus_app =
            shlex::try_quote(focus_app).unwrap_or_else(|_| focus_app.to_string().into());
        format!("open -a {focus_app}")
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::ghostty_macos_default_app(
        None,
        true,
        "osascript -e 'tell application \"Ghostty\"' -e 'activate (first window whose name is \"👻\")' -e 'activate' -e 'end tell'"
    )]
    #[case::ghostty_custom_app(None, false, "open -a Ghostty")]
    #[case::explicit_app_overrides_macos_default(
        Some("Custom Terminal"),
        true,
        "open -a 'Custom Terminal'"
    )]
    fn builds_focus_command(
        #[case] focus_app: Option<&str>,
        #[case] is_macos: bool,
        #[case] expected: &str,
    ) {
        assert_eq!(
            build_focus_app_command(&Terminal::Ghostty, focus_app, is_macos),
            expected,
        );
    }
}
