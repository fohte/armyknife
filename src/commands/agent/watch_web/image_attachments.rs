use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use thiserror::Error;

pub(super) const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Request {
    pub(super) mime_type: String,
    pub(super) size: usize,
    pub(super) data: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Image {
    pub(super) extension: &'static str,
    pub(super) bytes: Vec<u8>,
}

#[derive(Debug, Error)]
pub(super) enum ValidationError {
    #[error("image attachments exceed the 10 MiB limit")]
    TooLarge,
    #[error("unsupported image type: {0}")]
    UnsupportedType(String),
    #[error("image data is invalid")]
    InvalidData,
}

pub(super) fn validate(images: Vec<Request>) -> Result<Vec<Image>, ValidationError> {
    let mut total_size = 0_usize;
    images
        .into_iter()
        .map(|image| {
            if image.size > MAX_IMAGE_BYTES {
                return Err(ValidationError::TooLarge);
            }
            total_size = total_size
                .checked_add(image.size)
                .ok_or(ValidationError::TooLarge)?;
            if total_size > MAX_IMAGE_BYTES {
                return Err(ValidationError::TooLarge);
            }

            let (extension, signature) = match image.mime_type.as_str() {
                "image/png" => ("png", &[137, 80, 78, 71, 13, 10, 26, 10][..]),
                "image/jpeg" => ("jpg", &[255, 216, 255][..]),
                "image/gif" => ("gif", b"GIF8" as &[u8]),
                "image/webp" => ("webp", b"RIFF" as &[u8]),
                _ => return Err(ValidationError::UnsupportedType(image.mime_type)),
            };
            let bytes = STANDARD
                .decode(image.data)
                .map_err(|_| ValidationError::InvalidData)?;
            if bytes.len() != image.size
                || !bytes.starts_with(signature)
                || (extension == "webp" && bytes.get(8..12) != Some(b"WEBP"))
            {
                return Err(ValidationError::InvalidData);
            }
            Ok(Image { extension, bytes })
        })
        .collect()
}

pub(super) fn save(images: Vec<Image>, directory: Option<PathBuf>) -> anyhow::Result<Vec<PathBuf>> {
    if images.is_empty() {
        return Ok(Vec::new());
    }
    let directory = directory.unwrap_or(state_dir()?.join("armyknife").join("agent-watch-images"));
    create_private_directory(&directory)?;
    let mut paths = Vec::with_capacity(images.len());
    for image in images {
        match save_one(&directory, image) {
            Ok(path) => paths.push(path),
            Err(error) => {
                remove(&paths);
                return Err(error);
            }
        }
    }
    Ok(paths)
}

pub(super) fn remove(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn save_one(directory: &Path, image: Image) -> anyhow::Result<PathBuf> {
    let path = directory.join(format!("{}.{}", uuid::Uuid::new_v4(), image.extension));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    if let Err(error) = file.write_all(&image.bytes) {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(error.into());
    }
    path.canonicalize().map_err(Into::into)
}

fn create_private_directory(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "image attachment directory must not be a symlink",
            ));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "image attachment path is not a directory",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(path)?,
        Err(error) => return Err(error),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn state_dir() -> anyhow::Result<PathBuf> {
    crate::shared::dirs::home_dir()
        .map(|home| home.join("Library").join("Application Support"))
        .context("user state directory is unavailable")
}

#[cfg(not(target_os = "macos"))]
fn state_dir() -> anyhow::Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME")
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
    }
    crate::shared::dirs::home_dir()
        .map(|home| home.join(".local").join("state"))
        .context("user state directory is unavailable")
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use rstest::rstest;

    use super::{Image, Request, ValidationError, validate};

    #[rstest]
    #[case::png("image/png", vec![137, 80, 78, 71, 13, 10, 26, 10], "png")]
    #[case::jpeg("image/jpeg", vec![255, 216, 255], "jpg")]
    #[case::gif("image/gif", b"GIF89a".to_vec(), "gif")]
    #[case::webp("image/webp", b"RIFF0000WEBP".to_vec(), "webp")]
    fn accepts_supported_signatures(
        #[case] mime_type: &str,
        #[case] bytes: Vec<u8>,
        #[case] _extension: &'static str,
    ) {
        let images = validate(vec![request(mime_type, &bytes)]).unwrap();
        assert_eq!(
            images,
            vec![Image {
                extension: _extension,
                bytes
            }]
        );
    }

    #[rstest]
    #[case::non_image("text/plain", b"hello".to_vec(), ValidationError::UnsupportedType("text/plain".to_string()))]
    #[case::mismatched_signature("image/png", b"GIF89a".to_vec(), ValidationError::InvalidData)]
    fn rejects_untrusted_image_data(
        #[case] mime_type: &str,
        #[case] bytes: Vec<u8>,
        #[case] expected: ValidationError,
    ) {
        let actual = validate(vec![request(mime_type, &bytes)]).unwrap_err();
        assert_eq!(actual.to_string(), expected.to_string());
    }

    #[test]
    fn rejects_declared_size_over_the_limit_before_decoding() {
        let error = validate(vec![Request {
            mime_type: "image/png".to_string(),
            size: super::MAX_IMAGE_BYTES + 1,
            data: String::new(),
        }])
        .unwrap_err();

        assert_eq!(
            error.to_string(),
            "image attachments exceed the 10 MiB limit"
        );
    }

    fn request(mime_type: &str, bytes: &[u8]) -> Request {
        Request {
            mime_type: mime_type.to_string(),
            size: bytes.len(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }
}
