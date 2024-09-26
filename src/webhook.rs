use anyhow::Error;
use serde::{Deserialize, Serialize};

use crate::filesystem::FileSystemResult;

pub struct Webhook {
    url: String,
}

#[derive(Serialize, Deserialize)]
struct WebhookRequest<T> {
    pub action: String,
    pub subject: Option<String>,
    pub payload: T,
}

impl Webhook {
    pub fn new(url: String) -> Self {
        Self {
            url
        }
    }

    /// Ask webhook api if this file can be accepted
    pub fn store_file(&self, pubkey: &Vec<u8>, fs: FileSystemResult) -> Result<bool, Error> {
        let body: WebhookRequest<FileSystemResult> = WebhookRequest {
            action: "store_file".to_string(),
            subject: Some(hex::encode(pubkey)),
            payload: fs,
        };
        let req = ureq::post(&self.url)
            .set("accept", "application/json")
            .send_json(body)?;

        if req.status() == 200 {
            Ok(true)
        } else {
            Ok(false)
        }
    }
}