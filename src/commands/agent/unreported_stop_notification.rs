use anyhow::Result;
use clap::Args;

use crate::commands::agent::peer::notify;
#[cfg(not(test))]
use crate::infra::process;
use crate::shared::sanitize::strip_angle_brackets;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct UnreportedStopNotificationArgs {
    #[arg(long)]
    pub session_id: String,
    #[arg(long)]
    pub recipient_session_id: String,
    #[arg(long, allow_hyphen_values = true)]
    pub message: String,
}

#[cfg(not(test))]
pub(super) fn spawn(session_id: &str, recipient_session_id: &str, message: &str) {
    process::spawn_self_detached(
        "agent.hook.unreported_stop.spawn",
        "agent.hook.unreported_stop.spawn_failed",
        session_id,
        &[
            "agent",
            "unreported-stop-notification",
            "--session-id",
            session_id,
            "--recipient-session-id",
            recipient_session_id,
            "--message",
            message,
        ],
    );
}

#[cfg(test)]
pub(super) fn spawn(_session_id: &str, _recipient_session_id: &str, _message: &str) {}

pub(super) fn render(template: &str, session_id: &str, last_message: &str) -> String {
    template
        .replace("{session_id}", &strip_angle_brackets(session_id))
        .replace("{last_message}", &strip_angle_brackets(last_message))
}

pub fn run(args: &UnreportedStopNotificationArgs) -> Result<()> {
    if let Err(error) = notify::notify(&args.recipient_session_id, &args.message, None, None) {
        tracing::warn!(
            event = "agent.hook.unreported_stop.delivery_failed",
            session = %args.session_id,
            recipient_session = %args.recipient_session_id,
            error = %error,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::commands::agent::AgentCommands;

    #[test]
    fn parses_dash_prefixed_notification_message() {
        let cli = crate::cli::Cli::try_parse_from([
            "a",
            "agent",
            "unreported-stop-notification",
            "--session-id",
            "example-session",
            "--recipient-session-id",
            "example-parent",
            "--message",
            "- review needed",
        ])
        .expect("notification arguments should parse");

        let crate::cli::Commands::Agent(AgentCommands::UnreportedStopNotification(args)) =
            cli.command
        else {
            panic!("CLI should parse an unreported stop notification");
        };

        assert_eq!(
            (args.session_id, args.recipient_session_id, args.message,),
            (
                "example-session".to_string(),
                "example-parent".to_string(),
                "- review needed".to_string(),
            ),
        );
    }
}
