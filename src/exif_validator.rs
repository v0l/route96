use anyhow::{Result, Error};
use std::path::Path;
use log::info;

/// Checks if an image file contains sensitive EXIF metadata
/// Returns Err if sensitive data is found, Ok otherwise
pub fn check_for_sensitive_exif(file_path: &Path) -> Result<()> {
    // Try to read the file
    let file = match std::fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => {
            // If we can't read the file, allow it to proceed
            // (it might not be an image or might not support EXIF)
            return Ok(());
        }
    };

    let mut bufreader = std::io::BufReader::new(&file);
    
    // Try to parse EXIF data
    let exifreader = match exif::Reader::new().read_from_container(&mut bufreader) {
        Ok(reader) => reader,
        Err(_) => {
            // No EXIF data found or not a supported format - allow upload
            return Ok(());
        }
    };

    // Check for GPS coordinates
    if has_gps_data(&exifreader) {
        info!("Rejecting upload: GPS data found in EXIF");
        return Err(Error::msg("Image contains GPS location data"));
    }

    // Check for device information
    if has_device_info(&exifreader) {
        info!("Rejecting upload: Device information found in EXIF");
        return Err(Error::msg("Image contains device identification information"));
    }

    Ok(())
}

/// Check if EXIF data contains GPS coordinates
fn has_gps_data(reader: &exif::Exif) -> bool {
    // GPS latitude
    if reader.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY).is_some() {
        return true;
    }
    
    // GPS longitude
    if reader.get_field(exif::Tag::GPSLongitude, exif::In::PRIMARY).is_some() {
        return true;
    }
    
    // GPS altitude
    if reader.get_field(exif::Tag::GPSAltitude, exif::In::PRIMARY).is_some() {
        return true;
    }
    
    // GPS timestamp
    if reader.get_field(exif::Tag::GPSTimeStamp, exif::In::PRIMARY).is_some() {
        return true;
    }
    
    // GPS date stamp
    if reader.get_field(exif::Tag::GPSDateStamp, exif::In::PRIMARY).is_some() {
        return true;
    }
    
    false
}

/// Check if EXIF data contains device identification information
fn has_device_info(reader: &exif::Exif) -> bool {
    // Camera make
    if reader.get_field(exif::Tag::Make, exif::In::PRIMARY).is_some() {
        return true;
    }
    
    // Camera model
    if reader.get_field(exif::Tag::Model, exif::In::PRIMARY).is_some() {
        return true;
    }
    
    // Software/firmware
    if reader.get_field(exif::Tag::Software, exif::In::PRIMARY).is_some() {
        return true;
    }
    
    // Lens make
    if reader.get_field(exif::Tag::LensMake, exif::In::PRIMARY).is_some() {
        return true;
    }
    
    // Lens model
    if reader.get_field(exif::Tag::LensModel, exif::In::PRIMARY).is_some() {
        return true;
    }
    
    // Serial number
    if reader.get_field(exif::Tag::BodySerialNumber, exif::In::PRIMARY).is_some() {
        return true;
    }
    
    // Lens serial number
    if reader.get_field(exif::Tag::LensSerialNumber, exif::In::PRIMARY).is_some() {
        return true;
    }
    
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_exif_data_allows_upload() {
        // Create a simple JPEG without EXIF
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_no_exif.jpg");
        
        // Minimal valid JPEG header (this won't have EXIF data)
        let jpeg_data = vec![
            0xFF, 0xD8, // SOI marker
            0xFF, 0xE0, // APP0 marker
            0x00, 0x10, // length
            0x4A, 0x46, 0x49, 0x46, 0x00, // "JFIF\0"
            0x01, 0x01, // version
            0x00, // units
            0x00, 0x01, 0x00, 0x01, // X and Y density
            0x00, 0x00, // thumbnail dimensions
            0xFF, 0xD9, // EOI marker
        ];
        
        std::fs::write(&test_file, jpeg_data).unwrap();
        
        // Should not error because there's no EXIF data
        let result = check_for_sensitive_exif(&test_file);
        assert!(result.is_ok());
        
        // Cleanup
        std::fs::remove_file(test_file).ok();
    }

    #[test]
    fn test_non_image_file_allows_upload() {
        // Create a text file
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_text.txt");
        
        std::fs::write(&test_file, "This is not an image").unwrap();
        
        // Should not error because it's not an image
        let result = check_for_sensitive_exif(&test_file);
        assert!(result.is_ok());
        
        // Cleanup
        std::fs::remove_file(test_file).ok();
    }
}
