//! Liveness and readiness probes for container orchestrators. `/healthz`
//! answers as soon as the listener is up; `/readyz` reflects the health
//! registry so a pod with a failed component is pulled from the service.

use arkavo_observability::health_reporter::{HealthRegistry, HealthStatus};
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::json;

pub async fn healthz_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

pub async fn readyz_handler() -> impl IntoResponse {
    let status = HealthRegistry::global().get_overall_status().await;
    let code = match status {
        HealthStatus::Healthy | HealthStatus::Degraded => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    (code, Json(json!({ "status": format!("{status:?}") })))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    #[tokio::test]
    async fn healthz_is_always_ok() {
        let response = healthz_handler().await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn readyz_reports_registry_status() {
        let response = readyz_handler().await.into_response();
        let status = response.status();
        assert!(
            status == axum::http::StatusCode::OK
                || status == axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "unexpected status {status}"
        );
    }
}
