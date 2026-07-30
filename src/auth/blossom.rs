use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use base64::prelude::*;
use log::info;
use nostr::{Alphabet, Event, JsonUtil, Kind, SingleLetterTag, TagKind, Timestamp};

pub struct BlossomAuth {
    pub content_type: Option<String>,
    pub x_content_type: Option<String>,
    pub x_sha_256: Option<String>,
    pub x_content_length: Option<u64>,
    /// BUD-12: client acknowledgement of a prior 409 identical-media response.
    /// Contains the decoded SHA-256 bytes the server previously returned in
    /// `X-Identical-Media`, signalling that the client wants to store a
    /// distinct copy regardless.
    pub x_identical_media: Option<Vec<u8>>,
    pub event: Event,
}

/// Rejection response for Blossom auth failures.
///
/// Sets the `x-reason` header so both the client and server-side logging
/// middleware can see why auth was rejected.
pub struct BlossomRejection {
    status: StatusCode,
    reason: &'static str,
}

impl IntoResponse for BlossomRejection {
    fn into_response(self) -> Response {
        let mut headers = HeaderMap::new();
        if let Ok(v) = self.reason.parse() {
            headers.insert("x-reason", v);
        }
        (self.status, headers).into_response()
    }
}

/// Reduce a `server` tag value or configured public URL to a bare lowercase host.
/// Strips any scheme, userinfo, port, path, query and fragment so that
/// `https://cdn.example.com:443/foo` and `cdn.example.com` compare equal.
fn normalize_server_host(value: &str) -> String {
    let v = value.trim().to_lowercase();
    // Strip scheme
    let v = match v.split_once("://") {
        Some((_, rest)) => rest,
        None => v.as_str(),
    };
    // Strip path / query / fragment
    let v = v.split(['/', '?', '#']).next().unwrap_or(v);
    // Strip userinfo
    let v = match v.rsplit_once('@') {
        Some((_, host)) => host,
        None => v,
    };
    // Strip port (IPv6 literals keep their brackets)
    let host = if v.starts_with('[') {
        match v.find(']') {
            Some(end) => &v[..=end],
            None => v,
        }
    } else {
        match v.split_once(':') {
            Some((h, _)) => h,
            None => v,
        }
    };
    host.to_string()
}

