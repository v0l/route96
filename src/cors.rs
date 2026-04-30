use axum::http::{HeaderName, Method};
use tower_http::cors::CorsLayer;

pub fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_methods([
            Method::GET,
            Method::HEAD,
            Method::PUT,
            Method::POST,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            HeaderName::from_static("authorization"),
            HeaderName::from_static("content-type"),
            HeaderName::from_static("x-sha-256"),
            HeaderName::from_static("x-content-length"),
            HeaderName::from_static("x-content-type"),
            HeaderName::from_static("x-identical-media"),
        ])
        .expose_headers([
            HeaderName::from_static("x-reason"),
            HeaderName::from_static("x-identical-media"),
            HeaderName::from_static("sunset"),
        ])
}
