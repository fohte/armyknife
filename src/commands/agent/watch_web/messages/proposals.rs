use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use super::ChatItem;
use crate::commands::agent::proposal::{ProposalAnswer, ProposalBody};
use crate::commands::agent::store::AgentProposal;

pub(super) struct ProposalItem {
    pub(super) seq: i64,
    pub(super) created_at: DateTime<Utc>,
    pub(super) item: ChatItem,
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
        item: ChatItem::Proposal {
            seq: proposal.seq,
            title: body.title,
            summary: body.summary,
            options: body.options,
            answer,
            resolved: proposal.resolved_at.is_some(),
        },
    })
}
