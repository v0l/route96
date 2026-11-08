use axum::extract::State;
use axum::http::uri::Authority;
use axum::http::{HeaderName, HeaderValue, Method, header};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use url::Url;

use axum::{extract::Request, middleware::Next, response::IntoResponse as _, response::Response};

use crate::routes::AppState;

/// Request paths that carry privileged or user-private JSON APIs. Unlike the
/// blob endpoints (whose permissive CORS is required by BUD-01), these
/// responses have no reason to be readable by arbitrary websites: an admin
/// using a NIP-07 extension that auto-approves signing could otherwise have
/// every website they visit invoke window.nostr and read admin API responses.
const SAME_ORIGIN_PATH_PREFIXES: &[&str] = &["admin", "user", "setup"];

fn is_same_origin_scoped_path(path: &str) -> bool {
    // Collapse leading slashes: a proxy that normalizes "//admin/stats" into
    // a downstream "/admin/stats" route must not slip past the scope check.
    let trimmed = path.trim_start_matches('/');
    SAME_ORIGIN_PATH_PREFIXES
        .iter()
        .any(|prefix| trimmed == *prefix || trimmed.starts_with(&format!("{prefix}/")))
}

fn origin_matches(origin: &Url, candidate: &Url) -> bool {
    origin.scheme() == candidate.scheme()
        && origin.host_str().map(str::to_ascii_lowercase)
            == candidate.host_str().map(str::to_ascii_lowercase)
        && origin.port_or_known_default() == candidate.port_or_known_default()
}

fn is_loopback_host(host: &str) -> bool {
    let stripped = host.trim_start_matches('[').trim_end_matches(']');
    stripped == "localhost" || stripped == "127.0.0.1" || stripped == "::1"
}

/// Compare a cross-origin request's Origin against a raw Host header.
///
/// The Host header is parsed strictly as an HTTP authority - never spliced
/// into a URL - because it is attacker-controlled on direct exposure: values
/// like "evil.com#@legitimate.com" or "user:pass@evil.com" must not reach
/// Url::parse, which would land on the attacker's host. The scheme comes
/// from the parsed Origin, so a plain http Origin cannot claim an https Host
/// (and vice versa) - unless the origin is loopback local development while
/// public_url is https.
fn host_matches_origin(origin_url: &Url, host_str: &str, public_url: Option<&str>) -> bool {
    if !matches!(origin_url.scheme(), "http" | "https") {
        return false;
    }
    // An https public_url forbids an http Origin downgrade via the Host
    // fallback - except loopback origins, which is how local development
    // talks to a production-configured server.
    if let Some(pub_url) = public_url.and_then(|u| Url::parse(u).ok())
        && pub_url.scheme() == "https"
        && origin_url.scheme() != "https"
        && !origin_url.host_str().is_some_and(is_loopback_host)
    {
        return false;
    }
    let Ok(authority) = host_str.parse::<Authority>() else {
        return false;
    };
    if authority.as_str().contains('@') {
        return false; // userinfo in a Host header is always hostile
    }
    let Some(origin_host) = origin_url.host_str() else {
        return false;
    };
    // Authority::host() is bracket-free for IPv6; normalize the Origin side.
    let origin_host = origin_host.trim_start_matches('[').trim_end_matches(']');
    if !origin_host.eq_ignore_ascii_case(authority.host()) {
        return false;
    }
    let default_port = match origin_url.scheme() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    origin_url.port().or(default_port) == authority.port_u16().or(default_port)
}

/// A foreign-origin request is allowed when its Origin matches the request's
/// Host header OR the configured public_url. Comparing only the Host header
/// breaks legitimate admin traffic behind a reverse proxy that rewrites Host
/// (e.g. nginx passing cdn.example.com through as 127.0.0.1:8000), so the
/// browser-visible origin from settings must be accepted too. Same-origin
/// browser GET requests send no Origin header at all, so a missing Origin is
/// always allowed.
fn origin_is_allowed(
    origin: Option<&HeaderValue>,
    host: Option<&HeaderValue>,
    public_url: Option<&str>,
) -> bool {
    let Some(origin) = origin.and_then(|o| o.to_str().ok()) else {
        return true;
    };
    let Ok(origin_url) = Url::parse(origin) else {
        return false;
    };
    if !matches!(origin_url.scheme(), "http" | "https") {
        return false;
    }
    // Candidate 1: the request's Host header (strict authority parsing).
    if let Some(host) = host.and_then(|h| h.to_str().ok())
        && host_matches_origin(&origin_url, host, public_url)
    {
        return true;
    }
    // Candidate 2: the configured public_url (both sides fully parsed, so
    // scheme comparison is real here - no construction to self-fulfill).
    if let Some(public_url) = public_url
        && let Ok(candidate) = Url::parse(public_url)
        && origin_matches(&origin_url, &candidate)
    {
        return true;
    }
    false
}

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

