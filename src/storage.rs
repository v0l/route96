use anyhow::Result;
use async_trait::async_trait;
use std::any::Any;
use tokio::io::AsyncRead;

/// Result type for storage put operations.
#[derive(Debug, Clone)]
pub enum StorageResult {
    /// Blob with the given hash already exists.
    AlreadyExists(Vec<u8>),
    /// A new blob was successfully stored.
    NewFile {
        id: Vec<u8>,
        size: u64,
        mime_type: String,
        // Consider adding other metadata derived during upload if needed
    },
}

/// Metadata associated with a stored blob.
#[derive(Debug, Clone)]
pub struct BlobMetadata {
    pub size: u64,
    pub mime_type: String,
    // Add other fields returned by HEAD if needed (e.g., last modified?)
}

// Represents a byte range for HTTP Range requests.
#[derive(Debug, Clone, Copy)]
pub struct HttpRange {
    // Optional starting byte (inclusive)
    pub start: Option<u64>,
    // Optional ending byte (inclusive)
    pub end: Option<u64>,
}

impl HttpRange {
    /// Formats the range specification string for S3 GetObject's `range` parameter.
    /// Follows HTTP Range header format (e.g., "bytes=0-1023", "bytes=500-").
    pub fn format_for_s3(&self) -> String {
        match (self.start, self.end) {
            (Some(start), Some(end)) => format!("bytes={}-{}", start, end), // e.g., bytes=0-499
            (Some(start), None) => format!("bytes={}-", start),             // e.g., bytes=500-
            // S3 GetObject `range` parameter primarily uses start-based ranges.
            // While HTTP allows `bytes=-suffix`, S3 API prefers `bytes=start-end`.
            // Handling `(None, Some(end))` (meaning last 'end+1' bytes) would require
            // knowing the total object size first. We treat it as invalid for direct S3 format.
            (None, Some(end_pos)) => {
                log::warn!(
                    "HttpRange with end ({}) but no start is not directly mappable to S3 range parameter format. Treating as bytes=0-{}.",
                    end_pos,
                    end_pos
                 );
                // Defaulting to start=0 might be unexpected. Consider erroring or requiring pre-computation.
                format!("bytes=0-{}", end_pos)
            }
            (None, None) => {
                // An empty range implies fetching the whole object.
                // This method shouldn't typically be called in that case,
                // as the absence of the range option is used.
                log::error!("format_for_s3 called on an empty HttpRange (None, None)");
                "".to_string() // Return empty string; S3 might ignore or error.
            }
        }
    }
}

/// Defines the interface for a storage backend capable of storing and retrieving blobs by hash.
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// Stores a blob from an asynchronous reader stream.
    ///
    /// Calculates the sha256 hash of the stream content and stores the blob
    /// using the hash as its identifier. If a blob with the same hash already
    /// exists, it should not be stored again.
    ///
    /// # Arguments
    ///
    /// * `stream` - An asynchronous reader providing the blob data.
    /// * `mime_type` - The declared MIME type of the blob.
    ///
    /// # Returns
    ///
    /// * `Ok(StorageResult::NewFile { .. })` if the blob was new and stored successfully.
    /// * `Ok(StorageResult::AlreadyExists(id))` if a blob with the same hash already exists.
    /// * `Err(_)` if an error occurred during hashing, IO, or storage.
    async fn put(
        &self,
        stream: Box<dyn AsyncRead + Send + Unpin>,
        mime_type: &str,
    ) -> Result<StorageResult>;

    /// Retrieves metadata for a blob identified by its hash.
    ///
    /// # Arguments
    ///
    /// * `id` - The sha256 hash of the blob.
    ///
    /// # Returns
    ///
    /// * `Ok(BlobMetadata)` if the blob exists.
    /// * `Err(_)` if the blob does not exist or an error occurred.
    async fn head(&self, id: &[u8]) -> Result<BlobMetadata>;

    /// Generates a publicly accessible URL for a blob.
    ///
    /// The nature of the URL depends on the backend (e.g., a direct link,
    /// a pre-signed URL, a server-relative path).
    ///
    /// # Arguments
    ///
    /// * `id` - The sha256 hash of the blob.
    /// * `preferred_extension` - An optional hint for the file extension to include in the URL.
    ///
    /// # Returns
    ///
    /// * `Ok(String)` containing the URL.
    /// * `Err(_)` if the blob does not exist or an error occurred.
    async fn get_url(&self, id: &[u8], preferred_extension: Option<&str>) -> Result<String>;

    /// Provides an asynchronous reader for accessing the content of a blob.
    ///
    /// Optionally supports range requests.
    ///
    /// # Arguments
    ///
    /// * `id` - The sha256 hash of the blob.
    /// * `range` - An optional range specifying the portion of the blob to read.
    ///
    /// # Returns
    ///
    /// * `Ok(Box<dyn AsyncRead + Send + Unpin>)` providing the blob content.
    /// * `Err(_)` if the blob does not exist or an error occurred.
    async fn stream_reader(
        &self,
        id: &[u8],
        range: Option<HttpRange>,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin>>;

    /// Deletes a blob identified by its hash.
    ///
    /// # Arguments
    ///
    /// * `id` - The sha256 hash of the blob.
    ///
    /// # Returns
    ///
    /// * `Ok(())` if the blob was deleted successfully or did not exist.
    /// * `Err(_)` if an error occurred during deletion.
    async fn delete(&self, id: &[u8]) -> Result<()>;

    // Add method for downcasting
    fn as_any(&self) -> &dyn Any;
}
