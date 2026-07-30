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
    /// Validate that a URL is safe for the server to fetch (mirror requests).
    ///
    /// Rejects non-http(s) schemes and any host that resolves to a
    /// loopback, private, link-local or otherwise non-public address, to
    /// prevent SSRF against internal services / cloud metadata endpoints.
    ///
    /// Returns the validated socket addresses. Callers MUST pin these on the
    /// HTTP client (`resolve_to_addrs`) rather than letting it re-resolve the
    /// hostname: a second lookup can return a different, private address
    /// (DNS rebinding), which would bypass this check entirely.
    pub async fn validate_mirror_url(
        url: &url::Url,
    ) -> Result<Vec<std::net::SocketAddr>, BlossomRejection> {
        use std::net::IpAddr;

        fn ip_is_public(ip: &IpAddr) -> bool {
            match ip {
                IpAddr::V4(v4) => {
                    !(v4.is_private()
                        || v4.is_loopback()
                        || v4.is_link_local()
                        || v4.is_broadcast()
                        || v4.is_documentation()
                        || v4.is_unspecified()
                        // CGNAT / carrier-grade NAT 100.64.0.0/10
                        || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
                        // 0.0.0.0/8, 192.0.0.0/24, 198.18.0.0/15 benchmarking,
                        // 240.0.0.0/4 reserved
                        || v4.octets()[0] == 0
                        || (v4.octets()[0] == 192 && v4.octets()[1] == 0 && v4.octets()[2] == 0)
                        || (v4.octets()[0] == 198 && (v4.octets()[1] & 0xFE) == 18)
                        // 224.0.0.0/4 multicast, 240.0.0.0/4 reserved
                        || v4.octets()[0] >= 224)
                }
                IpAddr::V6(v6) => {
                    let seg = v6.segments();
                    // Any embedded IPv4 address must be re-checked, otherwise a
                    // private v4 can be smuggled through a v6 literal/AAAA record.
                    // to_ipv4() covers both IPv4-mapped (::ffff:0:0/96) and the
                    // deprecated IPv4-compatible (::/96) forms.
                    let embedded_v4_is_private =
                        v6.to_ipv4().is_some_and(|v4| !ip_is_public(&IpAddr::V4(v4)));
                    // NAT64 (RFC 6052) 64:ff9b::/96 and 64:ff9b:1::/48 — translated
                    // straight to the embedded v4 on hosts with a NAT64 path, so
                    // 64:ff9b::a9fe:a9fe would reach 169.254.169.254.
                    let nat64 = seg[0] == 0x0064
                        && (seg[1] == 0xff9b)
                        && {
                            let v4 = std::net::Ipv4Addr::new(
                                (seg[6] >> 8) as u8,
                                (seg[6] & 0xff) as u8,
                                (seg[7] >> 8) as u8,
                                (seg[7] & 0xff) as u8,
                            );
                            !ip_is_public(&IpAddr::V4(v4))
                        };
                    // 6to4 2002::/16 embeds a v4 address in segments 1-2.
                    let six_to_four = seg[0] == 0x2002 && {
                        let v4 = std::net::Ipv4Addr::new(
                            (seg[1] >> 8) as u8,
                            (seg[1] & 0xff) as u8,
                            (seg[2] >> 8) as u8,
                            (seg[2] & 0xff) as u8,
                        );
                        !ip_is_public(&IpAddr::V4(v4))
                    };
                    !(v6.is_loopback()
                        || v6.is_unspecified()
                        || v6.is_unique_local()
                        || v6.is_unicast_link_local()
                        || v6.is_multicast()
                        // 2001:db8::/32 documentation, 2001::/32 Teredo, 100::/64 discard
                        || (seg[0] == 0x2001 && seg[1] == 0x0db8)
                        || (seg[0] == 0x2001 && seg[1] == 0x0000)
                        || (seg[0] == 0x0100 && seg[1] == 0 && seg[2] == 0 && seg[3] == 0)
                        || embedded_v4_is_private
                        || nat64
                        || six_to_four)
                }
            }
        }

        let reject = || BlossomRejection {
            status: StatusCode::BAD_REQUEST,
            reason: "URL is not fetchable by this server",
        };

        if url.scheme() != "http" && url.scheme() != "https" {
            return Err(reject());
        }

        let host = url.host_str().ok_or_else(reject)?;
        let port = url.port_or_known_default().unwrap_or(80);

        // Literal IP host — check directly. Strip brackets so IPv6 literals
        // (`host_str()` yields the bracketed form) are parsed rather than
        // falling through to a DNS lookup that can never succeed.
        let bare = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = bare.parse::<IpAddr>() {
            if !ip_is_public(&ip) {
                return Err(reject());
            }
            return Ok(vec![std::net::SocketAddr::new(ip, port)]);
        }

        // DNS name — resolve and require at least one public address; reject
        // if ANY resolved address is non-public (DNS rebinding safety).
        let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host, port))
            .await
            .map_err(|_| reject())?
            .collect();
        if addrs.is_empty() || addrs.iter().any(|a| !ip_is_public(&a.ip())) {
            return Err(reject());
        }

        Ok(addrs)
    }

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
            let u_exp: Timestamp = match expiration.parse() {
                Ok(t) => t,
                Err(_) => {
                    return Err(BlossomRejection {
                        status: StatusCode::BAD_REQUEST,
                        reason: "Invalid expiration tag",
                    });
                }
            };
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

    // ---- SSRF guard (validate_mirror_url) ----

    async fn rejects(u: &str) -> bool {
        let url = url::Url::parse(u).expect("test url must parse");
        super::BlossomAuth::validate_mirror_url(&url).await.is_err()
    }

    #[tokio::test]
    async fn rejects_non_http_schemes() {
        assert!(rejects("file:///etc/passwd").await);
        assert!(rejects("ftp://example.com/x").await);
        assert!(rejects("gopher://example.com/x").await);
    }

    #[tokio::test]
    async fn rejects_loopback_and_private_literals() {
        for u in [
            "http://127.0.0.1/x",
            "http://10.0.0.5/x",
            "http://192.168.1.1/x",
            "http://172.16.0.1/x",
            "http://169.254.169.254/latest/meta-data/", // cloud metadata
            "http://100.64.0.1/x",                      // CGNAT
            "http://0.0.0.0/x",
        ] {
            assert!(rejects(u).await, "should reject {u}");
        }
    }

    /// The URL parser normalises these to 127.0.0.1 before we see them;
    /// pin that behaviour so an encoding bypass can't regress silently.
    #[tokio::test]
    async fn rejects_obfuscated_ipv4_literals() {
        for u in [
            "http://0177.0.0.1/x",  // octal
            "http://2130706433/x",  // decimal
            "http://127.1/x",       // short form
        ] {
            assert!(rejects(u).await, "should reject {u}");
        }
    }

    /// IPv6 literals arrive bracketed from `host_str()`; these must be parsed
    /// rather than falling through to a DNS lookup.
    #[tokio::test]
    async fn rejects_ipv6_private_literals() {
        for u in [
            "http://[::1]/x",                  // loopback
            "http://[::ffff:127.0.0.1]/x",     // IPv4-mapped
            "http://[::ffff:10.0.0.1]/x",      // IPv4-mapped private
            "http://[fd00::1]/x",              // unique local
            "http://[fe80::1]/x",              // link local
            "http://[64:ff9b::a9fe:a9fe]/x",   // NAT64 -> 169.254.169.254
            "http://[64:ff9b::a00:5]/x",       // NAT64 -> 10.0.0.5
            "http://[2002:7f00:1::]/x",        // 6to4 -> 127.0.0.1
            "http://[2002:a00:5::]/x",         // 6to4 -> 10.0.0.5
            "http://[::7f00:1]/x",             // IPv4-compatible -> 127.0.0.1
        ] {
            assert!(rejects(u).await, "should reject {u}");
        }
    }

    #[tokio::test]
    async fn accepts_public_literal_and_returns_pinned_addr() {
        let url = url::Url::parse("https://1.1.1.1/blob").unwrap();
        let addrs = match super::BlossomAuth::validate_mirror_url(&url).await {
            Ok(a) => a,
            Err(_) => panic!("public literal must be accepted"),
        };
        // The caller pins these on the HTTP client; an empty set would silently
        // fall back to re-resolution.
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0].ip().to_string(), "1.1.1.1");
        assert_eq!(addrs[0].port(), 443);
    }
}
