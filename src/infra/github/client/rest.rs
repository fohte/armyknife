use super::super::error::{Result, api_error_from_response};
use super::GitHubClient;

pub(super) async fn ensure_success(response: reqwest::Response) -> Result<reqwest::Response> {
    let status = response.status();
    if !status.is_success() {
        let body: serde_json::Value = response
            .json()
            .await
            .unwrap_or_else(|_| serde_json::json!({"message": "Unknown error"}));
        return Err(api_error_from_response(status.as_u16(), &body).into());
    }
    Ok(response)
}

impl GitHubClient {
    pub(crate) async fn rest_post_no_content(&self, route: &str) -> Result<()> {
        let response = self.http.post(self.url(route)).send().await?;
        ensure_success(response).await?;
        Ok(())
    }
}
