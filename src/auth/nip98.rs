use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use log::debug;
use nostr::{Event, JsonUtil, Kind, TagKind, Timestamp};
use url::Url;

const DEFAULT_EXPIRATION_SECS: u64 = 60 * 10; // 10 minutes

pub struct Nip98Auth {
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub event: Event,
}

/// Rejection response for NIP-98 auth failures.
///
/// Sets the `x-reason` header so both the client and server-side logging
/// middleware can see why auth was rejected.
pub struct Nip98Rejection {
    status: StatusCode,
    reason: &'static str,
}

impl IntoResponse for Nip98Rejection {
    fn into_response(self) -> Response {
        let mut headers = HeaderMap::new();
        if let Ok(v) = self.reason.parse() {
            headers.insert("x-reason", v);
        }
        (self.status, headers).into_response()
    }
}

impl<S> FromRequestParts<S> for Nip98Auth
where
    S: Send + Sync,
{
    type Rejection = Nip98Rejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth = parts
            .headers
            .get("authorization")
            .ok_or(Nip98Rejection { status: StatusCode::FORBIDDEN, reason: "Auth header not found" })?
            .to_str()
            .map_err(|_| Nip98Rejection { status: StatusCode::FORBIDDEN, reason: "Invalid auth header" })?;

        if !auth.starts_with("Nostr ") {
            return Err(Nip98Rejection { status: StatusCode::FORBIDDEN, reason: "Auth scheme must be Nostr" });
        }

        let event = BASE64_STANDARD
            .decode(&auth[6..])
            .map_err(|_| Nip98Rejection { status: StatusCode::FORBIDDEN, reason: "Invalid auth string" })?;

        let event =
            Event::from_json(event).map_err(|_| Nip98Rejection { status: StatusCode::FORBIDDEN, reason: "Invalid nostr event" })?;

        if event.kind != Kind::HttpAuth {
            return Err(Nip98Rejection { status: StatusCode::UNAUTHORIZED, reason: "Wrong event kind" });
        }

        // Get expiration from tag, or use default (10 minutes from created_at)
        let expiration = event
            .tags
            .find(TagKind::Expiration)
            .and_then(|t| t.content())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or_else(|| event.created_at.as_secs() + DEFAULT_EXPIRATION_SECS);

        let now = Timestamp::now().as_secs();

        // Check "not before" - created_at should be in the past or very near future (allow 60s clock skew)
        if event.created_at.as_secs() > now + 60 {
            return Err(Nip98Rejection {
                status: StatusCode::UNAUTHORIZED,
                reason: "Event created_at is in the future",
            });
        }

        // Check "not after" - expiration should be in the future
        if now > expiration {
            return Err(Nip98Rejection {
                status: StatusCode::UNAUTHORIZED,
                reason: "Event has expired",
            });
        }

        // Check url tag - match any 'u' tag against the full URL (excluding query args)
        let request_path = parts.uri.path();
        let url_tags: Vec<_> = event.tags.filter(TagKind::u()).collect();

        if url_tags.is_empty() {
            return Err(Nip98Rejection {
                status: StatusCode::UNAUTHORIZED,
                reason: "Missing url tag",
            });
        }

        let url_matched = url_tags.iter().any(|tag| {
            tag.content()
                .and_then(|s| s.parse::<Url>().ok())
                .map(|u| u.path() == request_path)
                .unwrap_or(false)
        });

        if !url_matched {
            return Err(Nip98Rejection {
                status: StatusCode::UNAUTHORIZED,
                reason: "U tag does not match request URL",
            });
        }

        // check method tag - match any 'method' tag against the request method
        let method_tags: Vec<_> = event.tags.filter(TagKind::Method).collect();

        if method_tags.is_empty() {
            return Err(Nip98Rejection {
                status: StatusCode::UNAUTHORIZED,
                reason: "Missing method tag",
            });
        }

        let method_matched = method_tags.iter().any(|tag| {
            tag.content()
                .map(|m| m.eq_ignore_ascii_case(parts.method.as_str()))
                .unwrap_or(false)
        });

        if !method_matched {
            return Err(Nip98Rejection {
                status: StatusCode::UNAUTHORIZED,
                reason: "Method tag does not match request method",
            });
        }

        event
            .verify()
            .map_err(|_| Nip98Rejection { status: StatusCode::UNAUTHORIZED, reason: "Event signature invalid" })?;

        debug!("{}", event.as_json());

        let content_type = parts
            .headers
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        let content_length = parts
            .headers
            .get("content-length")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse().ok());

        Ok(Nip98Auth {
            event,
            content_type,
            content_length,
        })
    }
}
