/// How an agent process received its initial prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentLaunchRoute {
    /// No initial prompt was provided.
    Standard,
    /// Claude received its initial prompt through its messaging socket.
    ClaudeMessaging,
    /// Codex attached to the shared app-server and received its first turn by RPC.
    CodexDaemon { thread_id: String },
    /// The initial Codex turn/start request was sent, but its response was not confirmed.
    CodexDaemonTurnUnconfirmed { thread_id: String, reason: String },
}

impl AgentLaunchRoute {
    pub fn display_suffix(&self) -> String {
        match self {
            Self::Standard => String::new(),
            Self::ClaudeMessaging => " (Claude messaging)".to_string(),
            Self::CodexDaemon { .. } => " (Codex daemon)".to_string(),
            Self::CodexDaemonTurnUnconfirmed { reason, .. } => format!(
                " (Codex daemon; warning: initial turn/start response was not confirmed: {reason})"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::standard(AgentLaunchRoute::Standard, "")]
    #[case::claude_messaging(AgentLaunchRoute::ClaudeMessaging, " (Claude messaging)")]
    #[case::codex_daemon(
        AgentLaunchRoute::CodexDaemon {
            thread_id: "thread-a".to_string(),
        },
        " (Codex daemon)"
    )]
    #[case::codex_daemon_turn_unconfirmed(
        AgentLaunchRoute::CodexDaemonTurnUnconfirmed {
            thread_id: "thread-example".to_string(),
            reason: "response unavailable".to_string(),
        },
        " (Codex daemon; warning: initial turn/start response was not confirmed: response unavailable)"
    )]
    fn display_suffix(#[case] route: AgentLaunchRoute, #[case] expected: &str) {
        assert_eq!(route.display_suffix(), expected);
    }
}
