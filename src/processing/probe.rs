use anyhow::Result;
use ffmpeg_rs_raw::{Demuxer, DemuxerInfo};
use std::path::PathBuf;

/// Image converter to WEBP
pub struct FFProbe {}

impl FFProbe {
    pub fn new() -> Self {
        Self {}
    }

    pub fn process_file(self, in_file: PathBuf) -> Result<DemuxerInfo> {
        unsafe {
            let mut demuxer = Demuxer::new(in_file.to_str().unwrap())?;
            demuxer.probe_input()
        }
    }
}
