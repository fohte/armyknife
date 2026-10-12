use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStatus {
    pub kind: String,
    pub note: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_status_names_survive_session_serialization() {
        let status = AgentStatus {
            kind: "needs_attention".to_string(),
            note: "review the proposal".to_string(),
        };

        assert_eq!(
            serde_json::from_value::<AgentStatus>(
                serde_json::to_value(&status).expect("status should serialize")
            )
            .expect("status should deserialize"),
            status,
        );
    }
}
