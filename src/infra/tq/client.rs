//! tq API client implementation using reqwest.
//!
//! tq (<https://tq.fohte.net>) is the user's personal task manager and sits
//! behind Cloudflare Access, so `from_env` optionally attaches Cloudflare
//! Access service-token headers on top of the base HTTP client.

use serde::Deserialize;

use super::error::{Result, TqError};

/// Header names for Cloudflare Access service tokens.
/// See <https://developers.cloudflare.com/cloudflare-one/identity/service-tokens/>.
const CF_ACCESS_CLIENT_ID_HEADER: &str = "CF-Access-Client-Id";
const CF_ACCESS_CLIENT_SECRET_HEADER: &str = "CF-Access-Client-Secret";

/// tq API client using reqwest.
pub struct TqClient {
    http: reqwest::Client,
    base_url: String,
}

/// A tq task with an associated agent session, as returned by
/// `GET /api/agent-sessions/by-task`.
#[derive(Debug, PartialEq, Eq)]
pub struct TaskSessionLink {
    pub task_id: String,
    pub session_id: String,
}

/// Wire format for one item of `GET /api/agent-sessions/by-task`.
/// The response carries many more fields than this; unknown fields are ignored.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskSessionLinkResponse {
    task_id: String,
    session_id: String,
}

/// A tq task, as returned by `GET /api/tasks/{task_id}`.
#[derive(Debug, PartialEq, Eq)]
pub struct TqTask {
    pub number: u32,
    pub title: String,
}

/// Wire format for `GET /api/tasks/{task_id}`.
/// The response carries many more fields than this; unknown fields are ignored.
#[derive(Debug, Deserialize)]
struct TaskResponse {
    number: u32,
    title: String,
}

impl TqClient {
    /// Build a client from environment configuration.
    ///
    /// Returns `None` when `TQ_API_URL` is unset or empty, which means the tq
    /// integration is simply not configured -- callers should skip the feature
    /// silently rather than treat this as an error.
    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("TQ_API_URL").ok().filter(|v| !v.is_empty())?;

        let client_id = std::env::var("CF_ACCESS_CLIENT_ID")
            .ok()
            .filter(|v| !v.is_empty());
        let client_secret = std::env::var("CF_ACCESS_CLIENT_SECRET")
            .ok()
            .filter(|v| !v.is_empty());

        // Only attach Cloudflare Access headers if both are present; if building
        // the client with them fails for any reason, fall back to a plain client
        // rather than erroring -- requests will simply fail downstream (302 from
        // Cloudflare), which callers already treat as "tq unreachable".
        let http = match (client_id, client_secret) {
            (Some(id), Some(secret)) => cf_access_client(&id, &secret).unwrap_or_default(),
            _ => reqwest::Client::new(),
        };

        Some(Self { http, base_url })
    }

    /// Test-only constructor pointing at an arbitrary base URL (e.g. a wiremock
    /// server), with no Cloudflare Access headers attached.
    #[cfg(test)]
    pub(crate) fn with_base_url(base_url: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
        }
    }

    fn url(&self, route: &str) -> String {
        format!("{}{route}", self.base_url)
    }

    /// List all tq tasks that currently have an associated agent session.
    pub async fn list_task_session_links(&self) -> Result<Vec<TaskSessionLink>> {
        let response = self
            .http
            .get(self.url("/api/agent-sessions/by-task"))
            .send()
            .await?;
        let links: Vec<TaskSessionLinkResponse> = check_response(response).await?;

        Ok(links
            .into_iter()
            .map(|l| TaskSessionLink {
                task_id: l.task_id,
                session_id: l.session_id,
            })
            .collect())
    }

    /// Get a tq task by its UUID.
    pub async fn get_task(&self, task_id: &str) -> Result<TqTask> {
        let route = format!("/api/tasks/{task_id}");
        let response = self.http.get(self.url(&route)).send().await?;
        let task: TaskResponse = check_response(response).await?;

        Ok(TqTask {
            number: task.number,
            title: task.title,
        })
    }
}

/// Build a reqwest client with Cloudflare Access service-token headers attached.
/// Returns `None` if the header values are invalid; the caller falls back to a
/// plain client in that case.
fn cf_access_client(client_id: &str, client_secret: &str) -> Option<reqwest::Client> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        CF_ACCESS_CLIENT_ID_HEADER,
        reqwest::header::HeaderValue::from_str(client_id).ok()?,
    );
    headers.insert(
        CF_ACCESS_CLIENT_SECRET_HEADER,
        reqwest::header::HeaderValue::from_str(client_secret).ok()?,
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .ok()
}

