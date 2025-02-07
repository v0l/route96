use crate::db::{Database, FileUpload};
use crate::filesystem::FileStore;
#[cfg(feature = "media-compression")]
use crate::processing::WebpProcessor;
pub use crate::routes::admin::admin_routes;
#[cfg(feature = "blossom")]
pub use crate::routes::blossom::blossom_routes;
#[cfg(feature = "nip96")]
pub use crate::routes::nip96::nip96_routes;
use crate::settings::Settings;
use crate::void_file::VoidFile;
use anyhow::Error;
use http_range_header::{parse_range_header, EndPosition, StartPosition};
use log::{debug, warn};
use nostr::Event;
use rocket::fs::NamedFile;
use rocket::http::{ContentType, Header, Status};
use rocket::response::Responder;
use rocket::serde::Serialize;
use rocket::{Request, Response, State};
use std::env::temp_dir;
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
    pub content: Option<String>,
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
        let ext = if upload.mime_type != "application/octet-stream" {
            mime2ext::mime2ext(&upload.mime_type)
        } else {
            None
        };
        let mut tags = vec![
            vec![
                "url".to_string(),
                format!("{}/{}.{}", &settings.public_url, &hex_id, ext.unwrap_or("")),
            ],
            vec!["x".to_string(), hex_id.clone()],
            vec!["m".to_string(), upload.mime_type.clone()],
            vec!["size".to_string(), upload.size.to_string()],
        ];
        if upload.mime_type.starts_with("image/") || upload.mime_type.starts_with("video/") {
            tags.push(vec![
                "thumb".to_string(),
                format!("{}/thumb/{}.webp", &settings.public_url, &hex_id),
            ]);
        }

        if let Some(bh) = &upload.blur_hash {
            tags.push(vec!["blurhash".to_string(), bh.clone()]);
        }
        if let (Some(w), Some(h)) = (upload.width, upload.height) {
            tags.push(vec!["dim".to_string(), format!("{}x{}", w, h)])
        }
        if let Some(d) = &upload.duration {
            tags.push(vec!["duration".to_string(), d.to_string()]);
        }
        if let Some(b) = &upload.bitrate {
            tags.push(vec!["bitrate".to_string(), b.to_string()]);
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
        response.set_header(Header::new("cache-control", "max-age=31536000, immutable"));

        // handle ranges
        #[cfg(feature = "ranges")]
        {
            const MAX_UNBOUNDED_RANGE: u64 = 1024 * 1024;
            // only use range response for files > 1MiB
            if self.info.size < MAX_UNBOUNDED_RANGE {
                response.set_sized_body(None, self.file);
            } else {
                response.set_header(Header::new("accept-ranges", "bytes"));
                if let Some(r) = request.headers().get("range").next() {
                    if let Ok(ranges) = parse_range_header(r) {
                        if ranges.ranges.len() > 1 {
                            warn!(
                                "Multipart ranges are not supported, fallback to non-range request"
                            );
                            response.set_streamed_body(self.file);
                        } else {
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
                                format!(
                                    "bytes {}-{}/{}",
                                    range_start,
                                    range_end - 1,
                                    self.info.size
                                ),
                            ));
                            response.set_streamed_body(Box::pin(r_body));
                        }
                    }
                } else {
                    response.set_sized_body(None, self.file);
                }
            }
        }
        #[cfg(not(feature = "ranges"))]
        {
            response.set_sized_body(None, self.file);
        }

        if let Ok(ct) = ContentType::from_str(&self.info.mime_type) {
            response.set_header(ct);
        }
        if let Some(name) = &self.info.name {
            response.set_header(Header::new(
                "content-disposition",
                format!("inline; filename=\"{}\"", name),
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

/// Generate thumbnail for image / video
#[cfg(feature = "media-compression")]
#[rocket::get("/thumb/<sha256>")]
pub async fn get_blob_thumb(
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
    let info = if let Ok(Some(info)) = db.get_file(&id).await {
        info
    } else {
        return Err(Status::NotFound);
    };

    if !(info.mime_type.starts_with("image/") || info.mime_type.starts_with("video/")) {
        return Err(Status::NotFound);
    }

    let file_path = fs.get(&id);

    let mut thumb_file = temp_dir().join(format!("thumb_{}", sha256));
    thumb_file.set_extension("webp");

    if !thumb_file.exists() {
        let mut p = WebpProcessor::new();
        if p.thumbnail(&file_path, &thumb_file).is_err() {
            return Err(Status::InternalServerError);
        }
    };

    if let Ok(f) = File::open(&thumb_file).await {
        Ok(FilePayload {
            file: f,
            info: FileUpload {
                size: thumb_file.metadata().unwrap().len(),
                mime_type: "image/webp".to_string(),
                ..info
            },
        })
    } else {
        Err(Status::NotFound)
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
        let uuid = if let Ok(b58) = nostr::bitcoin::base58::decode(id) {
            uuid::Uuid::from_slice_le(b58.as_slice())
        } else {
            uuid::Uuid::parse_str(id)
        };
        if uuid.is_err() {
            return None;
        }
        let f = base.join(VoidFile::map_to_path(&uuid.unwrap()));
        debug!("Legacy file map: {} => {}", id, f.display());
        if let Ok(f) = NamedFile::open(f).await {
            Some(f)
        } else {
            None
        }
    } else {
        None
    }
}

#[rocket::head("/d/<id>")]
pub async fn void_cat_redirect_head(id: &str) -> VoidCatFile {
    let id = if id.contains(".") {
        id.split('.').next().unwrap()
    } else {
        id
    };
    let uuid =
        uuid::Uuid::from_slice_le(nostr::bitcoin::base58::decode(id).unwrap().as_slice()).unwrap();
    VoidCatFile {
        status: Status::Ok,
        uuid: Header::new("X-UUID", uuid.to_string()),
    }
}

#[derive(Responder)]
pub struct VoidCatFile {
    pub status: Status,
    pub uuid: Header<'static>,
}
