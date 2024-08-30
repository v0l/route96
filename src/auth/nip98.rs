use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use log::info;
use nostr::{Event, JsonUtil, Kind, Timestamp};
use rocket::{async_trait, Request};
use rocket::http::Status;
use rocket::http::uri::{Absolute, Uri};
use rocket::request::{FromRequest, Outcome};

pub struct Nip98Auth {
    pub content_type: Option<String>,
    pub content_length: Option<usize>,
    pub event: Event,
}

#[async_trait]
impl<'r> FromRequest<'r> for Nip98Auth {
    type Error = &'static str;

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        return if let Some(auth) = request.headers().get_one("authorization") {
            if auth.starts_with("Nostr ") {
                let event = if let Ok(j) = BASE64_STANDARD.decode(&auth[6..]) {
                    if let Ok(ev) = Event::from_json(j) {
                        ev
                    } else {
                        return Outcome::Error((Status::new(403), "Invalid nostr event"));
                    }
                } else {
                    return Outcome::Error((Status::new(403), "Invalid auth string"));
                };

                if event.kind != Kind::HttpAuth {
                    return Outcome::Error((Status::new(401), "Wrong event kind"));
                }
                if event.created_at > Timestamp::now() {
                    return Outcome::Error((
                        Status::new(401),
                        "Created timestamp is in the future",
                    ));
                }

                // check url tag
                if let Some(url) = event.tags.iter().find_map(|t| {
                    let vec = t.as_vec();
                    if vec[0] == "u" {
                        Some(vec[1].clone())
                    } else {
                        None
                    }
                }) {
                    if let Ok(u_req) = Uri::parse::<Absolute>(&url) {
                        if request.uri().path() != u_req.absolute().unwrap().path() {
                            return Outcome::Error((Status::new(401), "U tag does not match"));
                        }
                    } else {
                        return Outcome::Error((Status::new(401), "Invalid U tag"));
                    }
                } else {
                    return Outcome::Error((Status::new(401), "Missing url tag"));
                }

                // check method tag
                if let Some(method) = event.tags.iter().find_map(|t| {
                    let vec = t.as_vec();
                    if vec[0] == "method" {
                        Some(vec[1].clone())
                    } else {
                        None
                    }
                }) {
                    if request.method().to_string() != *method {
                        return Outcome::Error((Status::new(401), "Method tag incorrect"));
                    }
                } else {
                    return Outcome::Error((Status::new(401), "Missing method tag"));
                }

                if let Err(_err) = event.verify() {
                    return Outcome::Error((Status::new(401), "Event signature invalid"));
                }

                info!("{}", event.as_json());
                Outcome::Success(Nip98Auth {
                    event,
                    content_type: request.headers().iter().find_map(|h| {
                        if h.name == "content-type" {
                            Some(h.value.to_string())
                        } else {
                            None
                        }
                    }),
                    content_length: request.headers().iter().find_map(|h| {
                        if h.name == "content-length" {
                            Some(h.value.parse().unwrap())
                        } else {
                            None
                        }
                    }),
                })
            } else {
                Outcome::Error((Status::new(403), "Auth scheme must be Nostr"))
            }
        } else {
            Outcome::Error((Status::new(403), "Auth header not found"))
        };
    }
}
