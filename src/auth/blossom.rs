use axum::{
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use base64::prelude::*;
use log::info;
use nostr::{Event, JsonUtil, Kind, TagKind, Timestamp};

pub struct BlossomAuth {
    pub content_type: Option<String>,
    pub x_content_type: Option<String>,
    pub x_sha_256: Option<String>,
    pub x_content_length: Option<u64>,
    pub event: Event,
}

impl<S> FromRequestParts<S> for BlossomAuth
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth = parts
            .headers
            .get("authorization")
            .ok_or((StatusCode::UNAUTHORIZED, "Auth header not found"))?
            .to_str()
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid auth header"))?;

        if !auth.starts_with("Nostr ") {
            return Err((StatusCode::BAD_REQUEST, "Auth scheme must be Nostr"));
        }

        let event = BASE64_STANDARD
            .decode(&auth[6..])
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid auth string"))?;

        let event = Event::from_json(event)
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid nostr event"))?;

        if event.kind != Kind::Custom(24242) {
            return Err((StatusCode::BAD_REQUEST, "Wrong event kind"));
        }

        if (event.created_at.as_u64() as i64 - Timestamp::now().as_u64() as i64)
            .unsigned_abs()
            >= 60 * 3
        {
            return Err((StatusCode::BAD_REQUEST, "Created timestamp is out of range"));
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
                return Err((StatusCode::BAD_REQUEST, "Expiration invalid"));
            }
        } else {
            return Err((StatusCode::BAD_REQUEST, "Missing expiration tag"));
        }

        event
            .verify()
            .map_err(|_| (StatusCode::BAD_REQUEST, "Event signature invalid"))?;

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

        Ok(BlossomAuth {
            event,
            content_type,
            x_sha_256,
            x_content_length,
            x_content_type,
        })
    }
}
