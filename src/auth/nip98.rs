use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use log::info;
use nostr::{Event, JsonUtil, Kind, Timestamp};

pub struct Nip98Auth {
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub event: Event,
}

#[async_trait]
impl<S> FromRequestParts<S> for Nip98Auth
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let auth = parts
            .headers
            .get("authorization")
            .ok_or((StatusCode::FORBIDDEN, "Auth header not found"))?
            .to_str()
            .map_err(|_| (StatusCode::FORBIDDEN, "Invalid auth header"))?;

        if !auth.starts_with("Nostr ") {
            return Err((StatusCode::FORBIDDEN, "Auth scheme must be Nostr"));
        }

        let event = BASE64_STANDARD
            .decode(&auth[6..])
            .map_err(|_| (StatusCode::FORBIDDEN, "Invalid auth string"))?;

        let event = Event::from_json(event)
            .map_err(|_| (StatusCode::FORBIDDEN, "Invalid nostr event"))?;

        if event.kind != Kind::HttpAuth {
            return Err((StatusCode::UNAUTHORIZED, "Wrong event kind"));
        }

        if (event.created_at.as_u64() as i64 - Timestamp::now().as_u64() as i64)
            .unsigned_abs()
            >= 60 * 3
        {
            return Err((StatusCode::UNAUTHORIZED, "Created timestamp is out of range"));
        }

        // check url tag
        if let Some(url) = event.tags.iter().find_map(|t| {
            let vec = t.as_slice();
            if vec[0] == "u" {
                Some(vec[1].clone())
            } else {
                None
            }
        }) {
            let url_path = url.split('?').next().unwrap_or(&url);
            if parts.uri.path() != url_path && !url.ends_with(parts.uri.path()) {
                return Err((StatusCode::UNAUTHORIZED, "U tag does not match"));
            }
        } else {
            return Err((StatusCode::UNAUTHORIZED, "Missing url tag"));
        }

        // check method tag
        if let Some(method) = event.tags.iter().find_map(|t| {
            let vec = t.as_slice();
            if vec[0] == "method" {
                Some(vec[1].clone())
            } else {
                None
            }
        }) {
            if parts.method.as_str() != method {
                return Err((StatusCode::UNAUTHORIZED, "Method tag incorrect"));
            }
        } else {
            return Err((StatusCode::UNAUTHORIZED, "Missing method tag"));
        }

        event
            .verify()
            .map_err(|_| (StatusCode::UNAUTHORIZED, "Event signature invalid"))?;

        info!("{}", event.as_json());
        
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
