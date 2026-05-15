//! Browser-assisted publish flow for the Warzone 2100 maps database.
//!
//! GitHub's REST API does not support attaching binaries to issues, so the
//! file upload step has to happen in the browser. This module prepares the
//! `.wz.zip` next to the saved map and builds the prefilled issue URL for
//! `Warzone2100/map-submission`.

use std::path::{Path, PathBuf};

const SUBMISSION_BASE_URL: &str = "https://github.com/Warzone2100/map-submission/issues/new";
const ISSUE_TEMPLATE: &str = "submit_map.yml";

/// GitHub issue-form dropdowns match prefilled values against the full option
/// text, not a short key, so this string must stay byte-identical to the
/// `Mine:` entry in `Warzone2100/map-submission`'s `submit_map.yml`.
const AUTHORSHIP_MINE: &str = "Mine: I am the author of this map";

pub fn submission_url(map_name: &str) -> String {
    let title = format!("[MAP]: {map_name}");
    format!(
        "{SUBMISSION_BASE_URL}?template={ISSUE_TEMPLATE}&title={}&map-creator={}",
        percent_encode(&title),
        percent_encode(AUTHORSHIP_MINE),
    )
}

/// A WZ map is already a zip archive; only the extension needs to change
/// for GitHub's web form to accept it as an attachable archive.
pub fn write_wz_zip(wz_path: &Path) -> std::io::Result<PathBuf> {
    let file_name = wz_path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Map path has no file name",
        )
    })?;

    let mut zip_name = file_name.to_os_string();
    zip_name.push(".zip");

    let primary = wz_path.with_file_name(&zip_name);
    match std::fs::copy(wz_path, &primary) {
        Ok(_) => Ok(primary),
        Err(primary_err) if primary_err.kind() == std::io::ErrorKind::PermissionDenied => {
            let fallback = std::env::temp_dir().join(&zip_name);
            std::fs::copy(wz_path, &fallback)?;
            Ok(fallback)
        }
        Err(e) => Err(e),
    }
}

pub fn open_in_browser(url: &str) -> std::io::Result<()> {
    open::that(url)
}

pub fn reveal_in_file_manager(path: &Path) {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();

    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer.exe")
        .arg("/select,")
        .arg(path)
        .spawn();

    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn();

    if let Err(e) = result {
        log::warn!("Failed to reveal {}: {e}", path.display());
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        let safe = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~');
        if safe {
            out.push(byte as char);
        } else {
            use std::fmt::Write;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submission_url_title_is_url_encoded() {
        let url = submission_url("Sk-Rush");
        assert!(url.contains("title=%5BMAP%5D%3A%20Sk-Rush"));
        assert!(url.contains("template=submit_map.yml"));
        assert!(url.contains("map-creator=Mine%3A%20I%20am%20the%20author%20of%20this%20map"));
    }

    #[test]
    fn submission_url_handles_spaces_and_punctuation() {
        let url = submission_url("Big Open Plain!");
        assert!(
            url.contains("%5BMAP%5D%3A%20Big%20Open%20Plain%21"),
            "url was: {url}"
        );
    }

    #[test]
    fn percent_encode_keeps_unreserved() {
        assert_eq!(percent_encode("abcXYZ012-._~"), "abcXYZ012-._~");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("[MAP]:"), "%5BMAP%5D%3A");
    }
}
