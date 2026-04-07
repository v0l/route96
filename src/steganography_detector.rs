use anyhow::{bail, Result};
use std::io::Read;
use std::path::Path;

/// Checks if an image file contains signs of steganography or hidden data.
/// Returns Err if suspicious patterns are found, Ok otherwise.
pub fn check_for_steganography(file_path: &Path) -> Result<()> {
    let mut file = match std::fs::File::open(file_path) {
        Ok(f) => f,
        Err(_) => return Ok(()),
    };

    let mut buffer = Vec::new();
    if let Err(_) = file.read_to_end(&mut buffer) {
        return Ok(());
    }

    // Check if it's a JPEG file
    if buffer.len() < 2 || buffer[0] != 0xFF || buffer[1] != 0xD8 {
        return Ok(());
    }

    // Check for data after EOI marker
    if has_data_after_eoi(&buffer) {
        bail!("Image rejected: contains data after end-of-image marker");
    }

    // Check for MPF (Multi-Picture Format) segments
    if buffer.windows(4).any(|w| w == b"MPF\0") {
        bail!("Image rejected: contains MPF segment");
    }

    // Check for suspicious comment segments
    if has_suspicious_comments(&buffer) {
        bail!("Image rejected: contains suspicious comment segments");
    }

    // Check for excessive APP segment usage
    if has_excessive_app_segments(&buffer) {
        bail!("Image rejected: contains excessive metadata segments");
    }

    Ok(())
}

/// Check for data after EOI (End of Image) marker 0xFFD9
fn has_data_after_eoi(data: &[u8]) -> bool {
    for i in 0..data.len() - 1 {
        if data[i] == 0xFF && data[i + 1] == 0xD9 {
            // Found EOI, check if there's non-padding data after it
            for j in (i + 2)..data.len() {
                if data[j] != 0x00 && data[j] != 0xFF {
                    return true;
                }
            }
            break;
        }
    }
    false
}

/// Check for suspicious comment segments (COM marker 0xFE)
fn has_suspicious_comments(data: &[u8]) -> bool {
    let mut pos = 2;

    while pos < data.len() - 1 {
        if data[pos] != 0xFF {
            pos += 1;
            continue;
        }

        let mut marker_pos = pos;
        while marker_pos < data.len() && data[marker_pos] == 0xFF {
            marker_pos += 1;
        }

        if marker_pos >= data.len() {
            break;
        }

        let marker = data[marker_pos];

        // COM marker (0xFE)
        if marker == 0xFE {
            if marker_pos + 2 >= data.len() {
                break;
            }
            let segment_len =
                u16::from_be_bytes([data[marker_pos + 1], data[marker_pos + 2]]) as usize;
            if marker_pos + segment_len > data.len() {
                break;
            }

            let comment_data = &data[marker_pos + 4..marker_pos + segment_len];

            // Reject comments that look like binary data or have null bytes
            let null_count = comment_data.iter().filter(|&&b| b == 0).count();
            if null_count > 10 || null_count > comment_data.len() / 4 {
                return true;
            }

            // Reject comments with very high entropy (look like encrypted/encoded data)
            if !comment_data.is_empty() {
                let entropy = calculate_entropy(comment_data);
                if entropy > 7.0 {
                    return true;
                }
            }
        }

        if marker >= 0xD0 && marker <= 0xD9 {
            pos = marker_pos + 1;
        } else if marker_pos + 2 >= data.len() {
            break;
        } else {
            let segment_len =
                u16::from_be_bytes([data[marker_pos + 1], data[marker_pos + 2]]) as usize;
            pos = marker_pos + segment_len;
        }
    }

    false
}

