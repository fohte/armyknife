use axum::Json;
use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::routing::post;
use serde::Deserialize;

const MAX_FUNCTIONS: usize = 10;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LongTaskReport {
    started_at_ms: i64,
    duration_ms: f64,
    functions: Vec<FunctionSample>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FunctionSample {
    name: String,
    file: String,
    line: Option<u64>,
    percentage: f64,
}

pub(super) fn router() -> Router {
    Router::new()
        .route("/long-tasks", post(record_long_task))
        .layer(DefaultBodyLimit::max(32 * 1024))
}

async fn record_long_task(Json(report): Json<LongTaskReport>) -> Result<StatusCode, StatusCode> {
    if !valid_report(&report) {
        return Err(StatusCode::BAD_REQUEST);
    }

    let functions = report
        .functions
        .iter()
        .map(|function| {
            let line = function
                .line
                .map_or_else(|| "?".to_owned(), |line| line.to_string());
            format!(
                "{} ({}:{}, {:.1}%)",
                function.name, function.file, line, function.percentage
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    tracing::warn!(
        event = "agent.watch_web.long_task",
        started_at_ms = report.started_at_ms,
        duration_ms = report.duration_ms,
        top_functions = %functions,
        "watch-web detected a long JavaScript task"
    );

    Ok(StatusCode::NO_CONTENT)
}

fn valid_report(report: &LongTaskReport) -> bool {
    report.duration_ms.is_finite()
        && report.duration_ms >= 1_000.0
        && report.functions.len() <= MAX_FUNCTIONS
        && report.functions.iter().all(|function| {
            !function.name.is_empty()
                && function.name.len() <= 256
                && function.file.len() <= 2_048
                && function.line.is_none_or(|line| line > 0)
                && function.percentage.is_finite()
                && (0.0..=100.0).contains(&function.percentage)
        })
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use axum::http::header::{CONTENT_TYPE, ORIGIN};
    use axum::http::{HeaderValue, StatusCode};
    use tower::ServiceExt;

    use super::router;

    #[tokio::test]
    async fn records_valid_long_task_reports() {
        let response = router()
            .oneshot(
                Request::post("/long-tasks")
                    .header(CONTENT_TYPE, "application/json")
                    .header(ORIGIN, HeaderValue::from_static("http://localhost"))
                    .body(Body::from(
                        r#"{"startedAtMs":1795737600000,"durationMs":8200,"functions":[{"name":"renderList","file":"/assets/app.js","line":42,"percentage":75.0}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn rejects_reports_below_the_freeze_threshold() {
        let response = router()
            .oneshot(
                Request::post("/long-tasks")
                    .header(CONTENT_TYPE, "application/json")
                    .header(ORIGIN, HeaderValue::from_static("http://localhost"))
                    .body(Body::from(
                        r#"{"startedAtMs":1795737600000,"durationMs":999,"functions":[]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
