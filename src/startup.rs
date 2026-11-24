use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use anyhow::{Error, Result};

pub fn process_uid() -> Option<u32> {
    fs::metadata("/proc/self").ok().map(|m| m.uid())
}

fn owner_note(path: &Path) -> String {
    let process = process_uid()
        .map(|u| u.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    match fs::metadata(path) {
        Ok(m) => format!(
            "owner {}:{}, mode {:o}, process uid {}",
            m.uid(),
            m.gid(),
            m.mode() & 0o7777,
            process
        ),
        Err(_) => format!("process uid {process}"),
    }
}

fn path_error(what: &str, path: &Path, err: io::Error) -> Error {
    let shown = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut msg = format!("{}: {} ({})", shown.display(), err, owner_note(path));
    if err.kind() == io::ErrorKind::PermissionDenied {
        msg.push_str(&format!(
            ". The {what} is not accessible to this user; see \"Upgrading from images before 17 Nov\" in the README"
        ));
    }
    Error::msg(msg)
}

pub fn check_config_readable(path: &Path) -> Result<()> {
    match fs::metadata(path) {
        Ok(_) => File::open(path)
            .map(|_| ())
            .map_err(|e| path_error("config file", path, e)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(path_error("config file", path, e)),
    }
}

pub fn check_dir_writable(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|e| path_error("data directory", path, e))?;
    let probe = path.join(format!(".route96-write-check-{}", uuid::Uuid::new_v4()));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(|e| path_error("data directory", path, e))?;
    fs::remove_file(&probe).map_err(|e| path_error("data directory", path, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn running_as_root() -> bool {
        if process_uid() == Some(0) {
            eprintln!("skipping: running as root, mode 000 does not block root");
            return true;
        }
        false
    }

    #[test]
    fn unreadable_config_error_names_path_and_owner() {
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        fs::write(&path, "listen: \"0.0.0.0:8000\"\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

        let err = check_config_readable(&path).unwrap_err().to_string();
        assert!(err.starts_with(&path.display().to_string()), "{err}");
        assert!(err.contains("Permission denied"), "{err}");
        assert!(err.contains("mode 0"), "{err}");
        assert!(
            err.contains(&format!("process uid {}", process_uid().unwrap())),
            "{err}"
        );
    }

    #[test]
    fn readable_config_passes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        fs::write(&path, "listen: \"0.0.0.0:8000\"\n").unwrap();
        check_config_readable(&path).unwrap();
    }

    #[test]
    fn missing_config_is_left_to_the_loader() {
        let dir = tempfile::tempdir().unwrap();
        check_config_readable(&dir.path().join("config")).unwrap();
    }

    #[test]
    fn read_only_data_dir_error_names_path() {
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        fs::create_dir(&data).unwrap();
        fs::set_permissions(&data, fs::Permissions::from_mode(0o555)).unwrap();

        let err = check_dir_writable(&data).unwrap_err().to_string();
        fs::set_permissions(&data, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(err.starts_with(&data.display().to_string()), "{err}");
        assert!(err.contains("Permission denied"), "{err}");
    }

    #[test]
    fn writable_data_dir_is_created_and_left_clean() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("nested").join("data");
        check_dir_writable(&data).unwrap();
        assert_eq!(fs::read_dir(&data).unwrap().count(), 0);
    }
}
