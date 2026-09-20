/// How an agent process received its initial prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentLaunchRoute {
    /// No initial prompt was provided.
    Standard,
    /// Claude received its initial prompt through its messaging socket.
    ClaudeMessaging,
    /// Claude was launched with the prompt in argv because messaging failed.
    ClaudeArgvFallback { reason: String },
    /// Codex attached to the shared app-server and received its first turn by RPC.
    CodexDaemon { thread_id: String },
    /// Codex was launched with the prompt in argv because the daemon path failed.
    CodexArgvFallback { reason: String },
}

impl AgentLaunchRoute {
    pub fn display_suffix(&self) -> String {
        match self {
            Self::Standard => String::new(),
            Self::ClaudeMessaging => " (Claude messaging)".to_string(),
            Self::ClaudeArgvFallback { reason } => {
                format!(" (Claude argv fallback: {reason})")
            }
            Self::CodexDaemon { .. } => " (Codex daemon)".to_string(),
            Self::CodexArgvFallback { reason } => {
                format!(" (Codex argv fallback: {reason})")
            }
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
    #[case::claude_fallback(
        AgentLaunchRoute::ClaudeArgvFallback {
            reason: "socket unavailable".to_string(),
        },
        " (Claude argv fallback: socket unavailable)"
    )]
    #[case::codex_daemon(
        AgentLaunchRoute::CodexDaemon {
            thread_id: "thread-a".to_string(),
        },
        " (Codex daemon)"
    )]
    #[case::codex_fallback(
        AgentLaunchRoute::CodexArgvFallback {
            reason: "daemon unavailable".to_string(),
        },
        " (Codex argv fallback: daemon unavailable)"
    )]
    fn display_suffix(#[case] route: AgentLaunchRoute, #[case] expected: &str) {
        assert_eq!(route.display_suffix(), expected);
    }
}
