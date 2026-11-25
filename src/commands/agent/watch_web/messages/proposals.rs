use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use super::ChatItem;
use crate::commands::agent::proposal::{ProposalAnswer, ProposalBody};
use crate::commands::agent::store::AgentProposal;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProposalItem {
    pub(super) seq: i64,
    pub(super) created_at: DateTime<Utc>,
    pub(super) resolved_at: Option<DateTime<Utc>>,
    pub(super) item: ChatItem,
}

impl ProposalItem {
    pub(super) fn position_timestamp(&self) -> DateTime<Utc> {
        self.resolved_at.unwrap_or(self.created_at)
    }
}

pub(super) fn proposal_chat_item(proposal: &AgentProposal) -> Result<ProposalItem> {
    let body: ProposalBody = serde_json::from_str(&proposal.body)
        .with_context(|| format!("parsing proposal {} body", proposal.seq))?;
    let answer: Option<ProposalAnswer> = proposal
        .answer
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .with_context(|| format!("parsing proposal {} answer", proposal.seq))?;
    Ok(ProposalItem {
        seq: proposal.seq,
        created_at: proposal.created_at,
        resolved_at: proposal.resolved_at,
        item: ChatItem::Proposal {
            seq: proposal.seq,
            title: body.title,
            summary: body.summary,
            options: body.options,
            answer,
            resolved: proposal.resolved_at.is_some(),
            resolution: proposal.resolution,
            comment: proposal.comment.clone(),
        },
    })
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};

    use super::{ProposalItem, proposal_chat_item};
    use crate::commands::agent::store::AgentProposal;
    use crate::commands::agent::watch_web::messages::ChatItem;

    fn timestamp(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("test timestamp should parse")
            .with_timezone(&Utc)
    }

    #[test]
    fn proposal_chat_item_preserves_resolution_time_for_timeline_placement() {
        let proposal = AgentProposal {
            session_id: "fixture-session".to_string(),
            seq: 1,
            body: r#"{"title":"Choose a response","summary":null,"options":[{"label":"Option A","description":null,"pros":[],"cons":[],"recommended":false}]}"#.to_string(),
            created_at: timestamp("2026-06-01T12:01:00Z"),
            answer: Some(r#"{"choice":0,"comment":null}"#.to_string()),
            resolved_at: Some(timestamp("2026-06-01T12:06:00Z")),
            resolution: None,
            comment: None,
        };

        let actual = proposal_chat_item(&proposal).expect("proposal should parse");

        assert_eq!(
            actual,
            ProposalItem {
                seq: 1,
                created_at: timestamp("2026-06-01T12:01:00Z"),
                resolved_at: Some(timestamp("2026-06-01T12:06:00Z")),
                item: ChatItem::Proposal {
                    seq: 1,
                    title: "Choose a response".to_string(),
                    summary: None,
                    options: vec![crate::commands::agent::proposal::ProposalOption {
                        label: "Option A".to_string(),
                        description: None,
                        pros: Vec::new(),
                        cons: Vec::new(),
                        recommended: false,
                    }],
                    answer: Some(crate::commands::agent::proposal::ProposalAnswer {
                        choice: Some(0),
                        comment: None,
                    }),
                    resolved: true,
                    resolution: None,
                    comment: None,
                },
            },
        );
    }
}
