use crate::db::{Database, FileUpload};
use crate::filesystem::FileStore;
pub use crate::routes::admin::admin_routes;
#[cfg(feature = "blossom")]
pub use crate::routes::blossom::blossom_routes;
#[cfg(feature = "nip96")]
pub use crate::routes::nip96::nip96_routes;
use crate::settings::Settings;
use crate::void_file::VoidFile;
use anyhow::Error;
use http_range_header::{parse_range_header, EndPosition, StartPosition};
use log::warn;
use nostr::Event;
use rocket::fs::NamedFile;
use rocket::http::{ContentType, Header, Status};
use rocket::response::Responder;
use rocket::serde::Serialize;
use rocket::{Request, Response, State};
use std::io::SeekFrom;
use std::ops::Range;
use std::pin::{pin, Pin};
use std::str::FromStr;
use std::task::{Context, Poll};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncSeek, ReadBuf};

#[cfg(feature = "blossom")]
mod blossom;
#[cfg(feature = "nip96")]
mod nip96;

mod admin;

pub struct FilePayload {
    pub file: File,
    pub info: FileUpload,
}

#[derive(Clone, Debug, Serialize, Default)]
#[serde(crate = "rocket::serde")]
struct Nip94Event {
    pub created_at: i64,
    pub content: String,
    pub tags: Vec<Vec<String>>,
}

#[derive(Serialize, Default)]
#[serde(crate = "rocket::serde")]
struct PagedResult<T> {
    pub count: u32,
    pub page: u32,
    pub total: u32,
    pub files: Vec<T>,
}

impl Nip94Event {
    pub fn from_upload(settings: &Settings, upload: &FileUpload) -> Self {
        let hex_id = hex::encode(&upload.id);
        let mut tags = vec![
            vec![
                "url".to_string(),
                format!(
                    "{}/{}{}",
                    &settings.public_url,
                    &hex_id,
                    mime2ext::mime2ext(&upload.mime_type)
                        .map(|m| format!(".{m}"))
                        .unwrap_or("".to_string())
                ),
            ],
            vec!["x".to_string(), hex_id],
            vec!["m".to_string(), upload.mime_type.clone()],
            vec!["size".to_string(), upload.size.to_string()],
        ];
        if let Some(bh) = &upload.blur_hash {
            tags.push(vec!["blurhash".to_string(), bh.clone()]);
        }
        if let (Some(w), Some(h)) = (upload.width, upload.height) {
            tags.push(vec!["dim".to_string(), format!("{}x{}", w, h)])
        }
        #[cfg(feature = "labels")]
        for l in &upload.labels {
            let val = if l.label.contains(',') {
                let split_val: Vec<&str> = l.label.split(',').collect();
                split_val[0].to_string()
            } else {
                l.label.clone()
            };
            tags.push(vec!["t".to_string(), val])
        }

        Self {
            content: upload.name.clone(),
            created_at: upload.created.timestamp(),
            tags,
        }
    }
}

/// Range request handler over file handle
struct RangeBody {
    file: File,
    range_start: u64,
    range_end: u64,
    current_offset: u64,
    poll_complete: bool,
}

impl RangeBody {
    pub fn new(file: File, range: Range<u64>) -> Self {
        Self {
            file,
            range_start: range.start,
            range_end: range.end,
            current_offset: 0,
            poll_complete: false,
        }
    }
}

impl AsyncRead for RangeBody {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let range_start = self.range_start + self.current_offset;
        let range_len = self.range_end - range_start;
        let bytes_to_read = buf.remaining().min(range_len as usize) as u64;

        if bytes_to_read == 0 {
            return Poll::Ready(Ok(()));
        }

        // when no pending poll, seek to starting position
        if !self.poll_complete {
            let pinned = pin!(&mut self.file);
            pinned.start_seek(SeekFrom::Start(range_start))?;
            self.poll_complete = true;
        }

        // check poll completion
        if self.poll_complete {
            let pinned = pin!(&mut self.file);
            match pinned.poll_complete(cx) {
                Poll::Ready(Ok(_)) => {
                    self.poll_complete = false;
                }
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }

        // Read data from the file
        let pinned = pin!(&mut self.file);
        match pinned.poll_read(cx, buf) {
            Poll::Ready(Ok(_)) => {
                self.current_offset += bytes_to_read;
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => {
                self.poll_complete = true;
                Poll::Pending
            }
        }
    }
}

impl<'r> Responder<'r, 'static> for FilePayload {
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        let mut response = Response::new();

        // handle ranges
        #[cfg(feature = "ranges")]
        {
            response.set_header(Header::new("accept-ranges", "bytes"));
            if let Some(r) = request.headers().get("range").next() {
                if let Ok(ranges) = parse_range_header(r) {
                    if ranges.ranges.len() > 1 {
                        warn!("Multipart ranges are not supported, fallback to non-range request");
                        response.set_streamed_body(self.file);
                    } else {
                        const MAX_UNBOUNDED_RANGE: u64 = 1024 * 1024;
                        let single_range = ranges.ranges.first().unwrap();
                        let range_start = match single_range.start {
                            StartPosition::Index(i) => i,
                            StartPosition::FromLast(i) => self.info.size - i,
                        };
                        let range_end = match single_range.end {
                            EndPosition::Index(i) => i,
                            EndPosition::LastByte => {
                                (range_start + MAX_UNBOUNDED_RANGE).min(self.info.size)
                            }
                        };
                        let r_len = range_end - range_start;
                        let r_body = RangeBody::new(self.file, range_start..range_end);

                        response.set_status(Status::PartialContent);
                        response.set_header(Header::new("content-length", r_len.to_string()));
                        response.set_header(Header::new(
                            "content-range",
                            format!("bytes {}-{}/{}", range_start, range_end - 1, self.info.size),
                        ));
                        response.set_streamed_body(Box::pin(r_body));
                    }
                }
            } else {
                response.set_streamed_body(self.file);
            }
        }
        #[cfg(not(feature = "ranges"))]
        {
            response.set_streamed_body(self.file);
            response.set_header(Header::new("content-length", self.info.size.to_string()));
        }

