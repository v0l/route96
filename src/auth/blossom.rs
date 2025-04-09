use base64::prelude::*;
use log::{debug, info, warn};
use nostr_sdk::nostr::{Event, JsonUtil, Kind, TagKind, Timestamp};
use rocket::http::Status;
use rocket::request::{FromRequest, Outcome};
use rocket::{async_trait, Request};

#[derive(Debug)]
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
        debug!("Attempting BlossomAuth from_request");
        if let Some(auth) = request.headers().get_one("authorization") {
            debug!(
                "Found Authorization header starting with: {:?}...",
                &auth.chars().take(10).collect::<String>()
            );
            if auth.starts_with("Nostr ") {
                debug!("Authorization header scheme is Nostr");
                let event = if let Ok(j) = BASE64_STANDARD.decode(&auth[6..]) {
                    if let Ok(ev) = Event::from_json(j) {
                        debug!("Successfully decoded and parsed Nostr event: {}", ev.id);
                        ev
                    } else {
                        warn!("Failed to parse JSON from base64 decoded auth string");
                        return Outcome::Error((Status::new(400), "Invalid nostr event"));
                    }
                } else {
                    warn!("Failed to base64 decode auth string: {}", &auth[6..]);
                    return Outcome::Error((Status::new(400), "Invalid auth string"));
                };

                if event.kind != Kind::Custom(24242) {
                    warn!("Auth event has wrong kind: {:?}", event.kind);
                    return Outcome::Error((Status::new(400), "Wrong event kind"));
                }
                if event.created_at > Timestamp::now() {
                    warn!(
                        "Auth event created_at is in the future: {}",
                        event.created_at
                    );
                    return Outcome::Error((
                        Status::new(400),
                        "Created timestamp is in the future",
                    ));
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
                    debug!(
                        "Auth event expiration timestamp: {}, Current time: {}",
                        u_exp,
                        Timestamp::now()
                    );
                    if u_exp <= Timestamp::now() {
                        warn!("Auth event has expired: {} <= {}", u_exp, Timestamp::now());
                        return Outcome::Error((Status::new(400), "Expiration invalid"));
                    }
                } else {
                    warn!("Auth event missing expiration tag");
                    return Outcome::Error((Status::new(400), "Missing expiration tag"));
                }

                if let Err(e) = event.verify() {
                    warn!("Auth event signature verification failed: {}", e);
                    return Outcome::Error((Status::new(400), "Event signature invalid"));
                }

                info!("Successful BlossomAuth validation for event: {}", event.id);
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
                warn!("Authorization header scheme is not Nostr: {}", auth);
                Outcome::Error((Status::new(400), "Auth scheme must be Nostr"))
            }
        } else {
            warn!("Authorization header not found in request");
            Outcome::Error((Status::new(401), "Auth header not found"))
        }
    }
}
