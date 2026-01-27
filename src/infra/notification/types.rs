/// A notification to be displayed to the user.
#[derive(Debug, Clone)]
pub struct Notification {
    title: String,
    message: String,
    action: Option<NotificationAction>,
}

impl Notification {
    /// Creates a new notification with the given title and message.
    pub fn new(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            message: message.into(),
            action: None,
        }
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

    /// Returns the notification message.
    pub fn message(&self) -> &str {
        &self.message
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
