//! Perceptual hashing (pHash) for image similarity search.
//!
//! Images are decoded by ffmpeg and fed to
//! [`image_hasher`](https://crates.io/crates/image_hasher) which computes a
//! 64-bit DCT-based pHash.  The hash is stored in the database as four indexed
//! 16-bit LSH band columns, avoiding full table scans when searching for
//! similar images.  Exact Hamming distance is verified on candidates in
//! application code.

use anyhow::{Error, Result};
use image::{ImageBuffer, Rgb};
use image_hasher::{HashAlg, HasherConfig, ImageHash};
use std::path::Path;
use std::slice;

use ffmpeg_rs_raw::{
    Decoder, Demuxer, Scaler, ffmpeg_sys_the_third::AVPixelFormat::AV_PIX_FMT_RGB24,
};

/// Maximum Hamming distance (inclusive) for two images to be "similar".
pub const MAX_HAMMING_DISTANCE: u32 = 10;

// ── Hash type ──────────────────────────────────────────────────────────────

/// A 64-bit perceptual hash backed by [`image_hasher::ImageHash`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PHash(pub ImageHash<[u8; 8]>);

impl PHash {
    /// Hamming distance to another hash.
    pub fn hamming(&self, other: &PHash) -> u32 {
        self.0.dist(&other.0)
    }

    /// Raw 8-byte hash (split into LSH bands for storage).
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// Reconstruct from 8 raw bytes loaded from the database.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        ImageHash::from_bytes(bytes)
            .map(PHash)
            .map_err(|e| Error::msg(format!("invalid phash bytes: {:?}", e)))
    }
}

// ── Hasher ─────────────────────────────────────────────────────────────────

/// Build the shared hasher: Median + DCT preprocessing = canonical pHash.
pub fn make_hasher() -> image_hasher::Hasher<[u8; 8]> {
    HasherConfig::with_bytes_type::<[u8; 8]>()
        .hash_alg(HashAlg::Median)
        .hash_size(8, 8)
        .preproc_dct()
        .to_hasher()
}

// ── File entry point ───────────────────────────────────────────────────────

/// Decode `path` with ffmpeg and compute its perceptual hash.
/// Returns `Err` for non-`image/*` MIME types or decode failures.
pub fn phash_image(path: &Path, mime_type: &str) -> Result<PHash> {
    if !mime_type.starts_with("image/") {
        return Err(Error::msg(format!(
            "phash_image: unsupported mime type '{}'",
            mime_type
        )));
    }
    let (width, height, pixels) = unsafe { decode_rgb(path)? };
    let buf = ImageBuffer::<Rgb<u8>, Vec<u8>>::from_raw(width, height, pixels)
        .ok_or_else(|| Error::msg("pixel buffer size mismatch"))?;
    Ok(PHash(make_hasher().hash_image(&buf)))
}

unsafe fn decode_rgb(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let mut demux = Demuxer::new(path.to_str().unwrap())?;
    let info = unsafe { demux.probe_input()? };
    let stream = info
        .best_video()
        .ok_or(Error::msg("no video/image stream found"))?;

    let width = stream.width as u32;
    let height = stream.height as u32;

    let mut decoder = Decoder::new();
    decoder.setup_decoder(stream, None)?;

    let mut scaler = Scaler::new();

    while let Ok((pkt, _)) = unsafe { demux.get_packet() } {
        let pkt = match pkt {
            Some(p) => p,
            None => break,
        };
        let decoded = decoder.decode_pkt(Some(&pkt))?;
        if let Some((frame, _)) = decoded.into_iter().next() {
            let scaled =
                scaler.process_frame(&frame, width as u16, height as u16, AV_PIX_FMT_RGB24)?;
            let mut out = Vec::with_capacity(3 * width as usize * height as usize);
            for row in 0..height as usize {
                let stride = scaled.linesize[0] as usize;
                let row_slice = unsafe {
                    slice::from_raw_parts(scaled.data[0].add(stride * row), 3 * width as usize)
                };
                out.extend_from_slice(row_slice);
            }
            return Ok((width, height, out));
        }
    }
    Err(Error::msg("no image data found"))
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn make_jpeg(r: u8, g: u8, b: u8) -> tempfile::NamedTempFile {
        let f = tempfile::Builder::new().suffix(".jpg").tempfile().unwrap();
        let color_str = format!("color=c=0x{:02X}{:02X}{:02X}:size=64x64:rate=1", r, g, b);
        let status = Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                &color_str,
                "-frames:v",
                "1",
                f.path().to_str().unwrap(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("ffmpeg must be in PATH to run phash tests");
        assert!(status.success(), "ffmpeg failed to create test JPEG");
        f
    }

    #[test]
    fn test_make_hasher_does_not_panic() {
        let _ = make_hasher();
    }

    #[test]
    fn test_phash_image_rejects_non_image_mime() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let err = phash_image(tmp.path(), "video/mp4").unwrap_err();
        assert!(err.to_string().contains("unsupported mime type"));
    }

    #[test]
    fn test_phash_image_jpeg_succeeds() {
        let f = make_jpeg(100, 150, 200);
        assert_eq!(
            phash_image(f.path(), "image/jpeg")
                .unwrap()
                .as_bytes()
                .len(),
            8
        );
    }

    #[test]
    fn test_hamming_same_file_is_zero() {
        let f = make_jpeg(80, 120, 160);
        let h1 = phash_image(f.path(), "image/jpeg").unwrap();
        let h2 = phash_image(f.path(), "image/jpeg").unwrap();
        assert_eq!(h1.hamming(&h2), 0);
    }

    #[test]
    fn test_hamming_symmetric() {
        let f1 = make_jpeg(10, 20, 30);
        let f2 = make_jpeg(200, 180, 160);
        let h1 = phash_image(f1.path(), "image/jpeg").unwrap();
        let h2 = phash_image(f2.path(), "image/jpeg").unwrap();
        assert_eq!(h1.hamming(&h2), h2.hamming(&h1));
    }

    #[test]
    fn test_hamming_different_images_nonzero() {
        let gradient = {
            let f = tempfile::Builder::new().suffix(".jpg").tempfile().unwrap();
            let status = Command::new("ffmpeg")
                .args([
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    "gradients=size=64x64:speed=0:type=linear",
                    "-frames:v",
                    "1",
                    f.path().to_str().unwrap(),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .expect("ffmpeg must be in PATH");
            assert!(status.success());
            f
        };
        let solid = make_jpeg(128, 128, 128);
        let h1 = phash_image(gradient.path(), "image/jpeg").unwrap();
        let h2 = phash_image(solid.path(), "image/jpeg").unwrap();
        assert!(h1.hamming(&h2) > 0);
    }

    #[test]
    fn test_roundtrip_bytes() {
        let f = make_jpeg(64, 128, 192);
        let h = phash_image(f.path(), "image/jpeg").unwrap();
        let h2 = PHash::from_bytes(h.as_bytes()).unwrap();
        assert_eq!(h.hamming(&h2), 0);
    }

    #[test]
    fn test_identical_images_within_threshold() {
        let f = make_jpeg(128, 128, 128);
        let h1 = phash_image(f.path(), "image/jpeg").unwrap();
        let h2 = phash_image(f.path(), "image/jpeg").unwrap();
        assert!(h1.hamming(&h2) <= MAX_HAMMING_DISTANCE);
    }
}
