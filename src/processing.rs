#![cfg(feature = "media-compression")]

use anyhow::{Context, Result};
use ffmpeg_the_third as ffmpeg;
use std::path::Path;

/// Opens a media file using ffmpeg to probe its format and streams.
///
/// Returns an Input context if successful.
pub fn probe_file(path: &Path) -> Result<ffmpeg::format::context::Input> {
    ffmpeg::init().context("Failed to initialize ffmpeg")?;

    let ictx =
        ffmpeg::format::input(&path).context(format!("Failed to open input file: {:?}", path))?;

    Ok(ictx)
}
