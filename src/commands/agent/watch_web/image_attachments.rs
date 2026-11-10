use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use anyhow::Context;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use thiserror::Error;

pub(super) const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
const MAX_STORAGE_BYTES: u64 = 1024 * 1024 * 1024;
const ATTACHMENT_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
static STORAGE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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
    let total_size = images.iter().try_fold(0_usize, |total, image| {
        total
            .checked_add(image.size)
            .filter(|size| *size <= MAX_IMAGE_BYTES)
            .ok_or(ValidationError::TooLarge)
    })?;
    if total_size > MAX_IMAGE_BYTES {
        return Err(ValidationError::TooLarge);
    }
    images
        .into_iter()
        .map(|image| {
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
    let _guard = STORAGE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow::anyhow!("image attachment storage lock is poisoned"))?;
    let directory = directory.unwrap_or(
        crate::shared::dirs::state_dir()
            .context("user state directory is unavailable")?
            .join("armyknife")
            .join("agent-watch-images"),
    );
    create_private_directory(&directory)?;
    prune_expired(&directory)?;
    let existing_size = directory_size(&directory)?;
    let incoming_size = images.iter().map(|image| image.bytes.len() as u64).sum();
    if existing_size.saturating_add(incoming_size) > MAX_STORAGE_BYTES {
        anyhow::bail!("image attachment storage is full");
    }
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

pub(super) fn append_paths_to_message(content: String, image_paths: &[PathBuf]) -> String {
    if image_paths.is_empty() {
        return content;
    }
    let paths = image_paths
        .iter()
        .map(|path| format!("- {}", path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    if content.trim().is_empty() {
        format!("Please inspect the attached image(s):\n{paths}")
    } else {
        format!(
            indoc::indoc! {"
                {}

                Attached image(s):
                {}
            "},
            content, paths
        )
        .trim_end()
        .to_string()
    }
}

fn prune_expired(directory: &Path) -> std::io::Result<()> {
    let now = SystemTime::now();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let metadata = entry.metadata()?;
        if now
            .duration_since(metadata.modified()?)
            .is_ok_and(|age| age >= ATTACHMENT_RETENTION)
        {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn directory_size(directory: &Path) -> std::io::Result<u64> {
    fs::read_dir(directory)?.try_fold(0_u64, |size, entry| {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Ok(size);
        }
        size.checked_add(entry.metadata()?.len()).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "directory size overflow")
        })
    })
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

#[cfg(test)]
mod tests {
    use std::fs::OpenOptions;

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

    #[test]
    fn save_rejects_when_the_retained_attachment_limit_is_reached() {
        let directory = tempfile::tempdir().unwrap();
        let occupied_path = directory.path().join("occupied.bin");
        OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&occupied_path)
            .unwrap()
            .set_len(super::MAX_STORAGE_BYTES)
            .unwrap();

        let error = super::save(
            vec![super::Image {
                extension: "gif",
                bytes: b"GIF89a".to_vec(),
            }],
            Some(directory.path().to_path_buf()),
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "image attachment storage is full");
    }

    fn request(mime_type: &str, bytes: &[u8]) -> Request {
        Request {
            mime_type: mime_type.to_string(),
            size: bytes.len(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }
}
