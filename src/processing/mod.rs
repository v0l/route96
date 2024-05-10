use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Error;
use crate::processing::image::ImageProcessor;

mod image;

pub(crate) trait FileProcessor {
    fn process_file(&mut self, in_file: PathBuf, mime_type: &str) -> Result<PathBuf, Error>;
}

pub(crate) struct MediaProcessor {
    processors: HashMap<String, Box<dyn FileProcessor + Sync + Send>>,
}

impl MediaProcessor {
    pub fn new() -> Self {
        Self {
            processors: HashMap::new()
        }
    }
}

impl FileProcessor for MediaProcessor {
    fn process_file(&mut self, in_file: PathBuf, mime_type: &str) -> Result<PathBuf, Error> {
        if !self.processors.contains_key(mime_type) {
            if mime_type.starts_with("image/") {
                let ix = ImageProcessor::new();
                self.processors.insert(mime_type.to_string(), Box::new(ix));
            } else if mime_type.starts_with("video/") {
                
            }
        }
        
        let proc = match self.processors.get_mut(mime_type) {
            Some(p) => p,
            None => {
                return Err(Error::msg("Not supported mime type"));
            }
        };
        
        proc.process_file(in_file, mime_type)
    }
}