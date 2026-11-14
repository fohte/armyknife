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

/// Configuration for one user-defined agent status name.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentStatusConfig {
    pub state: AgentStatusState,
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
              needs_attention: { state: user }
              external_wait: { state: waiting }
              work_finished: { state: done }
            "
        })
        .expect("agent status configuration should parse");

        assert_eq!(
            [
                config.status_state("needs_attention"),
                config.status_state("external_wait"),
                config.status_state("work_finished"),
                config.status_state("removed_name"),
            ],
            [
                AgentStatusState::User,
                AgentStatusState::Waiting,
                AgentStatusState::Done,
                AgentStatusState::User,
            ]
        );
    }
}
