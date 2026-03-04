use axum::http::HeaderName;
use tower_http::cors::CorsLayer;

pub fn cors_layer() -> CorsLayer {
    CorsLayer::very_permissive().expose_headers([HeaderName::from_static("x-reason")])
}