        if let Ok(ct) = ContentType::from_str(&self.info.mime_type) {
            response.set_header(ct);
        }
        if !self.info.name.is_empty() {
            response.set_header(Header::new(
                "content-disposition",
                format!("inline; filename=\"{}\"", self.info.name),
            ));
        }
        Ok(response)
    }
}

async fn delete_file(
    sha256: &str,
    auth: &Event,
    fs: &FileStore,
    db: &Database,
) -> Result<(), Error> {
    let sha256 = if sha256.contains(".") {
        sha256.split('.').next().unwrap()
    } else {
        sha256
    };
    let id = if let Ok(i) = hex::decode(sha256) {
        i
    } else {
        return Err(Error::msg("Invalid file id"));
    };

    if id.len() != 32 {
        return Err(Error::msg("Invalid file id"));
    }
    if let Ok(Some(_info)) = db.get_file(&id).await {
        let pubkey_vec = auth.pubkey.to_bytes().to_vec();
        let auth_user = db.get_user(&pubkey_vec).await?;
        let owners = db.get_file_owners(&id).await?;
        if auth_user.is_admin {
            if let Err(e) = db.delete_all_file_owner(&id).await {
                return Err(Error::msg(format!("Failed to delete (db): {}", e)));
            }
            if let Err(e) = db.delete_file(&id).await {
                return Err(Error::msg(format!("Failed to delete (fs): {}", e)));
            }
            if let Err(e) = tokio::fs::remove_file(fs.get(&id)).await {
                warn!("Failed to delete (fs): {}", e);
            }
        } else {
            let this_owner = match owners.iter().find(|o| o.pubkey.eq(&pubkey_vec)) {
                Some(o) => o,
                None => return Err(Error::msg("You dont own this file, you cannot delete it")),
            };
            if let Err(e) = db.delete_file_owner(&id, this_owner.id).await {
                return Err(Error::msg(format!("Failed to delete (db): {}", e)));
            }
            // only 1 owner was left, delete file completely
            if owners.len() == 1 {
                if let Err(e) = db.delete_file(&id).await {
                    return Err(Error::msg(format!("Failed to delete (fs): {}", e)));
                }
                if let Err(e) = tokio::fs::remove_file(fs.get(&id)).await {
                    warn!("Failed to delete (fs): {}", e);
                }
            }
        }
        Ok(())
    } else {
        Err(Error::msg("File not found"))
    }
}

#[rocket::get("/")]
pub async fn root() -> Result<NamedFile, Status> {
    #[cfg(all(debug_assertions, feature = "react-ui"))]
    let index = "./ui_src/dist/index.html";
    #[cfg(all(not(debug_assertions), feature = "react-ui"))]
    let index = "./ui/index.html";
    #[cfg(not(feature = "react-ui"))]
    let index = "./index.html";
    if let Ok(f) = NamedFile::open(index).await {
        Ok(f)
    } else {
        Err(Status::InternalServerError)
    }
}

#[rocket::get("/<sha256>")]
pub async fn get_blob(
    sha256: &str,
    fs: &State<FileStore>,
    db: &State<Database>,
) -> Result<FilePayload, Status> {
    let sha256 = if sha256.contains(".") {
        sha256.split('.').next().unwrap()
    } else {
        sha256
    };
    let id = if let Ok(i) = hex::decode(sha256) {
        i
    } else {
        return Err(Status::NotFound);
    };

    if id.len() != 32 {
        return Err(Status::NotFound);
    }
    if let Ok(Some(info)) = db.get_file(&id).await {
        if let Ok(f) = File::open(fs.get(&id)).await {
            return Ok(FilePayload { file: f, info });
        }
    }
    Err(Status::NotFound)
}

#[rocket::head("/<sha256>")]
pub async fn head_blob(sha256: &str, fs: &State<FileStore>) -> Status {
    let sha256 = if sha256.contains(".") {
        sha256.split('.').next().unwrap()
    } else {
        sha256
    };
    let id = if let Ok(i) = hex::decode(sha256) {
        i
    } else {
        return Status::NotFound;
    };

    if id.len() != 32 {
        return Status::NotFound;
    }
    if fs.get(&id).exists() {
        Status::Ok
    } else {
        Status::NotFound
    }
}

/// Legacy URL redirect for void.cat uploads
#[rocket::get("/d/<id>")]
pub async fn void_cat_redirect(id: &str, settings: &State<Settings>) -> Option<NamedFile> {
    let id = if id.contains(".") {
        id.split('.').next().unwrap()
    } else {
        id
    };
    if let Some(base) = &settings.void_cat_files {
        let uuid =
            uuid::Uuid::from_slice_le(nostr::bitcoin::base58::decode(id).unwrap().as_slice())
                .unwrap();
        let f = base.join(VoidFile::map_to_path(&uuid));
        if let Ok(f) = NamedFile::open(f).await {
            Some(f)
        } else {
            None
        }
    } else {
        None
    }
}
