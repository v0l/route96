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

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<AppState>,
    ) -> Result<Self, Self::Rejection> {
        let auth = parts
            .headers
            .get("authorization")
            .ok_or(Nip98Rejection {
                status: StatusCode::FORBIDDEN,
                reason: "Auth header not found",
            })?
            .to_str()
            .map_err(|_| Nip98Rejection {
                status: StatusCode::FORBIDDEN,
                reason: "Invalid auth header",
            })?;

        if !auth.starts_with("Nostr ") {
            return Err(Nip98Rejection {
                status: StatusCode::FORBIDDEN,
                reason: "Auth scheme must be Nostr",
            });
        }

        let event = BASE64_STANDARD
            .decode(&auth[6..])
            .map_err(|_| Nip98Rejection {
                status: StatusCode::FORBIDDEN,
                reason: "Invalid auth string",
            })?;

        let event = Event::from_json(event).map_err(|_| Nip98Rejection {
            status: StatusCode::FORBIDDEN,
            reason: "Invalid nostr event",
        })?;

        if event.kind != Kind::HttpAuth {
            return Err(Nip98Rejection {
                status: StatusCode::UNAUTHORIZED,
                reason: "Wrong event kind",
            });
        }

        // Verify the signature and id hash before trusting any other event
        // field — `from_json` only deserializes, it does not validate.
        event.verify().map_err(|_| Nip98Rejection {
            status: StatusCode::UNAUTHORIZED,
            reason: "Event signature invalid",
        })?;

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
                reason: "U tag does not match public_url and request path, set public_url to the address clients use",
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

        debug!("nip98 auth ok: id={} pubkey={}", event.id, event.pubkey);

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
    use crate::db::Database;
    use crate::file_stats::FileStatsTracker;
    use crate::filesystem::FileStore;
    use crate::settings::Settings;
    use crate::whitelist::Whitelist;
    use axum::{Router, body::Body, routing::get};
    use nostr::{EventBuilder, Keys, Tag, TagKind};
    use sqlx::mysql::MySqlPoolOptions;
    use tokio::sync::RwLock;
    use tower::ServiceExt;

    fn test_state() -> (Arc<AppState>, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let settings = Settings {
            listen: None,
            storage_dir: temp.path().to_string_lossy().into_owned(),
            database: "mysql://user:pass@127.0.0.1:1/route96".to_string(),
            max_upload_bytes: 1024,
            public_url: "https://example.com".to_string(),
            whitelist: None,
            #[cfg(feature = "labels")]
            models_dir: None,
            #[cfg(feature = "labels")]
            label_models: None,
            #[cfg(feature = "labels")]
            label_flag_terms: None,
            webhook_url: None,
            reject_sensitive_exif: None,
            reject_steganography: None,
            #[cfg(feature = "media-compression")]
            identical_media_dedup: None,
            #[cfg(feature = "media-compression")]
            identical_media_dedup_distance: None,
            #[cfg(feature = "media-compression")]
            identical_media_dedup_allow_override: None,
            #[cfg(feature = "payments")]
            payments: None,
            delete_unaccessed_days: None,
            delete_after_days: None,
            delete_zero_egress_days: None,
            cold_storage_dir: None,
            cold_storage_min_downloads: None,
            cold_storage_after_days: None,
        };
        let settings = Arc::new(RwLock::new(settings));
        let pool = MySqlPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(50))
            .connect_lazy("mysql://user:pass@127.0.0.1:1/route96")
            .unwrap();
        let state = Arc::new(AppState {
            fs: FileStore::new(settings.clone()),
            db: Database::from_pool(pool),
            config_path: String::new(),
            settings,
            wl: Arc::new(RwLock::new(Whitelist::default())),
            file_stats: FileStatsTracker::new(),
            #[cfg(feature = "payments")]
            lnd: None,
        });
        (state, temp)
    }

    async fn handler(_auth: Nip98Auth) -> &'static str {
        "ok"
    }

    async fn status_for(
        u: &str,
        created_at: u64,
        expiration: Option<u64>,
        host: &str,
    ) -> StatusCode {
        let (state, _temp) = test_state();
        let keys = Keys::generate();
        let mut tags = vec![
            Tag::custom(TagKind::u(), [u.to_string()]),
            Tag::custom(TagKind::Method, ["GET"]),
        ];
        if let Some(e) = expiration {
            tags.push(Tag::custom(TagKind::Expiration, [e.to_string()]));
        }
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags(tags)
            .custom_created_at(Timestamp::from_secs(created_at))
            .sign_with_keys(&keys)
            .unwrap();
        let header = format!("Nostr {}", BASE64_STANDARD.encode(event.as_json()));
        let app = Router::new()
            .route("/admin/self", get(handler))
            .with_state(state);
        app.oneshot(
            axum::http::Request::builder()
                .uri("/admin/self")
                .header("authorization", header)
                .header("host", host)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
    }

    fn now() -> u64 {
        Timestamp::now().as_secs()
    }

    #[tokio::test]
    async fn extractor_checks_signature_before_timestamp() {
        let (state, _temp) = test_state();
        let event = EventBuilder::new(Kind::HttpAuth, "")
            .tags([
                Tag::custom(TagKind::u(), [OWN.to_string()]),
                Tag::custom(TagKind::Method, ["GET"]),
            ])
            .custom_created_at(Timestamp::from_secs(now() - 86_400))
            .sign_with_keys(&Keys::generate())
            .unwrap();
        let tampered = event
            .as_json()
            .replace("\"content\":\"\"", "\"content\":\"x\"");
        let app = Router::new()
            .route("/admin/self", get(handler))
            .with_state(state);
        let res = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/admin/self")
                    .header(
                        "authorization",
                        format!("Nostr {}", BASE64_STANDARD.encode(tampered)),
                    )
                    .header("host", "example.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(res.headers()["x-reason"], "Event signature invalid");
    }

    const OWN: &str = "https://example.com/admin/self";

    #[tokio::test]
    async fn extractor_accepts_valid_event() {
        assert_eq!(
            status_for(OWN, now(), None, "example.com").await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn extractor_rejects_u_tag_on_another_host() {
        assert_eq!(
            status_for(
                "https://other.example/admin/self",
                now(),
                None,
                "example.com"
            )
            .await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn extractor_rejects_forged_host_header_matching_u_tag() {
        assert_eq!(
            status_for(
                "https://other.example/admin/self",
                now(),
                None,
                "other.example"
            )
            .await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn extractor_rejects_61s_old_event_with_year_long_expiration() {
        let n = now();
        assert_eq!(
            status_for(OWN, n - 61, Some(n + 365 * 86400), "example.com").await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn extractor_rejects_30_day_old_event() {
        let n = now();
        assert_eq!(
            status_for(OWN, n - 30 * 86400, Some(n + 365 * 86400), "example.com").await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn extractor_accepts_query_string_on_u_tag() {
        assert_eq!(
            status_for(
                "https://example.com/admin/self?a=1",
                now(),
                None,
                "example.com"
            )
            .await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn extractor_accepts_uppercase_host_and_explicit_default_port() {
        assert_eq!(
            status_for(
                "https://Example.COM:443/admin/self",
                now(),
                None,
                "example.com"
            )
            .await,
            StatusCode::OK
        );
    }

    #[test]
    fn url_tag_for_other_host_is_rejected() {
        assert!(!url_matches(
            "https://other.example/admin/self",
            "https://example.com",
            "/admin/self"
        ));
    }

    #[test]
    fn url_tag_for_own_host_is_accepted() {
        assert!(url_matches(
            "https://example.com/admin/self",
            "https://example.com",
            "/admin/self"
        ));
        assert!(url_matches(
            "https://example.com/admin/self?x=1",
            "https://example.com/",
            "/admin/self"
        ));
    }

    #[test]
    fn url_tag_with_other_scheme_or_path_is_rejected() {
        assert!(!url_matches(
            "http://example.com/admin/self",
            "https://example.com",
            "/admin/self"
        ));
        assert!(!url_matches(
            "https://example.com/admin/other",
            "https://example.com",
            "/admin/self"
        ));
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
