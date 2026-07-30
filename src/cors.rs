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
        // BUD-01: servers MUST set, at minimum,
        // `Access-Control-Allow-Headers: Authorization, *`.
        // The `*` wildcard does not cover `Authorization` under the CORS spec
        // (it is a forbidden-ish header that must be named explicitly), which is
        // why both are required. The remaining names are listed for clarity and
        // for clients that do not honour the wildcard.
        .allow_headers([
            HeaderName::from_static("authorization"),
            HeaderName::from_static("content-type"),
            HeaderName::from_static("x-sha-256"),
            HeaderName::from_static("x-content-length"),
            HeaderName::from_static("x-content-type"),
            HeaderName::from_static("x-identical-media"),
            HeaderName::from_static("*"),
        ])
        .expose_headers([
            HeaderName::from_static("x-reason"),
            HeaderName::from_static("x-identical-media"),
            HeaderName::from_static("sunset"),
        ])
}

#[cfg(test)]
mod tests {
    use super::cors_layer;
    use axum::{Router, body::Body, http::Request, routing::get};
    use tower::ServiceExt;

    /// BUD-01: "servers MUST also set, at minimum, the
    /// `Access-Control-Allow-Headers: Authorization, *` and
    /// `Access-Control-Allow-Methods: GET, HEAD, PUT, DELETE` headers."
    #[tokio::test]
    async fn preflight_allows_authorization_and_wildcard() {
        let app = Router::new()
            .route("/", get(|| async { "ok" }))
            .layer(cors_layer());

        let res = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/")
                    .header("origin", "https://example.com")
                    .header("access-control-request-method", "PUT")
                    .header("access-control-request-headers", "authorization")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let headers = res.headers();

        let allow_headers = headers
            .get("access-control-allow-headers")
            .expect("preflight must set access-control-allow-headers")
            .to_str()
            .unwrap()
            .to_lowercase();
        assert!(
            allow_headers.contains("authorization"),
            "must allow Authorization, got: {allow_headers}"
        );
        assert!(
            allow_headers.split(',').any(|h| h.trim() == "*"),
            "must allow the `*` wildcard, got: {allow_headers}"
        );

        let allow_methods = headers
            .get("access-control-allow-methods")
            .expect("preflight must set access-control-allow-methods")
            .to_str()
            .unwrap()
            .to_uppercase();
        for m in ["GET", "HEAD", "PUT", "DELETE"] {
            assert!(
                allow_methods.contains(m),
                "must allow {m}, got: {allow_methods}"
            );
        }

        assert_eq!(
            headers
                .get("access-control-allow-origin")
                .map(|v| v.to_str().unwrap()),
            Some("*"),
            "BUD-01 requires Access-Control-Allow-Origin: *"
        );
    }
}