/// Same-origin guard for privileged API paths (/admin, /user, /setup):
/// foreign-origin requests are rejected with 403 before any handler runs and
/// before CorsLayer can attach wildcard headers, so malicious pages cannot
/// read responses from those endpoints even when a victim's NIP-07 extension
/// auto-approves signing. The BUD-01 wildcard is untouched on all other
/// paths. Must be layered OUTSIDE [`cors_layer`] (via `from_fn_with_state`)
/// so the early 403 short-circuits both CorsLayer and the handler.
pub async fn scope_admin_cors(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    // Fast path: blob/thumb/NIP-96 traffic never touches the settings lock.
    if !is_same_origin_scoped_path(request.uri().path()) {
        return next.run(request).await;
    }
    let public_url = state.settings().await.public_url;
    scope_admin_cors_inner(Some(public_url), request, next).await
}

/// Testable core: `public_url` is the configured browser-visible origin.
/// Wire with `from_fn_with_state` (see main.rs), layered OUTSIDE
/// `cors_layer` so the 403 short-circuit precedes CorsLayer and handlers.
pub async fn scope_admin_cors_inner(
    public_url: Option<String>,
    request: Request,
    next: Next,
) -> Response {
    // Only privileged paths are subject to the same-origin scope; everything
    // else (blobs, thumbs, NIP-96, payment, props) flows straight through.
    if !is_same_origin_scoped_path(request.uri().path()) {
        return next.run(request).await;
    }

    let origin = request.headers().get(header::ORIGIN);
    let host = request.headers().get(header::HOST);

    if origin_is_allowed(origin, host, public_url.as_deref()) {
        return next.run(request).await;
    }

    // Reject foreign-origin preflights AND actual requests BEFORE any
    // handler runs and BEFORE CorsLayer can attach wildcard headers: this
    // layer sits outside cors_layer, so the early return never gains
    // Access-Control-Allow-* headers at all.
    axum::http::StatusCode::FORBIDDEN.into_response()
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
    use super::{cors_layer, scope_admin_cors_inner};
    use axum::middleware::Next;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
        middleware,
        routing::get,
    };
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

    fn admin_app() -> Router {
        // Middleware is layered outside cors_layer so the response filter
        // sees the CORS headers the layer produced. Uses the inner core with
        // a fixed test public_url (the stateful wrapper needs AppState).
        Router::new()
            .route("/admin/stats", get(|| async { "ok" }))
            .route("/n96", get(|| async { "nip96" }))
            .route("/{sha256}", get(|| async { "blob" }))
            .layer(cors_layer())
            .layer(middleware::from_fn(
                move |req: Request<Body>, next: Next| {
                    let public_url = Some("https://cdn.example.com".to_string());
                    async move { scope_admin_cors_inner(public_url, req, next).await }
                },
            ))
    }

    async fn preflight(
        app: Router,
        path: &str,
        origin: Option<&str>,
        host: &str,
    ) -> axum::http::Response<Body> {
        let mut builder = Request::builder().method("OPTIONS").uri(path);
        if let Some(o) = origin {
            builder = builder.header("origin", o);
        }
        builder = builder.header("host", host);
        builder = builder.header("access-control-request-method", "GET");
        app.oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    /// Privileged endpoints (/admin, /user, /setup) are not readable by
    /// arbitrary origins: a website can otherwise pair a NIP-07 extension's
    /// auto-approved signing with cross-origin reads of admin API responses.
    #[tokio::test]
    async fn admin_preflight_rejects_foreign_origins() {
        let app = admin_app();

        let res = preflight(
            app.clone(),
            "/admin/stats",
            Some("https://evil.example"),
            "cdn.example.com",
        )
        .await;
        assert_eq!(
            res.status(),
            StatusCode::FORBIDDEN,
            "foreign preflight must 403"
        );
        assert_eq!(
            res.headers().get("access-control-allow-origin"),
            None,
            "foreign origin must not be allowed on /admin"
        );

        // Same origin (host matches the Host header) is allowed.
        let res = preflight(
            app.clone(),
            "/admin/stats",
            Some("https://cdn.example.com"),
            "cdn.example.com",
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get("access-control-allow-origin")
                .map(|v| v.to_str().unwrap()),
            Some("*")
        );

        // Same host, different port is a different origin.
        let res = preflight(
            app.clone(),
            "/admin/stats",
            Some("https://cdn.example.com:8443"),
            "cdn.example.com",
        )
        .await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert_eq!(res.headers().get("access-control-allow-origin"), None);

        // No Origin header (same-origin browser request) is allowed.
        let res = preflight(app.clone(), "/admin/stats", None, "cdn.example.com").await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get("access-control-allow-origin")
                .map(|v| v.to_str().unwrap()),
            Some("*")
        );
    }

    /// Blob endpoints keep the BUD-01 wildcard regardless of origin.
    #[tokio::test]
    async fn blob_preflight_keeps_wildcard_origin() {
        let app = admin_app();
        let res = preflight(
            app,
            "/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            Some("https://evil.example"),
            "cdn.example.com",
        )
        .await;
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get("access-control-allow-origin")
                .map(|v| v.to_str().unwrap()),
            Some("*")
        );
    }

    /// A cross-origin simple GET (no preflight) on a privileged path is
    /// rejected BEFORE the handler runs - 403 with no CORS headers, and the
    /// handler never executes (no state reads, no auth-side effects).
    #[tokio::test]
    async fn admin_get_rejects_foreign_origin_before_handler() {
        let app = admin_app();
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/stats")
                    .header("origin", "https://evil.example")
                    .header("host", "cdn.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        assert_eq!(res.headers().get("access-control-allow-origin"), None);
    }

    /// An http Origin cannot claim an https Host via the Host-header
    /// candidate when public_url is https (scheme-downgrade block).
    #[tokio::test]
    async fn admin_rejects_http_downgrade_when_public_url_is_https() {
        let app = admin_app();
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/stats")
                    .header("origin", "http://cdn.example.com")
                    .header("host", "cdn.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    /// Loopback origins keep the http carve-out for local development even
    /// when public_url is https.
    #[tokio::test]
    async fn admin_allows_loopback_http_dev_origin() {
        let app = admin_app();
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/stats")
                    .header("origin", "http://localhost:5173")
                    .header("host", "localhost:5173")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    /// Host-header authority injection ("evil.com#@legitimate.com") fails
    /// Authority parsing ('#' is not a valid authority character).
    #[tokio::test]
    async fn admin_rejects_host_fragment_injection() {
        let app = admin_app();
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/stats")
                    .header("origin", "https://evil.example")
                    .header("host", "evil.example#@cdn.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    /// Userinfo in a Host header ("user:pass@evil.example") is rejected
    /// outright - never handed to URL parsing.
    #[tokio::test]
    async fn admin_rejects_userinfo_in_host() {
        let app = admin_app();
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/stats")
                    .header("origin", "https://evil.example")
                    .header("host", "user:pass@evil.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    /// Leading double slashes must not slip a privileged path past the
    /// scope check when a proxy normalizes the path downstream.
    #[tokio::test]
    async fn admin_scope_survives_leading_double_slash() {
        let app = admin_app();
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("//admin/stats")
                    .header("origin", "https://evil.example")
                    .header("host", "cdn.example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
    }

    /// Behind a reverse proxy that rewrites Host (nginx passing
    /// cdn.example.com through as 127.0.0.1:8000), the browser-visible
    /// Origin no longer matches Host - the configured public_url must be
    /// accepted as the admin UI's origin too.
    #[tokio::test]
    async fn admin_accepts_origin_matching_public_url_behind_proxy() {
        let app = admin_app();
        let res = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/admin/stats")
                    .header("origin", "https://cdn.example.com")
                    .header("host", "127.0.0.1:8000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get("access-control-allow-origin")
                .map(|v| v.to_str().unwrap()),
            Some("*")
        );
    }

    /// NIP-96 endpoints are public web-client surface (BUD-01-style CORS) -
    /// the same-origin scope must not touch them.
    #[tokio::test]
    async fn nip96_endpoints_keep_wildcard_origin() {
        let app = admin_app();
        let res = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/n96")
                    .header("origin", "https://evil.example")
                    .header("host", "cdn.example.com")
                    .header("access-control-request-method", "POST")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers()
                .get("access-control-allow-origin")
                .map(|v| v.to_str().unwrap()),
            Some("*")
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