impl BlossomAuth {
    /// Get all x tags from the authorization event
    pub fn x_tags(&self) -> Vec<String> {
        self.event
            .tags
            .iter()
            .filter_map(|t| {
                if t.kind() == TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::X)) {
                    t.content().map(|s| s.to_lowercase())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get all server tags from the authorization event
    pub fn server_tags(&self) -> Vec<String> {
        self.event
            .tags
            .iter()
            .filter_map(|t| {
                if t.kind() == TagKind::Server {
                    t.content().map(|s| s.to_lowercase())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Validate x tag requirement for endpoints that require it.
    /// Returns Ok(()) if at least one x tag matches the expected hash.
    pub fn validate_x_tag(&self, expected_hash: &str) -> Result<(), BlossomRejection> {
        let expected_lower = expected_hash.to_lowercase();
        let has_match = self.x_tags().iter().any(|h| h == &expected_lower);

        if has_match {
            Ok(())
        } else {
            Err(BlossomRejection {
                status: StatusCode::UNAUTHORIZED,
                reason: "Missing or mismatched x tag",
            })
        }
    }

    /// Validate server tag requirement.
    /// If server tags are present, the server's domain must be in the list.
    /// Returns Ok(()) if no server tags are present (unscoped token) or if the server is in the list.
    ///
    /// BUD-11 (Tag scoping): the `server` tag value "MUST be a lowercase domain name
    /// only (e.g. `cdn.example.com`), not a full URL". Callers pass `public_url`,
    /// which is a full URL, so both sides are normalised to a bare host before
    /// comparison. Previously this compared the tag against the raw `public_url`,
    /// which rejected every spec-compliant token and only accepted full-URL tags.
    pub fn validate_server_tag(&self, server_domain: &str) -> Result<(), BlossomRejection> {
        let server_tags = self.server_tags();

        // If no server tags, token is valid on any server (unscoped)
        if server_tags.is_empty() {
            return Ok(());
        }

        let expected = normalize_server_host(server_domain);
        let has_match = server_tags
            .iter()
            .any(|s| normalize_server_host(s) == expected);

        if has_match {
            Ok(())
        } else {
            Err(BlossomRejection {
                status: StatusCode::UNAUTHORIZED,
                reason: "Server not in authorization token scope",
            })
        }
    }
}

impl<S> FromRequestParts<S> for BlossomAuth
where
    S: Send + Sync,
{
    type Rejection = BlossomRejection;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth = parts
            .headers
            .get("authorization")
            .ok_or(BlossomRejection {
                status: StatusCode::UNAUTHORIZED,
                reason: "Auth header not found",
            })?
            .to_str()
            .map_err(|_| BlossomRejection {
                status: StatusCode::BAD_REQUEST,
                reason: "Invalid auth header",
            })?;

        if !auth.starts_with("Nostr ") {
            return Err(BlossomRejection {
                status: StatusCode::BAD_REQUEST,
                reason: "Auth scheme must be Nostr",
            });
        }

        let event = BASE64_STANDARD
            .decode(&auth[6..])
            .map_err(|_| BlossomRejection {
                status: StatusCode::BAD_REQUEST,
                reason: "Invalid auth string",
            })?;

        let event = Event::from_json(event).map_err(|_| BlossomRejection {
            status: StatusCode::BAD_REQUEST,
            reason: "Invalid nostr event",
        })?;

        if event.kind != Kind::Custom(24242) {
            return Err(BlossomRejection {
                status: StatusCode::BAD_REQUEST,
                reason: "Wrong event kind",
            });
        }

        if (event.created_at.as_secs() as i64 - Timestamp::now().as_secs() as i64).unsigned_abs()
            >= 60 * 3
        {
            return Err(BlossomRejection {
                status: StatusCode::BAD_REQUEST,
                reason: "Created timestamp is out of range",
            });
        }

        // check expiration tag
        if let Some(expiration) = event.tags.iter().find_map(|t| {
            if t.kind() == TagKind::Expiration {
                t.content()
            } else {
                None
            }
        }) {
            let u_exp: Timestamp = expiration.parse().unwrap();
            if u_exp <= Timestamp::now() {
                return Err(BlossomRejection {
                    status: StatusCode::BAD_REQUEST,
                    reason: "Expiration invalid",
                });
            }
        } else {
            return Err(BlossomRejection {
                status: StatusCode::BAD_REQUEST,
                reason: "Missing expiration tag",
            });
        }

        event.verify().map_err(|_| BlossomRejection {
            status: StatusCode::BAD_REQUEST,
            reason: "Event signature invalid",
        })?;

        info!("{}", event.as_json());

        let content_type = parts
            .headers
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        let x_sha_256 = parts
            .headers
            .get("x-sha-256")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        let x_content_length = parts
            .headers
            .get("x-content-length")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse().ok());

        let x_content_type = parts
            .headers
            .get("x-content-type")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());

        let x_identical_media = parts
            .headers
            .get("x-identical-media")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| hex::decode(s).ok());

        Ok(BlossomAuth {
            event,
            content_type,
            x_sha_256,
            x_content_length,
            x_content_type,
            x_identical_media,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_server_host;

    /// BUD-11: the `server` tag "MUST be a lowercase domain name only
    /// (e.g. `cdn.example.com`), not a full URL". Callers pass `public_url`
    /// (a full URL), so both sides must normalise to the same bare host.
    #[test]
    fn normalizes_full_url_to_bare_host() {
        assert_eq!(
            normalize_server_host("https://cdn.example.com"),
            "cdn.example.com"
        );
        assert_eq!(normalize_server_host("http://localhost:8000"), "localhost");
        assert_eq!(
            normalize_server_host("https://cdn.example.com:443/path?q=1#f"),
            "cdn.example.com"
        );
    }

    #[test]
    fn normalizes_bare_domain_unchanged() {
        assert_eq!(normalize_server_host("cdn.example.com"), "cdn.example.com");
        assert_eq!(normalize_server_host("localhost"), "localhost");
    }

    #[test]
    fn normalization_is_case_insensitive_and_trims() {
        assert_eq!(
            normalize_server_host("  HTTPS://CDN.Example.COM/  "),
            "cdn.example.com"
        );
    }

    #[test]
    fn strips_userinfo() {
        assert_eq!(
            normalize_server_host("https://user:pass@cdn.example.com"),
            "cdn.example.com"
        );
    }

    #[test]
    fn preserves_ipv6_literal() {
        assert_eq!(normalize_server_host("http://[::1]:8000"), "[::1]");
    }

    /// Regression: a spec-compliant bare-domain tag must match a full-URL public_url.
    #[test]
    fn bare_domain_tag_matches_full_url_public_url() {
        assert_eq!(
            normalize_server_host("localhost"),
            normalize_server_host("http://localhost:8000")
        );
    }
}
