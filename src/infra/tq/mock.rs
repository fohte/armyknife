//! wiremock-based tq mock server for testing.
//!
//! Provides `TqMockServer` for HTTP-level mocking of tq API calls.
//!
//! # Usage
//!
//! ```ignore
//! let mock = TqMockServer::start().await;
//! mock.task_session_links(&[("task-1", "session-1")]).await;
//!
//! let client = mock.client();
//! let links = client.list_task_session_links().await.unwrap();
//! ```

use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::client::TqClient;

/// wiremock-based tq mock server for testing.
pub struct TqMockServer {
    server: MockServer,
}

impl TqMockServer {
    /// Start a new mock server.
    pub async fn start() -> Self {
        Self {
            server: MockServer::start().await,
        }
    }

    /// The mock server's base URL, e.g. for setting `TQ_API_URL` directly.
    pub fn uri(&self) -> String {
        self.server.uri()
    }

    /// Get a TqClient configured to use this mock server, with no Cloudflare
    /// Access headers attached.
    pub fn client(&self) -> TqClient {
        TqClient::with_base_url(self.server.uri())
    }

    /// Mock GET /api/agent-sessions/by-task returning the given task/session
    /// pairs. Each item also carries extra unknown fields to verify the
    /// client ignores them.
    pub async fn task_session_links(&self, links: &[(&str, &str)]) {
        let body: Vec<serde_json::Value> = links
            .iter()
            .map(|(task_id, session_id)| {
                json!({
                    "taskId": task_id,
                    "sessionId": session_id,
                    "context": "some agent context",
                    "cwd": "/some/path",
                    "label": "some-label"
                })
            })
            .collect();

        Mock::given(method("GET"))
            .and(path("/api/agent-sessions/by-task"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!(body)))
            .mount(&self.server)
            .await;
    }

    /// Mock a non-2xx response for GET /api/agent-sessions/by-task.
    pub async fn task_session_links_error(&self, status: u16) {
        Mock::given(method("GET"))
            .and(path("/api/agent-sessions/by-task"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&self.server)
            .await;
    }

    /// Mock GET /api/tasks/{task_id} returning a task with the given
    /// number/title, plus extra unknown fields to verify the client ignores
    /// them.
    pub async fn task(&self, task_id: &str, number: u32, title: &str) {
        Mock::given(method("GET"))
            .and(path(format!("/api/tasks/{task_id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": task_id,
                "number": number,
                "title": title,
                "status": "todo",
                "createdAt": "2024-01-01T00:00:00Z"
            })))
            .mount(&self.server)
            .await;
    }

    /// Mock a non-2xx response for GET /api/tasks/{task_id}.
    pub async fn task_error(&self, task_id: &str, status: u16) {
        Mock::given(method("GET"))
            .and(path(format!("/api/tasks/{task_id}")))
            .respond_with(ResponseTemplate::new(status))
            .mount(&self.server)
            .await;
    }

    /// Mock GET /api/tasks/{task_id}, but only responds when the Cloudflare
    /// Access service-token headers match. Used to prove the client actually
    /// sends those headers when configured.
    pub async fn task_requiring_cf_headers(
        &self,
        task_id: &str,
        number: u32,
        title: &str,
        client_id: &str,
        client_secret: &str,
    ) {
        Mock::given(method("GET"))
            .and(path(format!("/api/tasks/{task_id}")))
            .and(header("CF-Access-Client-Id", client_id))
            .and(header("CF-Access-Client-Secret", client_secret))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": task_id,
                "number": number,
                "title": title
            })))
            .mount(&self.server)
            .await;
    }
}
