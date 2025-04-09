#![cfg(feature = "s3-storage")]

use crate::settings::S3StorageSettings;
use crate::storage::{BlobMetadata, HttpRange, StorageBackend, StorageResult};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use aws_config::meta::region::RegionProviderChain;
use aws_config::BehaviorVersion;
use aws_sdk_s3::config::{Credentials, Region};
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use hex;
use log;
use sha2::{Digest, Sha256};
use std::any::Any;
use std::env; // Added for temp_dir
use std::path::PathBuf;
use std::time::Duration; // For presigned URL expiration
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use uuid::Uuid; // For specific error types // For presigning

#[derive(Clone)]
pub struct SpacesStore {
    client: Client,
    bucket_name: String,
    temp_dir: PathBuf, // For temporary storage during upload hashing
    presign_duration: Duration,
}

impl SpacesStore {
    pub async fn new(settings: &S3StorageSettings) -> Result<Self> {
        // Configure region provider
        let region_provider =
            RegionProviderChain::first_try(settings.region.clone().map(Region::new))
                .or_default_provider()
                .or_else(Region::new("us-east-1")); // Default region if not found

        // Load shared configuration
        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .region(region_provider)
            .credentials_provider(Credentials::new(
                &settings.access_key_id,
                &settings.secret_access_key,
                None,     // session_token
                None,     // expiry
                "Static", // provider_name
            ))
            .load()
            .await;

        // Build S3 config from the shared SDK config
        let mut s3_config_builder = aws_sdk_s3::config::Builder::from(&sdk_config);

        // Override endpoint if provided
        if let Some(endpoint) = &settings.endpoint_url {
            s3_config_builder = s3_config_builder.endpoint_url(endpoint.as_str());
        }

        // Set force_path_style
        s3_config_builder = s3_config_builder.force_path_style(settings.force_path_style);

        let s3_config = s3_config_builder.build();

        let client = Client::from_conf(s3_config);

        // Default presign duration (e.g., 15 minutes)
        // TODO: Make this configurable in settings
        let presign_duration = Duration::from_secs(15 * 60);

        // Get temp_dir from environment
        let temp_dir = env::temp_dir();
        tokio::fs::create_dir_all(&temp_dir)
            .await
            .context(format!("Failed to create temp directory: {:?}", temp_dir))?;
        log::info!("S3 backend using temporary directory: {:?}", temp_dir);

        Ok(Self {
            client,
            bucket_name: settings.bucket_name.clone(),
            temp_dir,
            presign_duration, // Initialize the duration
        })
    }

    // Helper to store stream to temp file and calculate hash (similar to FileStore)
    async fn store_and_hash_temp_boxed(
        &self,
        mut stream: Box<dyn AsyncRead + Send + Unpin>,
    ) -> Result<(PathBuf, u64, Vec<u8>)> {
        let uid = Uuid::new_v4();
        let temp_path = self.temp_dir.join(uid.to_string());
        tokio::fs::create_dir_all(&self.temp_dir).await?; // Ensure temp dir exists

        let mut file = File::create(&temp_path)
            .await
            .context(format!("Failed to create temp file at {:?}", temp_path))?;
        let mut hasher = Sha256::new();
        let mut total_bytes = 0u64;
        let mut buf = [0; 8192]; // Use 8KB buffer

        loop {
            let n = stream
                .read(&mut buf)
                .await
                .context("Failed to read from input stream")?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .await
                .context("Failed to write to temp file")?;
            hasher.update(&buf[..n]);
            total_bytes += n as u64;
        }

        let hash = hasher.finalize().to_vec();
        file.flush().await.context("Failed to flush temp file")?;
        drop(file); // Close file handle explicitly
        Ok((temp_path, total_bytes, hash))
    }
}

