use axum::response::{Html, IntoResponse, Response};
use axum::{body::Body, extract::Path as AxumPath, http::header};

pub async fn index_handler() -> Html<&'static str> {
    Html(include_str!("../static/index.html"))
}

pub async fn static_file_handler(AxumPath(path): AxumPath<String>) -> Response {
    let path = path.strip_prefix('/').unwrap_or(&path);
    match path {
        "toolbar.js" => serve_js(include_str!("../static/toolbar.js")),
        "styles/base.css" => serve_css(include_str!("../static/styles/base.css")),
        "styles/panels.css" => serve_css(include_str!("../static/styles/panels.css")),
        "styles/dashboard.css" => serve_css(include_str!("../static/styles/dashboard.css")),
        "styles/operations.css" => serve_css(include_str!("../static/styles/operations.css")),
        "js/icons.js" => serve_js(include_str!("../static/js/icons.js")),
        "js/lexicon.js" => serve_js(include_str!("../static/js/lexicon.js")),
        "js/help.js" => serve_js(include_str!("../static/js/help.js")),
        "js/inspection.js" => serve_js(include_str!("../static/js/inspection.js")),
        "js/routes.js" => serve_js(include_str!("../static/js/routes.js")),
        "js/panels/agent-detail.js" => {
            serve_js(include_str!("../static/js/panels/agent-detail.js"))
        }
        "js/navigation.js" => serve_js(include_str!("../static/js/navigation.js")),
        "js/task-composer.js" => serve_js(include_str!("../static/js/task-composer.js")),
        "js/panels/operations.js" => serve_js(include_str!("../static/js/panels/operations.js")),
        "js/panels/evidence.js" => serve_js(include_str!("../static/js/panels/evidence.js")),
        "icons/LICENSE" => Response::builder()
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from(include_str!("../static/icons/LICENSE")))
            .expect("valid response"),
        "icons/arkavo.svg" => Response::builder()
            .header(header::CONTENT_TYPE, "image/svg+xml")
            .body(Body::from(include_str!("../static/icons/arkavo.svg")))
            .expect("valid response"),
        "js/state.js" => serve_js(include_str!("../static/js/state.js")),
        "js/websocket.js" => serve_js(include_str!("../static/js/websocket.js")),
        "js/panels/agents.js" => serve_js(include_str!("../static/js/panels/agents.js")),
        "js/panels/tasks.js" => serve_js(include_str!("../static/js/panels/tasks.js")),
        "js/panels/telemetry.js" => serve_js(include_str!("../static/js/panels/telemetry.js")),
        "js/panels/budget.js" => serve_js(include_str!("../static/js/panels/budget.js")),
        "js/panels/health.js" => serve_js(include_str!("../static/js/panels/health.js")),
        "js/panels/security.js" => serve_js(include_str!("../static/js/panels/security.js")),
        "js/panels/router.js" => serve_js(include_str!("../static/js/panels/router.js")),
        "js/panels/learning-charts.js" => {
            serve_js(include_str!("../static/js/panels/learning-charts.js"))
        }
        "js/panels/learning.js" => serve_js(include_str!("../static/js/panels/learning.js")),
        "js/panels/context-charts.js" => {
            serve_js(include_str!("../static/js/panels/context-charts.js"))
        }
        "js/panels/context.js" => serve_js(include_str!("../static/js/panels/context.js")),
        "js/panels/arp.js" => serve_js(include_str!("../static/js/panels/arp.js")),
        "js/panels/published-trust.js" => {
            serve_js(include_str!("../static/js/panels/published-trust.js"))
        }
        "js/panels/task-detail.js" => serve_js(include_str!("../static/js/panels/task-detail.js")),
        "js/chat.js" => serve_js(include_str!("../static/js/chat.js")),
        "js/task-assets.js" => serve_js(include_str!("../static/js/task-assets.js")),
        "js/notices.js" => serve_js(include_str!("../static/js/notices.js")),
        "js/app.js" => serve_js(include_str!("../static/js/app.js")),
        _ => Response::builder()
            .status(404)
            .body(Body::from("Not Found"))
            .expect("valid response")
            .into_response(),
    }
}

fn serve_js(content: &'static str) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "application/javascript")
        .body(Body::from(content))
        .expect("valid response")
        .into_response()
}

fn serve_css(content: &'static str) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "text/css")
        .body(Body::from(content))
        .expect("valid response")
        .into_response()
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "Tokio's test macro starts a runtime outside async execution"
)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn every_shell_asset_is_embedded_and_served_with_its_content_type() {
        let html = index_handler().await.0;
        for suffix in html.split("/static/").skip(1) {
            let path = suffix.split('"').next().unwrap();
            let response = static_file_handler(AxumPath(path.to_owned())).await;
            assert_eq!(response.status(), 200, "missing shell asset: {path}");
            let expected = if path.ends_with(".js") {
                "application/javascript"
            } else if path.ends_with(".svg") {
                "image/svg+xml"
            } else {
                "text/css"
            };
            assert_eq!(response.headers()[header::CONTENT_TYPE], expected);
        }
        let license = static_file_handler(AxumPath("icons/LICENSE".into())).await;
        assert_eq!(license.status(), 200);
        assert_eq!(
            license.headers()[header::CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );
    }

    #[tokio::test]
    async fn static_asset_allowlist_rejects_path_traversal() {
        for path in ["../Cargo.toml", "js/../../../src/gateway.rs", "missing.js"] {
            let response = static_file_handler(AxumPath(path.into())).await;
            assert_eq!(response.status(), 404);
        }
    }
}
