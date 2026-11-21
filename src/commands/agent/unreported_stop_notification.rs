use anyhow::Result;
use clap::Args;

use crate::commands::agent::peer::notify;
#[cfg(not(test))]
use crate::infra::process;

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
