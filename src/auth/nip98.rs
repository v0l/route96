use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use log::debug;
use nostr::{Event, JsonUtil, Kind, TagKind, Timestamp};
use std::sync::Arc;
use url::Url;

use crate::routes::AppState;

const MAX_EVENT_AGE_SECS: u64 = 60;

pub fn created_at_in_window(created_at: u64, now: u64) -> bool {
    created_at <= now + MAX_EVENT_AGE_SECS && now <= created_at + MAX_EVENT_AGE_SECS
}

pub fn url_matches(tag_url: &str, public_url: &str, request_path: &str) -> bool {
    let (Ok(tag), Ok(base)) = (tag_url.parse::<Url>(), public_url.parse::<Url>()) else {
        return false;
    };
    tag.scheme() == base.scheme()
        && tag.host_str().is_some()
        && tag.host_str() == base.host_str()
        && tag.port_or_known_default() == base.port_or_known_default()
        && tag.path() == request_path
}

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

impl FromRequestParts<Arc<AppState>> for Nip98Auth {
    type Rejection = Nip98Rejection;

    async fn from_request_parts(parts: &mut Parts, state: &Arc<AppState>) -> Result<Self, Self::Rejection> {
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

        let now = Timestamp::now().as_secs();
        let created_at = event.created_at.as_secs();

        if created_at > now + MAX_EVENT_AGE_SECS {
            return Err(Nip98Rejection {
                status: StatusCode::UNAUTHORIZED,
                reason: "Event created_at is in the future",
            });
        }

        let expired_by_tag = event
            .tags
            .find(TagKind::Expiration)
            .and_then(|t| t.content())
            .and_then(|s| s.parse::<u64>().ok())
            .map(|exp| now > exp)
            .unwrap_or(false);

        if expired_by_tag || !created_at_in_window(created_at, now) {
            return Err(Nip98Rejection {
                status: StatusCode::UNAUTHORIZED,
                reason: "Event has expired",
            });
        }

        let public_url = state.settings().await.public_url;

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
                .map(|s| url_matches(s, &public_url, request_path))
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

        // NOTE: NIP-98's optional `payload` tag (body hash) cannot be
        // verified here because `FromRequestParts` has no access to the
        // request body. Replay resistance relies on the short 10-minute
        // expiration window enforced above.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_tag_for_other_host_is_rejected() {
        assert!(!url_matches("https://other.example/admin/self", "https://example.com", "/admin/self"));
    }

    #[test]
    fn url_tag_for_own_host_is_accepted() {
        assert!(url_matches("https://example.com/admin/self", "https://example.com", "/admin/self"));
        assert!(url_matches("https://example.com/admin/self?x=1", "https://example.com/", "/admin/self"));
    }

    #[test]
    fn url_tag_with_other_scheme_or_path_is_rejected() {
        assert!(!url_matches("http://example.com/admin/self", "https://example.com", "/admin/self"));
        assert!(!url_matches("https://example.com/admin/other", "https://example.com", "/admin/self"));
    }

    #[test]
    fn old_created_at_is_rejected_whatever_expiration_says() {
        let now = 10_000_000;
        assert!(!created_at_in_window(now - 30 * 86400, now));
        assert!(!created_at_in_window(now - 61, now));
        assert!(!created_at_in_window(now + 61, now));
        assert!(created_at_in_window(now - 60, now));
        assert!(created_at_in_window(now + 60, now));
    }
}
