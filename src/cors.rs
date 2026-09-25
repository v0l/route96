use axum::http::{HeaderName, Method, header};
use tower_http::cors::CorsLayer;

#[cfg(feature = "blossom")]
use axum::{extract::Request, middleware::Next, response::Response};

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
            header::ALLOW,
            HeaderName::from_static("x-reason"),
            HeaderName::from_static("x-identical-media"),
            HeaderName::from_static("sunset"),
        ])
}

/// Correct the generic CORS method list for BUD-13 hash resources.
///
/// The application also has POST endpoints, so the global CORS layer must
/// allow POST. BUD-01 feature discovery, however, requires `/<sha256>` to
/// advertise only methods actually supported on that resource.
#[cfg(feature = "blossom")]
pub async fn bud13_discovery_headers(request: Request, next: Next) -> Response {
    let is_hash_resource = request
        .uri()
        .path()
        .strip_prefix('/')
        .is_some_and(crate::routes::blossom::is_bud13_sha256);
    let is_options = request.method() == Method::OPTIONS;

    let mut response = next.run(request).await;
    if is_hash_resource
        && (is_options || response.status() == axum::http::StatusCode::METHOD_NOT_ALLOWED)
    {
        response.headers_mut().insert(
            header::ALLOW,
            crate::routes::blossom::HASH_RESOURCE_METHODS
                .parse()
                .expect("valid Allow value"),
        );
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            crate::routes::blossom::HASH_RESOURCE_METHODS
                .parse()
                .expect("valid Access-Control-Allow-Methods value"),
        );
    }

    response
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

    #[cfg(feature = "blossom")]
    #[tokio::test]
    async fn bud13_discovery_omits_unsupported_post() {
        use axum::middleware;

        let hash = "a".repeat(64);
        let app = Router::new()
            .route("/{sha256}", get(|| async { "ok" }))
            .layer(cors_layer())
            .layer(middleware::from_fn(super::bud13_discovery_headers));

        let response = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri(format!("/{hash}"))
                    .header("origin", "https://example.com")
                    .header("access-control-request-method", "PUT")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let allow = response.headers().get("allow").unwrap().to_str().unwrap();
        let cors_allow = response
            .headers()
            .get("access-control-allow-methods")
            .unwrap()
            .to_str()
            .unwrap();
        for method in ["GET", "HEAD", "PUT", "DELETE", "OPTIONS"] {
            assert!(allow.contains(method));
            assert!(cors_allow.contains(method));
        }
        assert!(!allow.contains("POST"));
        assert!(!cors_allow.contains("POST"));
    }
}
