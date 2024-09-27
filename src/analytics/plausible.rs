use crate::analytics::Analytics;
use crate::settings::Settings;
use anyhow::Error;
use log::{info, warn};
use rocket::Request;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

#[derive(Debug, Serialize, Deserialize)]
struct Event {
    pub name: String,
    pub domain: String,
    pub url: String,
    pub referrer: Option<String>,
    #[serde(skip_serializing)]
    pub user_agent: Option<String>,
    #[serde(skip_serializing)]
    pub xff: Option<String>,
}

pub struct PlausibleAnalytics {
    tx: UnboundedSender<Event>,
}

impl PlausibleAnalytics {
    pub fn new(settings: &Settings) -> Self {
        let (tx, mut rx) = unbounded_channel::<Event>();
        let url = match &settings.plausible_url {
            Some(s) => s.clone(),
            _ => "".to_string(),
        };
        let pub_url = settings.public_url.clone();
        tokio::spawn(async move {
            while let Some(mut msg) = rx.recv().await {
                msg.url = format!("{}{}", pub_url, msg.url);
                match ureq::post(&format!("{}/api/event", url))
                    .set(
                        "user-agent",
                        match &msg.user_agent {
                            Some(s) => s,
                            None => "",
                        },
                    )
                    .set(
                        "x-forwarded-for",
                        match &msg.xff {
                            Some(s) => s,
                            None => "",
                        },
                    )
                    .send_json(&msg)
                {
                    Ok(v) => info!("Sent {:?}", msg),
                    Err(e) => warn!("Failed to track: {}", e),
                }
            }
        });

        Self { tx }
    }
}

impl Analytics for PlausibleAnalytics {
    fn track(&self, req: &Request) -> Result<(), Error> {
        Ok(self.tx.send(Event {
            name: "pageview".to_string(),
            domain: match req.host() {
                Some(s) => s.to_string(),
                None => return Ok(()), // ignore request
            },
            url: req.uri().to_string(),
            referrer: match req.headers().get_one("Referer") {
                Some(s) => Some(s.to_string()),
                None => None,
            },
            user_agent: match req.headers().get_one("User-Agent") {
                Some(s) => Some(s.to_string()),
                None => None,
            },
            xff: match req.headers().get_one("X-Forwarded-For") {
                Some(s) => Some(s.to_string()),
                None => None,
            }
        })?)
    }
}