#[async_trait]
impl StorageBackend for SpacesStore {
    async fn put(
        &self,
        stream: Box<dyn AsyncRead + Send + Unpin>,
        mime_type: &str,
    ) -> Result<StorageResult> {
        // 1. Store stream to temp file & calculate hash (use helper)
        let (temp_path, size, hash) = self.store_and_hash_temp_boxed(stream).await?;

        // Ensure the temporary file is removed eventually
        // Use a guard or similar pattern in real code, but for simplicity:
        let _cleanup = TempFileCleanup(&temp_path); // Basic RAII cleanup guard

        let key = hex::encode(&hash);
        log::debug!("Calculated hash {} for S3 object key", key);

        // 2. Check if object with hash exists using HEAD
        match self
            .client
            .head_object()
            .bucket(&self.bucket_name)
            .key(&key)
            .send()
            .await
        {
            Ok(_) => {
                // 3. If exists, delete temp file (handled by cleanup) and return AlreadyExists
                log::debug!(
                    "Object {} already exists in bucket {}",
                    key,
                    self.bucket_name
                );
                // Note: Temp file is deleted automatically when _cleanup goes out of scope here
                return Ok(StorageResult::AlreadyExists(hash));
            }
            Err(e) => {
                // Check if the error is NotFound using ProvideErrorMetadata trait
                if let Some(aws_err) = e.as_service_error() {
                    if aws_err.is_not_found() {
                        log::debug!("Object {} not found, proceeding with upload", key);
                        // Continue to upload logic below
                    } else {
                        // Other AWS error, clean up temp file and return error
                        // Note: Temp file is deleted automatically when _cleanup goes out of scope here
                        return Err(anyhow::Error::new(e).context(format!(
                            "Failed to check S3 object existence for key {}",
                            key
                        )));
                    }
                } else {
                    // Not an AWS service error (e.g., network error), clean up and return
                    // Note: Temp file is deleted automatically when _cleanup goes out of scope here
                    return Err(anyhow::Error::new(e)
                        .context(format!("Error during S3 head_object call for key {}", key)));
                }
            }
        }

        // 4. If not exists, upload temp file using PutObject
        log::debug!("Uploading object {} to bucket {}", key, self.bucket_name);
        let body = ByteStream::from_path(&temp_path).await.context(format!(
            "Failed to create ByteStream from temp file {:?}",
            temp_path
        ))?;

        let put_object_output = self
            .client
            .put_object()
            .bucket(&self.bucket_name)
            .key(&key)
            .content_type(mime_type) // Use the declared mime type
            .body(body)
            .send()
            .await;

        // 5. Delete temp file (handled by cleanup guard)
        // Drop the guard explicitly *after* the upload attempt to ensure deletion happens
        // If the upload fails, the guard still cleans up when the function returns Err.
        // If successful, we drop it here.
        drop(_cleanup);

        match put_object_output {
            Ok(_) => {
                log::debug!("Successfully uploaded object {}", key);
                // 6. Return NewFile
                Ok(StorageResult::NewFile {
                    id: hash,
                    size,
                    mime_type: mime_type.to_string(), // Return the declared mime type
                })
            }
            Err(e) => {
                // Upload failed, temp file was already deleted by the guard dropping.
                Err(anyhow::Error::new(e).context(format!("Failed to upload object {} to S3", key)))
            }
        }
    }

    async fn head(&self, id: &[u8]) -> Result<BlobMetadata> {
        let key = hex::encode(id);
        log::debug!(
            "Requesting HEAD for object key: {} in bucket {}",
            key,
            self.bucket_name
        );

        match self
            .client
            .head_object()
            .bucket(&self.bucket_name)
            .key(&key)
            .send()
            .await
        {
            Ok(output) => {
                let size = output
                    .content_length()
                    .map(|cl| cl as u64) // S3 uses i64 for content_length
                    .unwrap_or(0); // Default to 0 if not present? Or error? Let's default for now.
                let mime_type = output
                    .content_type()
                    .map(String::from)
                    .unwrap_or_else(|| "application/octet-stream".to_string()); // Default mime type

                log::debug!(
                    "HEAD successful for key {}: size={}, mime_type={}",
                    key,
                    size,
                    mime_type
                );
                Ok(BlobMetadata { size, mime_type })
            }
            Err(e) => {
                // Check if the error is NotFound
                if let Some(aws_err) = e.as_service_error() {
                    if aws_err.is_not_found() {
                        log::warn!("HEAD failed for key {}: Object not found", key);
                        Err(anyhow!("Blob not found: {}", key)) // Specific error for not found
                    } else {
                        log::error!("AWS SDK error during HEAD for key {}: {}", key, aws_err);
                        Err(anyhow::Error::new(e)
                            .context(format!("Failed S3 HEAD request for key {}", key)))
                    }
                } else {
                    log::error!("Non-SDK error during HEAD for key {}: {}", key, e);
                    Err(anyhow::Error::new(e)
                        .context(format!("Error during S3 head_object call for key {}", key)))
                }
            }
        }
    }

