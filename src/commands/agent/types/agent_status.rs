use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatusKind {
    #[serde(alias = "decide", alias = "read", alias = "do", alias = "idle")]
    #[value(alias("decide"), alias("read"), alias("do"), alias("idle"))]
    You,
    #[serde(alias = "close")]
    #[value(alias("close"))]
    Done,
    Wait,
}

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
    #[case::you(AgentStatusKind::You, "you")]
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

    #[rstest]
    #[case::decide("decide", AgentStatusKind::You, "you")]
    #[case::read("read", AgentStatusKind::You, "you")]
    #[case::do_work("do", AgentStatusKind::You, "you")]
    #[case::idle("idle", AgentStatusKind::You, "you")]
    #[case::close("close", AgentStatusKind::Done, "done")]
    fn legacy_kind_names_deserialize_to_canonical_values(
        #[case] input: &str,
        #[case] expected: AgentStatusKind,
        #[case] canonical: &str,
    ) {
        let kind: AgentStatusKind = serde_json::from_value(serde_json::json!(input))
            .expect("legacy kind should deserialize");

        assert_eq!(
            (kind, serde_json::to_value(kind).unwrap()),
            (expected, serde_json::json!(canonical))
        );
    }
}