/// Check HTTP response status and deserialize the JSON body, or return an error.
///
/// tq sits behind Cloudflare Access, so an unauthenticated request comes back
/// as a 302 redirect rather than 401/403; any non-2xx status is treated the
/// same way here.
async fn check_response<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let status = response.status();
    if !status.is_success() {
        return Err(TqError::ApiError(status.as_u16()).into());
    }
    let body = response.json().await?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::infra::tq::mock::TqMockServer;

    #[rstest]
    #[tokio::test]
    async fn test_list_task_session_links_ignores_unknown_fields() {
        let mock = TqMockServer::start().await;
        mock.task_session_links(&[("task-1", "session-1"), ("task-2", "session-2")])
            .await;

        let client = mock.client();
        let result = client.list_task_session_links().await.unwrap();

        assert_eq!(
            result,
            vec![
                TaskSessionLink {
                    task_id: "task-1".to_string(),
                    session_id: "session-1".to_string(),
                },
                TaskSessionLink {
                    task_id: "task-2".to_string(),
                    session_id: "session-2".to_string(),
                },
            ]
        );
    }

    #[rstest]
    #[tokio::test]
    async fn test_list_task_session_links_error() {
        let mock = TqMockServer::start().await;
        mock.task_session_links_error(401).await;

        let client = mock.client();
        let result = client.list_task_session_links().await;

        assert!(result.is_err());
    }

    #[rstest]
    #[tokio::test]
    async fn test_get_task_ignores_unknown_fields() {
        let mock = TqMockServer::start().await;
        mock.task("task-1", 42, "Fix the bug").await;

        let client = mock.client();
        let result = client.get_task("task-1").await.unwrap();

        assert_eq!(
            result,
            TqTask {
                number: 42,
                title: "Fix the bug".to_string(),
            }
        );
    }

    #[rstest]
    #[case::unauthorized(401)]
    #[case::cloudflare_access_redirect(302)]
    #[tokio::test]
    async fn test_get_task_error(#[case] status: u16) {
        let mock = TqMockServer::start().await;
        mock.task_error("task-1", status).await;

        let client = mock.client();
        let result = client.get_task("task-1").await;

        assert!(result.is_err());
    }

    #[test]
    fn test_from_env_returns_none_when_url_unset() {
        temp_env::with_vars(
            [
                ("TQ_API_URL", None::<&str>),
                ("CF_ACCESS_CLIENT_ID", None::<&str>),
                ("CF_ACCESS_CLIENT_SECRET", None::<&str>),
            ],
            || {
                assert!(TqClient::from_env().is_none());
            },
        );
    }

    #[rstest]
    #[tokio::test]
    async fn test_from_env_attaches_cf_access_headers() {
        let mock = TqMockServer::start().await;
        mock.task_requiring_cf_headers("task-1", 1, "Task", "test-id", "test-secret")
            .await;

        let client = temp_env::with_vars(
            [
                ("TQ_API_URL", Some(mock.uri().as_str())),
                ("CF_ACCESS_CLIENT_ID", Some("test-id")),
                ("CF_ACCESS_CLIENT_SECRET", Some("test-secret")),
            ],
            TqClient::from_env,
        );

        let result = client.unwrap().get_task("task-1").await.unwrap();

        assert_eq!(
            result,
            TqTask {
                number: 1,
                title: "Task".to_string(),
            }
        );
    }

    #[rstest]
    #[tokio::test]
    async fn test_from_env_without_cf_headers_still_builds_working_client() {
        let mock = TqMockServer::start().await;
        mock.task("task-1", 1, "Task").await;

        let client = temp_env::with_vars(
            [
                ("TQ_API_URL", Some(mock.uri().as_str())),
                ("CF_ACCESS_CLIENT_ID", None),
                ("CF_ACCESS_CLIENT_SECRET", None),
            ],
            TqClient::from_env,
        );

        let result = client.unwrap().get_task("task-1").await.unwrap();

        assert_eq!(
            result,
            TqTask {
                number: 1,
                title: "Task".to_string(),
            }
        );
    }
}