/// Check for excessive APP segments (could hide data in metadata)
fn has_excessive_app_segments(data: &[u8]) -> bool {
    let mut app_count = 0;
    let mut total_app_size = 0;
    let mut pos = 2;

    while pos < data.len() - 1 {
        if data[pos] != 0xFF {
            pos += 1;
            continue;
        }

        let mut marker_pos = pos;
        while marker_pos < data.len() && data[marker_pos] == 0xFF {
            marker_pos += 1;
        }

        if marker_pos >= data.len() {
            break;
        }

        let marker = data[marker_pos];

        // APP markers (0xE0-0xEF)
        if marker >= 0xE0 && marker <= 0xEF {
            app_count += 1;

            if marker_pos + 2 >= data.len() {
                break;
            }
            let segment_len =
                u16::from_be_bytes([data[marker_pos + 1], data[marker_pos + 2]]) as usize;
            total_app_size += segment_len;

            if marker_pos + segment_len > data.len() {
                break;
            }
            pos = marker_pos + segment_len;
        } else if marker >= 0xD0 && marker <= 0xD9 {
            pos = marker_pos + 1;
        } else if marker_pos + 2 >= data.len() {
            break;
        } else {
            let segment_len =
                u16::from_be_bytes([data[marker_pos + 1], data[marker_pos + 2]]) as usize;
            pos = marker_pos + segment_len;
        }
    }

    // Flag if there are more than 5 APP segments or total metadata > 50KB
    app_count > 5 || total_app_size > 50 * 1024
}

fn calculate_entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }

    let mut frequency = [0u64; 256];
    for &byte in data {
        frequency[byte as usize] += 1;
    }

    let len = data.len() as f64;
    let mut entropy = 0.0;

    for &count in &frequency {
        if count > 0 {
            let p = count as f64 / len;
            entropy -= p * p.log2();
        }
    }

    entropy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_jpeg_allowed() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_not_jpeg.png");
        std::fs::write(&test_file, b"not a jpeg").unwrap();

        let result = check_for_steganography(&test_file);
        assert!(result.is_ok());

        std::fs::remove_file(&test_file).ok();
    }

    #[test]
    fn test_minimal_jpeg_allowed() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_minimal.jpg");

        let jpeg_data = vec![0xFF, 0xD8, 0xFF, 0xD9];
        std::fs::write(&test_file, jpeg_data).unwrap();

        let result = check_for_steganography(&test_file);
        assert!(result.is_ok());

        std::fs::remove_file(&test_file).ok();
    }

    #[test]
    fn test_data_after_eoi_rejected() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_eoi.jpg");

        // JPEG with hidden data after EOI
        let jpeg_data = vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xD9, // EOI
            0x00, 0x01, 0x02, 0x03, 0x04, // Hidden data after EOI
        ];
        std::fs::write(&test_file, jpeg_data).unwrap();

        let result = check_for_steganography(&test_file);
        assert!(result.is_err());

        std::fs::remove_file(&test_file).ok();
    }

    #[test]
    fn test_mpf_rejected() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_mpf.jpg");

        let jpeg_data = vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xE2, // APP2
            0x00, 0x0C, 0x4D, 0x50, 0x46, 0x00, // "MPF\0"
            0x00, 0x00, 0x00, 0x00, 0xFF, 0xD9, // EOI
        ];
        std::fs::write(&test_file, jpeg_data).unwrap();

        let result = check_for_steganography(&test_file);
        assert!(result.is_err());

        std::fs::remove_file(&test_file).ok();
    }

    #[test]
    fn test_binary_comment_rejected() {
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_comment.jpg");

        // JPEG with binary comment segment (many null bytes)
        // COM segment: marker(2) + length(2) + data. Length includes itself.
        // So length=20 means 18 bytes of comment data
        let jpeg_data = vec![
            0xFF, 0xD8, // SOI
            0xFF, 0xFE, // COM
            0x00, 0x14, // length = 20 (includes these 2 bytes)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 8 null bytes
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 8 more null bytes
            0x01, 0x02, // 2 bytes data
            0xFF, 0xD9, // EOI
        ];
        std::fs::write(&test_file, jpeg_data).unwrap();

        let result = check_for_steganography(&test_file);
        assert!(result.is_err());

        std::fs::remove_file(&test_file).ok();
    }
}
