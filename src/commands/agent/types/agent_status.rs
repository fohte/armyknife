use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatusKind {
    Decide,
    Read,
    Do,
    Idle,
    Close,
    Done,
    Wait,
}

#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStatus {
    pub kind: AgentStatusKind,
    pub note: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::decide(AgentStatusKind::Decide, "decide")]
    #[case::read(AgentStatusKind::Read, "read")]
    #[case::do_work(AgentStatusKind::Do, "do")]
    #[case::idle(AgentStatusKind::Idle, "idle")]
    #[case::close(AgentStatusKind::Close, "close")]
    #[case::done(AgentStatusKind::Done, "done")]
    #[case::wait(AgentStatusKind::Wait, "wait")]
    fn kind_names_match_cli_and_json(#[case] kind: AgentStatusKind, #[case] expected: &str) {
        assert_eq!(
            (
                kind.to_possible_value()
                    .map(|value| value.get_name().to_string()),
                serde_json::to_value(kind).unwrap(),
            ),
            (Some(expected.to_string()), serde_json::json!(expected)),
        );
    }
}
