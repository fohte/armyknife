use super::{EditorConfig, Terminal};

impl EditorConfig {
    /// Builds the shell command that brings the configured terminal app to the foreground.
    pub(crate) fn focus_app_command(&self) -> String {
        build_focus_app_command(
            &self.terminal,
            self.focus_app.is_some(),
            self.focus_app(),
            cfg!(target_os = "macos"),
        )
    }
}

fn build_focus_app_command(
    terminal: &Terminal,
    has_custom_focus_app: bool,
    focus_app: &str,
    is_macos: bool,
) -> String {
    const GHOSTTY_DEFAULT_TITLE: &str = "👻";

    // Ghostty exposes no per-window tty metadata, so its default title selects the main window.
    if is_macos && *terminal == Terminal::Ghostty && !has_custom_focus_app {
        format!(
            "osascript -e 'tell application \"Ghostty\"' -e 'activate (first window whose name is \"{GHOSTTY_DEFAULT_TITLE}\")' -e 'activate' -e 'end tell'"
        )
    } else {
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
        false,
        "Ghostty",
        true,
        "osascript -e 'tell application \"Ghostty\"' -e 'activate (first window whose name is \"👻\")' -e 'activate' -e 'end tell'"
    )]
    #[case::ghostty_non_macos_default_app(false, "Ghostty", false, "open -a Ghostty")]
    #[case::explicit_app_overrides_macos_default(
        true,
        "Custom Terminal",
        true,
        "open -a 'Custom Terminal'"
    )]
    fn builds_focus_command(
        #[case] has_custom_focus_app: bool,
        #[case] focus_app: &str,
        #[case] is_macos: bool,
        #[case] expected: &str,
    ) {
        assert_eq!(
            build_focus_app_command(
                &Terminal::Ghostty,
                has_custom_focus_app,
                focus_app,
                is_macos,
            ),
            expected,
        );
    }
}
