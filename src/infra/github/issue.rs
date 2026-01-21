//! Issue operations.

use super::client::OctocrabClient;
use super::error::Result;

/// Trait for issue operations.
#[allow(dead_code)]
#[async_trait::async_trait]
pub trait IssueClient: Send + Sync {
    /// Get an issue by number.
    async fn get_issue(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<crate::commands::gh::issue_agent::models::Issue>;

    /// Update an issue's body.
    async fn update_issue_body(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        body: &str,
    ) -> Result<()>;

    /// Update an issue's title.
    async fn update_issue_title(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        title: &str,
    ) -> Result<()>;

    /// Add labels to an issue.
    async fn add_labels(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        labels: &[String],
    ) -> Result<()>;

    /// Remove a label from an issue.
    async fn remove_label(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        label: &str,
    ) -> Result<()>;
}

#[async_trait::async_trait]
impl IssueClient for OctocrabClient {
    async fn get_issue(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
    ) -> Result<crate::commands::gh::issue_agent::models::Issue> {
        let issue = self.client.issues(owner, repo).get(issue_number).await?;

        // Convert octocrab::models::issues::Issue to our Issue model
        let state = match issue.state {
            octocrab::models::IssueState::Open => "OPEN".to_string(),
            octocrab::models::IssueState::Closed => "CLOSED".to_string(),
            // IssueState is #[non_exhaustive], so handle future variants
            _ => format!("{:?}", issue.state).to_uppercase(),
        };
        Ok(crate::commands::gh::issue_agent::models::Issue {
            number: issue.number as i64,
            title: issue.title,
            body: issue.body,
            state,
            labels: issue
                .labels
                .into_iter()
                .map(|l| crate::commands::gh::issue_agent::models::Label { name: l.name })
                .collect(),
            assignees: issue
                .assignees
                .into_iter()
                .map(|a| crate::commands::gh::issue_agent::models::Author { login: a.login })
                .collect(),
            milestone: issue
                .milestone
                .map(|m| crate::commands::gh::issue_agent::models::Milestone { title: m.title }),
            author: Some(crate::commands::gh::issue_agent::models::Author {
                login: issue.user.login,
            }),
            created_at: issue.created_at,
            updated_at: issue.updated_at,
        })
    }

    async fn update_issue_body(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        body: &str,
    ) -> Result<()> {
        self.client
            .issues(owner, repo)
            .update(issue_number)
            .body(body)
            .send()
            .await?;
        Ok(())
    }

    async fn update_issue_title(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        title: &str,
    ) -> Result<()> {
        self.client
            .issues(owner, repo)
            .update(issue_number)
            .title(title)
            .send()
            .await?;
        Ok(())
    }

    async fn add_labels(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        labels: &[String],
    ) -> Result<()> {
        self.client
            .issues(owner, repo)
            .add_labels(issue_number, labels)
            .await?;
        Ok(())
    }

    async fn remove_label(
        &self,
        owner: &str,
        repo: &str,
        issue_number: u64,
        label: &str,
    ) -> Result<()> {
        self.client
            .issues(owner, repo)
            .remove_label(issue_number, label)
            .await?;
        Ok(())
    }
}
