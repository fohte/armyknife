use serde::{Deserialize, Serialize};

use super::AgentConfig;

/// Meaning assigned to a configured agent status name.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatusState {
    User,
    Waiting,
    Done,
}

/// Call kind shown in watch-web for a configured agent status.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatusWebKind {
    Question,
    Review,
    #[default]
    Stopped,
}

/// Configuration for one user-defined agent status name.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStatusConfig {
    pub state: AgentStatusState,

    /// Call kind shown in watch-web. Omitted values default to `stopped`.
    #[serde(default)]
    pub watch_web_kind: AgentStatusWebKind,
}

impl AgentConfig {
    /// Resolve a persisted status name, treating names removed from config as user turns.
    pub fn status_state(&self, kind: &str) -> AgentStatusState {
        self.statuses
            .get(kind)
            .map_or(AgentStatusState::User, |status| status.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;

    #[test]
    fn configured_names_map_to_states_and_unknown_names_are_user_turns() {
        let config: AgentConfig = serde_yaml::from_str(indoc! {
            "\
            statuses:
              needs_attention: { state: user, watch_web_kind: question }
              external_wait: { state: waiting }
              work_finished: { state: done, watch_web_kind: review }
            "
        })
        .expect("agent status configuration should parse");

        assert_eq!(
            [
                (
                    config.status_state("needs_attention"),
                    config.statuses["needs_attention"].watch_web_kind,
                ),
                (
                    config.status_state("external_wait"),
                    config.statuses["external_wait"].watch_web_kind,
                ),
                (
                    config.status_state("work_finished"),
                    config.statuses["work_finished"].watch_web_kind,
                ),
                (
                    config.status_state("removed_name"),
                    AgentStatusWebKind::Stopped,
                ),
            ],
            [
                (AgentStatusState::User, AgentStatusWebKind::Question),
                (AgentStatusState::Waiting, AgentStatusWebKind::Stopped),
                (AgentStatusState::Done, AgentStatusWebKind::Review),
                (AgentStatusState::User, AgentStatusWebKind::Stopped),
            ]
        );
    }
}
