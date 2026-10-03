use super::{EditorConfig, Terminal};

impl EditorConfig {
    /// Builds the shell command that brings the configured terminal app to the foreground.
    pub(crate) fn focus_app_command(&self) -> String {
        const GHOSTTY_DEFAULT_TITLE: &str = "👻";

        // Ghostty exposes no per-window tty metadata, so its default title selects the main window.
        if cfg!(target_os = "macos")
            && self.terminal == Terminal::Ghostty
            && self.focus_app.is_none()
        {
            format!(
                "osascript -e 'tell application \"Ghostty\"' -e 'activate (first window whose name is \"{GHOSTTY_DEFAULT_TITLE}\")' -e 'activate' -e 'end tell'"
            )
        } else {
            let focus_app = self.focus_app();
            let focus_app =
                shlex::try_quote(focus_app).unwrap_or_else(|_| focus_app.to_string().into());
            format!("open -a {focus_app}")
        }
    }
}
