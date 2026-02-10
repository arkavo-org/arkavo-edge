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
        "js/state.js" => serve_js(include_str!("../static/js/state.js")),
        "js/websocket.js" => serve_js(include_str!("../static/js/websocket.js")),
        "js/panels/agents.js" => serve_js(include_str!("../static/js/panels/agents.js")),
        "js/panels/tasks.js" => serve_js(include_str!("../static/js/panels/tasks.js")),
        "js/panels/telemetry.js" => serve_js(include_str!("../static/js/panels/telemetry.js")),
        "js/panels/budget.js" => serve_js(include_str!("../static/js/panels/budget.js")),
        "js/panels/health.js" => serve_js(include_str!("../static/js/panels/health.js")),
        "js/panels/router.js" => serve_js(include_str!("../static/js/panels/router.js")),
        "js/chat.js" => serve_js(include_str!("../static/js/chat.js")),
        "js/app.js" => serve_js(include_str!("../static/js/app.js")),
        _ => Response::builder()
            .status(404)
            .body(Body::from("Not Found"))
            .unwrap()
            .into_response(),
    }
}

fn serve_js(content: &'static str) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "application/javascript")
        .body(Body::from(content))
        .unwrap()
        .into_response()
}

fn serve_css(content: &'static str) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "text/css")
        .body(Body::from(content))
        .unwrap()
        .into_response()
}
