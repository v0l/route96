use serde::{Deserialize, Serialize};

use crate::db::FileUpload;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct BlobDescriptor {
    pub url: String,
    pub sha256: String,
    pub size: u64,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    pub created: u64,
}

impl BlobDescriptor {
    pub fn from_upload(value: &FileUpload, public_url: &String) -> Self {
        let id_hex = hex::encode(&value.id);
        Self {
            url: format!("{}/{}", public_url, &id_hex),
            sha256: id_hex,
            size: value.size,
            mime_type: Some(value.mime_type.clone()),
            created: value.created.timestamp() as u64,
        }
    }
}