    async fn get_url(&self, id: &[u8], _preferred_extension: Option<&str>) -> Result<String> {
        let key = hex::encode(id);
        log::debug!(
            "Generating presigned URL for object key: {} in bucket {}",
            key,
            self.bucket_name
        );

        // Note: _preferred_extension is ignored for S3 presigned URLs as the content-type
        // is determined by the object itself or how the client handles the URL.

        match PresigningConfig::expires_in(self.presign_duration) {
            Ok(presigning_config) => {
                let presigned_request = self
                    .client
                    .get_object()
                    .bucket(&self.bucket_name)
                    .key(&key)
                    .presigned(presigning_config) // Pass the config here
                    .await
                    .context(format!("Failed to generate presigned URL for key {}", key))?;

                log::debug!("Successfully generated presigned URL for key {}", key);
                Ok(presigned_request.uri().to_string())
            }
            Err(e) => {
                // This error comes from building the PresigningConfig (e.g., invalid duration)
                log::error!("Failed to create PresigningConfig: {}", e);
                Err(anyhow::Error::new(e).context("Failed to create presigning configuration"))
            }
        }
    }

    async fn stream_reader(
        &self,
        id: &[u8],
        range: Option<HttpRange>,
    ) -> Result<Box<dyn AsyncRead + Send + Unpin>> {
        let key = hex::encode(id);
        log::debug!(
            "Requesting stream for object key: {} in bucket {}",
            key,
            self.bucket_name
        );

        let mut request_builder = self.client.get_object().bucket(&self.bucket_name).key(&key);

        // Implement range request support for S3 GetObject.
        if let Some(http_range) = range {
            // Format the range string according to HTTP Range header spec (e.g., "bytes=0-1023")
            let range_str = http_range.format_for_s3();
            log::debug!("Applying S3 range request: {} for key {}", range_str, key);
            request_builder = request_builder.range(range_str);
        } else {
            log::debug!(
                "No range specified for key {}, fetching entire object.",
                key
            );
        }

        match request_builder.send().await {
            Ok(output) => {
                log::debug!("GetObject successful for key {}, returning stream.", key);
                // Convert the S3 ByteStream into something that implements AsyncRead
                let async_read_stream = output.body.into_async_read();
                Ok(Box::new(async_read_stream) as Box<dyn AsyncRead + Send + Unpin>)
            }
            Err(e) => {
                if let Some(aws_err) = e.as_service_error() {
                    // S3 uses NoSuchKey for GetObject not found, map it for consistency
                    if aws_err.code() == Some("NoSuchKey") {
                        log::warn!(
                            "GetObject failed for key {}: Object not found (NoSuchKey)",
                            key
                        );
                        Err(anyhow!("Blob not found: {}", key))
                    } else {
                        log::error!(
                            "AWS SDK error during GetObject for key {}: {}",
                            key,
                            aws_err
                        );
                        Err(anyhow::Error::new(e)
                            .context(format!("Failed S3 GetObject request for key {}", key)))
                    }
                } else {
                    log::error!("Non-SDK error during GetObject for key {}: {}", key, e);
                    Err(anyhow::Error::new(e)
                        .context(format!("Error during S3 GetObject call for key {}", key)))
                }
            }
        }
    }

    async fn delete(&self, id: &[u8]) -> Result<()> {
        let key = hex::encode(id);
        log::debug!(
            "Requesting deletion of object key: {} from bucket {}",
            key,
            self.bucket_name
        );

        match self
            .client
            .delete_object()
            .bucket(&self.bucket_name)
            .key(&key)
            .send()
            .await
        {
            Ok(_) => {
                // DeleteObject returns success even if the key doesn't exist.
                log::debug!(
                    "DeleteObject successful for key {} (or key did not exist).",
                    key
                );
                Ok(())
            }
            Err(e) => {
                // An error here indicates a problem with the request itself (permissions, network, etc.)
                log::error!("Error during DeleteObject call for key {}: {}", key, e);
                // Check if it's a service error we can log more details about
                if let Some(aws_err) = e.as_service_error() {
                    log::error!(
                        "AWS SDK error during DeleteObject for key {}: {}",
                        key,
                        aws_err
                    );
                }
                Err(anyhow::Error::new(e)
                    .context(format!("Failed S3 DeleteObject request for key {}", key)))
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Helper struct for basic RAII cleanup of the temporary file
struct TempFileCleanup<'a>(&'a PathBuf);

impl<'a> Drop for TempFileCleanup<'a> {
    fn drop(&mut self) {
        let path = self.0;
        if path.exists() {
            log::debug!("Cleaning up temporary file: {:?}", path);
            // Use blocking remove for simplicity in drop, consider tokio::spawn in complex scenarios
            match std::fs::remove_file(path) {
                Ok(_) => log::debug!("Successfully cleaned up temporary file: {:?}", path),
                Err(e) => log::error!("Failed to clean up temporary file {:?}: {}", path, e),
            }
        }
    }
}
