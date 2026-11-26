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
    column: Option<u64>,
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

    let functions = format_functions(&report.functions);
    tracing::warn!(
        event = "agent.watch_web.long_task",
        started_at_ms = report.started_at_ms,
        duration_ms = report.duration_ms,
        top_functions = %functions,
        "watch-web detected a long JavaScript task"
    );

    Ok(StatusCode::NO_CONTENT)
}

fn format_functions(functions: &[FunctionSample]) -> String {
    functions
        .iter()
        .map(|function| {
            let line = function
                .line
                .map_or_else(|| "?".to_owned(), |line| line.to_string());
            let column = function
                .column
                .map_or_else(String::new, |column| format!(":{column}"));
            format!(
                "{} ({}:{}{}, {:.1}%)",
                function.name, function.file, line, column, function.percentage
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
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
                && function.column.is_none_or(|column| column > 0)
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
    use rstest::rstest;
    use tower::ServiceExt;

    use super::{FunctionSample, format_functions, router};

    #[test]
    fn formats_function_locations_and_percentages() {
        assert_eq!(
            format_functions(&[
                FunctionSample {
                    name: "callback".to_owned(),
                    file: "/assets/app.js".to_owned(),
                    line: Some(12),
                    column: Some(8),
                    percentage: 66.7,
                },
                FunctionSample {
                    name: "renderList".to_owned(),
                    file: "/assets/app.js".to_owned(),
                    line: Some(42),
                    column: None,
                    percentage: 33.3,
                },
            ]),
            "callback (/assets/app.js:12:8, 66.7%), renderList (/assets/app.js:42, 33.3%)"
        );
    }

    #[rstest]
    #[case::without_column(
        r#"{"startedAtMs":1795737600000,"durationMs":8200,"functions":[{"name":"renderList","file":"/assets/app.js","line":42,"percentage":75.0}]}"#.to_owned()
    )]
    #[case::with_column(
        r#"{"startedAtMs":1795737600000,"durationMs":8200,"functions":[{"name":"renderList","file":"/assets/app.js","line":42,"column":8,"percentage":75.0}]}"#.to_owned()
    )]
    #[tokio::test]
    async fn records_valid_long_task_reports(#[case] body: String) {
        let response = router()
            .oneshot(
                Request::post("/long-tasks")
                    .header(CONTENT_TYPE, "application/json")
                    .header(ORIGIN, HeaderValue::from_static("http://localhost"))
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[rstest]
    #[case::duration_below_threshold(
        r#"{"startedAtMs":1795737600000,"durationMs":999,"functions":[]}"#.to_owned()
    )]
    #[case::percentage_above_maximum(
        r#"{"startedAtMs":1795737600000,"durationMs":1000,"functions":[{"name":"renderList","file":"/assets/app.js","line":42,"percentage":101.0}]}"#.to_owned()
    )]
    #[case::zero_column(
        r#"{"startedAtMs":1795737600000,"durationMs":1000,"functions":[{"name":"renderList","file":"/assets/app.js","line":42,"column":0,"percentage":100.0}]}"#.to_owned()
    )]
    #[tokio::test]
    async fn rejects_invalid_long_task_reports(#[case] body: String) {
        let response = router()
            .oneshot(
                Request::post("/long-tasks")
                    .header(CONTENT_TYPE, "application/json")
                    .header(ORIGIN, HeaderValue::from_static("http://localhost"))
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
