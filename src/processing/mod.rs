use std::path::PathBuf;

use anyhow::Error;

use crate::processing::webp::WebpProcessor;

mod webp;
mod blurhash;

pub(crate) enum FileProcessorResult {
    NewFile(NewFileProcessorResult),
    Skip,
}

pub(crate) struct NewFileProcessorResult {
    pub result: PathBuf,
    pub mime_type: String,
    pub width: usize,
    pub height: usize,
    pub blur_hash: String,
}

pub(crate) trait FileProcessor {
    fn process_file(&mut self, in_file: PathBuf, mime_type: &str) -> Result<FileProcessorResult, Error>;
}

pub(crate) struct MediaProcessor {}

impl MediaProcessor {
    pub fn new() -> Self {
        Self {}
    }
}

impl FileProcessor for MediaProcessor {
    fn process_file(&mut self, in_file: PathBuf, mime_type: &str) -> Result<FileProcessorResult, Error> {
        let proc = if mime_type.starts_with("image/") {
            Some(WebpProcessor::new())
        } else {
            None
        };
        if let Some(mut proc) = proc {
            proc.process_file(in_file, mime_type)
        } else {
            Ok(FileProcessorResult::Skip)
        }
    }
}