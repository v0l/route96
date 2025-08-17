use base64::prelude::*;
use log::info;
use nostr::{Event, JsonUtil, Kind, TagKind, Timestamp};
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome};
use rocket::{async_trait, Request};

pub struct BlossomAuth {
    pub content_type: Option<String>,
    pub x_content_type: Option<String>,
    pub x_sha_256: Option<String>,
    pub x_content_length: Option<u64>,
    pub event: Event,
}

#[async_trait]
impl<'r> FromRequest<'r> for BlossomAuth {
    type Error = &'static str;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        if let Some(auth) = request.headers().get_one("authorization") {
            if auth.starts_with("Nostr ") {
                let event = if let Ok(j) = BASE64_STANDARD.decode(&auth[6..]) {
                    if let Ok(ev) = Event::from_json(j) {
                        ev
                    } else {
                        return Outcome::Error((Status::new(400), "Invalid nostr event"));
                    }
                } else {
                    return Outcome::Error((Status::new(400), "Invalid auth string"));
                };

                if event.kind != Kind::Custom(24242) {
                    return Outcome::Error((Status::new(400), "Wrong event kind"));
                }
                if (event.created_at.as_u64() as i64 - Timestamp::now().as_u64() as i64)
                    .unsigned_abs()
                    >= 60 * 3
                {
                    return Outcome::Error((Status::new(400), "Created timestamp is out of range"));
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
                        return Outcome::Error((Status::new(400), "Expiration invalid"));
                    }
                } else {
                    return Outcome::Error((Status::new(400), "Missing expiration tag"));
                }

                if event.verify().is_err() {
                    return Outcome::Error((Status::new(400), "Event signature invalid"));
                }

                info!("{}", event.as_json());
                Outcome::Success(BlossomAuth {
                    event,
                    content_type: request.headers().iter().find_map(|h| {
                        if h.name == "content-type" {
                            Some(h.value.to_string())
                        } else {
                            None
                        }
                    }),
                    x_sha_256: request.headers().iter().find_map(|h| {
                        if h.name == "x-sha-256" {
                            Some(h.value.to_string())
                        } else {
                            None
                        }
                    }),
                    x_content_length: request.headers().iter().find_map(|h| {
                        if h.name == "x-content-length" {
                            Some(h.value.parse().unwrap())
                        } else {
                            None
                        }
                    }),
                    x_content_type: request.headers().iter().find_map(|h| {
                        if h.name == "x-content-type" {
                            Some(h.value.to_string())
                        } else {
                            None
                        }
                    }),
                })
            } else {
                Outcome::Error((Status::new(400), "Auth scheme must be Nostr"))
            }
        } else {
            Outcome::Error((Status::new(401), "Auth header not found"))
        }
    }
}
