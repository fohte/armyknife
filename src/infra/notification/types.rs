/// A notification to be displayed to the user.
#[derive(Debug, Clone)]
pub struct Notification {
    title: String,
    subtitle: Option<String>,
    message: String,
    sound: Option<String>,
    action: Option<NotificationAction>,
}

impl Notification {
    /// Creates a new notification with the given title and message.
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
            message: message.into(),
            sound: None,
            action: None,
        }
    }

    /// Sets the subtitle (displayed below the title).
    pub fn with_subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Sets the sound to play when the notification is displayed.
    pub fn with_sound(mut self, sound: impl Into<String>) -> Self {
        self.sound = Some(sound.into());
        self
    }

    /// Sets the action to execute when the notification is clicked.
    pub fn with_action(mut self, action: NotificationAction) -> Self {
        self.action = Some(action);
        self
    }

    /// Returns the notification title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// Returns the notification subtitle, if any.
    pub fn subtitle(&self) -> Option<&str> {
        self.subtitle.as_deref()
    }

    /// Returns the notification message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the sound to play, if any.
    pub fn sound(&self) -> Option<&str> {
        self.sound.as_deref()
    }

    /// Returns the action to execute on click, if any.
    pub fn action(&self) -> Option<&NotificationAction> {
        self.action.as_ref()
    }
}

/// An action to execute when a notification is clicked.
#[derive(Debug, Clone)]
pub struct NotificationAction {
    command: String,
}

impl NotificationAction {
    /// Creates a new action with the given shell command.
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
        }
    }

    /// Returns the command to execute.
    pub fn command(&self) -> &str {
        &self.command
    }
}
