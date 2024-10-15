use anyhow::Error;
use reqwest::{Client, ClientBuilder};
use serde::{Deserialize, Serialize};

use crate::filesystem::FileSystemResult;

pub struct Webhook {
    url: String,
    client: Client,
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
            url,
            client: ClientBuilder::new().build().unwrap(),
        }
    }

    /// Ask webhook api if this file can be accepted
    pub async fn store_file(&self, pubkey: &Vec<u8>, fs: FileSystemResult) -> Result<bool, Error> {
        let body: WebhookRequest<FileSystemResult> = WebhookRequest {
            action: "store_file".to_string(),
            subject: Some(hex::encode(pubkey)),
            payload: fs,
        };
        let req = self
            .client
            .post(&self.url)
            .header("accept", "application/json")
            .json(&body)
            .send()
            .await?;

        if req.status() == 200 {
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
