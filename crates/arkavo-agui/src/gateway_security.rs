use axum::http::{HeaderValue, header};
use tower_http::set_header::SetResponseHeaderLayer;

type H = SetResponseHeaderLayer<HeaderValue>;

#[allow(clippy::type_complexity)]
pub fn security_headers() -> tower::ServiceBuilder<
    tower::layer::util::Stack<
        H,
        tower::layer::util::Stack<
            H,
            tower::layer::util::Stack<
                H,
                tower::layer::util::Stack<
                    H,
                    tower::layer::util::Stack<H, tower::layer::util::Identity>,
                >,
            >,
        >,
    >,
> {
    tower::ServiceBuilder::new()
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_XSS_PROTECTION,
            HeaderValue::from_static("0"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(
                "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self' ws: wss:",
            ),
        ))
}
