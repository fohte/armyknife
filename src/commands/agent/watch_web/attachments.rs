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

pub(super) const MAX_ATTACHMENT_BYTES: usize = 50 * 1024 * 1024;
pub(super) const MAX_ATTACHMENTS_PER_MESSAGE: usize = 16;
const MAX_STORAGE_BYTES: u64 = 1024 * 1024 * 1024;
const STORAGE_FILE_OVERHEAD_BYTES: u64 = 64 * 1024;
const ATTACHMENT_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
static STORAGE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Request {
    pub(super) name: String,
    #[serde(default)]
    pub(super) mime_type: Option<String>,
    pub(super) size: usize,
    pub(super) data: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LegacyImageRequest {
    pub(super) mime_type: String,
    pub(super) size: usize,
    pub(super) data: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Attachment {
    pub(super) name: String,
    pub(super) bytes: Vec<u8>,
    kind: AttachmentKind,
}

#[derive(Debug, PartialEq, Eq)]
enum AttachmentKind {
    Image,
    File,
}

#[derive(Debug, Error)]
pub(super) enum ValidationError {
    #[error("a message cannot include more than 16 file attachments")]
    TooManyAttachments,
    #[error("file attachments exceed the 50 MiB limit")]
    TooLarge,
    #[error("unsupported image type: {0}")]
    UnsupportedImageType(String),
    #[error("file attachment data is invalid")]
    InvalidData,
}

pub(super) fn validate(
    attachments: Vec<Request>,
    legacy_images: Vec<LegacyImageRequest>,
) -> Result<Vec<Attachment>, ValidationError> {
    if attachments.len() + legacy_images.len() > MAX_ATTACHMENTS_PER_MESSAGE {
        return Err(ValidationError::TooManyAttachments);
    }
    let total_size = attachments
        .iter()
        .map(|attachment| attachment.size)
        .chain(legacy_images.iter().map(|image| image.size))
        .try_fold(0_usize, |total, size| {
            total
                .checked_add(size)
                .filter(|size| *size <= MAX_ATTACHMENT_BYTES)
                .ok_or(ValidationError::TooLarge)
        })?;
    if total_size > MAX_ATTACHMENT_BYTES {
        return Err(ValidationError::TooLarge);
    }
    let attachments = attachments
        .into_iter()
        .map(|attachment| {
            let bytes = STANDARD
                .decode(attachment.data)
                .map_err(|_| ValidationError::InvalidData)?;
            if bytes.len() != attachment.size {
                return Err(ValidationError::InvalidData);
            }
            let extension = preview_extension(attachment.mime_type.as_deref(), &bytes);
            let (name, kind) = if let Some(extension) = extension {
                (
                    ensure_image_extension(attachment.name, extension),
                    AttachmentKind::Image,
                )
            } else {
                (attachment.name, AttachmentKind::File)
            };
            Ok(Attachment { name, bytes, kind })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let legacy_images = legacy_images
        .into_iter()
        .map(validate_legacy_image)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(attachments.into_iter().chain(legacy_images).collect())
}

fn preview_extension(mime_type: Option<&str>, bytes: &[u8]) -> Option<&'static str> {
    let (detected_mime_type, extension) = image_format_from_bytes(bytes)?;
    (mime_type? == detected_mime_type).then_some(extension)
}

pub(super) fn image_format_from_bytes(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    let mime_type = if bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]) {
        "image/png"
    } else if bytes.starts_with(&[255, 216, 255]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return None;
    };
    let extension = match mime_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => return None,
    };
    Some((mime_type, extension))
}

fn ensure_image_extension(name: String, extension: &str) -> String {
    let name = sanitize_filename(&name);
    if image_extension_from_name(&name) == Some(extension) {
        name
    } else {
        let stem_length = 180 - extension.len() - 1;
        let stem = name.chars().take(stem_length).collect::<String>();
        format!("{}.{extension}", stem.trim_end_matches('.'))
    }
}

fn image_extension_from_name(name: &str) -> Option<&'static str> {
    match Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpg"),
        "gif" => Some("gif"),
        "webp" => Some("webp"),
        _ => None,
    }
}

fn validate_legacy_image(image: LegacyImageRequest) -> Result<Attachment, ValidationError> {
    if !matches!(
        image.mime_type.as_str(),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    ) {
        return Err(ValidationError::UnsupportedImageType(image.mime_type));
    }
    let bytes = STANDARD
        .decode(image.data)
        .map_err(|_| ValidationError::InvalidData)?;
    let Some((detected_mime_type, extension)) = image_format_from_bytes(&bytes) else {
        return Err(ValidationError::InvalidData);
    };
    if bytes.len() != image.size || detected_mime_type != image.mime_type {
        return Err(ValidationError::InvalidData);
    }
    Ok(Attachment {
        name: format!("legacy-image.{extension}"),
        bytes,
        kind: AttachmentKind::Image,
    })
}

pub(super) fn save(
    attachments: Vec<Attachment>,
    directory: Option<PathBuf>,
) -> anyhow::Result<Vec<PathBuf>> {
    if attachments.is_empty() {
        return Ok(Vec::new());
    }
    let _guard = STORAGE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow::anyhow!("file attachment storage lock is poisoned"))?;
    let directory = directory.unwrap_or(
        crate::shared::dirs::state_dir()
            .context("user state directory is unavailable")?
            .join("armyknife")
            .join("agent-watch-images"),
    );
    create_private_directory(&directory)?;
    prune_expired(&directory)?;
    let existing_usage = directory_usage_bytes(&directory)?;
    let incoming_usage = attachments.iter().try_fold(0_u64, |total, attachment| {
        let attachment_usage =
            (attachment.bytes.len() as u64).checked_add(STORAGE_FILE_OVERHEAD_BYTES)?;
        total.checked_add(attachment_usage)
    });
    let Some(incoming_usage) = incoming_usage else {
        anyhow::bail!("file attachment storage size overflow");
    };
    if existing_usage.saturating_add(incoming_usage) > MAX_STORAGE_BYTES {
        anyhow::bail!("file attachment storage is full");
    }
    let mut paths = Vec::with_capacity(attachments.len());
    for attachment in attachments {
        match save_one(&directory, attachment) {
            Ok(path) => paths.push(path),
            Err(error) => {
                remove_files_unlocked(&paths);
                return Err(error);
            }
        }
    }
    Ok(paths)
}

pub(super) fn remove(paths: &[PathBuf]) {
    let _guard = STORAGE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    remove_files_unlocked(paths);
}

fn remove_files_unlocked(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

pub(super) fn append_paths_to_message(content: String, attachment_paths: &[PathBuf]) -> String {
    if attachment_paths.is_empty() {
        return content;
    }
    let paths = attachment_paths
        .iter()
        .map(|path| format!("- {}", path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    if content.trim().is_empty() {
        format!("Please inspect the attached file(s):\n{paths}")
    } else {
        format!(
            indoc::indoc! {"
                {}

                Attached file(s):
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

fn directory_usage_bytes(directory: &Path) -> std::io::Result<u64> {
    fs::read_dir(directory)?.try_fold(0_u64, |size, entry| {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            return Ok(size);
        }
        let file_usage = entry
            .metadata()?
            .len()
            .checked_add(STORAGE_FILE_OVERHEAD_BYTES)
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "directory size overflow")
            })?;
        size.checked_add(file_usage).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "directory size overflow")
        })
    })
}

fn save_one(directory: &Path, attachment: Attachment) -> anyhow::Result<PathBuf> {
    let kind = match attachment.kind {
        AttachmentKind::Image => "image",
        AttachmentKind::File => "file",
    };
    let path = directory.join(format!(
        "{}~{kind}~{}",
        uuid::Uuid::new_v4(),
        sanitize_filename(&attachment.name),
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    if let Err(error) = file.write_all(&attachment.bytes) {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(error.into());
    }
    path.canonicalize().map_err(Into::into)
}

fn sanitize_filename(name: &str) -> String {
    let basename = name.replace('\\', "/");
    let basename = basename.rsplit('/').next().unwrap_or_default();
    let sanitized: String = basename
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(180)
        .collect();
    let sanitized = sanitized.trim_matches('.');
    if sanitized.is_empty() {
        "attachment".to_string()
    } else {
        sanitized.to_string()
    }
}

fn create_private_directory(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "file attachment directory must not be a symlink",
            ));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "file attachment path is not a directory",
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
    use base64::engine::general_purpose::STANDARD;
    use rstest::rstest;

    use super::{
        Attachment, AttachmentKind, LegacyImageRequest, MAX_ATTACHMENT_BYTES, Request,
        ValidationError, validate,
    };

    const PNG_SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    const GIF_SIGNATURE: &[u8] = b"GIF89a";
    const WEBP_SIGNATURE: &[u8] = b"RIFF0000WEBP";

    #[test]
    fn accepts_non_image_files_and_preserves_their_names() {
        let attachments =
            validate(vec![request("design.html", b"<main>Mock</main>")], vec![]).unwrap();

        assert_eq!(
            attachments,
            vec![Attachment {
                name: "design.html".to_string(),
                bytes: b"<main>Mock</main>".to_vec(),
                kind: AttachmentKind::File,
            }]
        );
    }

    #[rstest]
    #[case::nameless_png("image/png", "Pasted file", PNG_SIGNATURE, "Pasted_file.png")]
    #[case::jpeg_with_another_extension(
        "image/jpeg",
        "diagram.png",
        b"\xff\xd8\xff",
        "diagram.png.jpg"
    )]
    #[case::gif("image/gif", "Pasted file", GIF_SIGNATURE, "Pasted_file.gif")]
    #[case::webp("image/webp", "Pasted file", WEBP_SIGNATURE, "Pasted_file.webp")]
    fn retains_previewable_image_types_in_saved_names(
        #[case] mime_type: &str,
        #[case] name: &str,
        #[case] bytes: &[u8],
        #[case] expected_name: &str,
    ) {
        let attachments =
            validate(vec![request_with_mime_type(name, mime_type, bytes)], vec![]).unwrap();

        assert_eq!(
            attachments,
            vec![Attachment {
                name: expected_name.to_string(),
                bytes: bytes.to_vec(),
                kind: AttachmentKind::Image,
            }]
        );
    }

    #[test]
    fn retains_the_image_extension_when_the_original_name_is_long() {
        let name = "a".repeat(180);
        let attachments = validate(
            vec![request_with_mime_type(&name, "image/png", PNG_SIGNATURE)],
            vec![],
        )
        .unwrap();

        assert_eq!(
            attachments,
            vec![Attachment {
                name: format!("{}.png", "a".repeat(176)),
                bytes: PNG_SIGNATURE.to_vec(),
                kind: AttachmentKind::Image,
            }]
        );
    }

    #[rstest]
    #[case::declared_size_over_limit(vec![Request {
        name: "large.bin".to_string(),
        mime_type: None,
        size: MAX_ATTACHMENT_BYTES + 1,
        data: String::new(),
    }], ValidationError::TooLarge)]
    #[case::decoded_size_mismatch(vec![Request {
        name: "mismatch.bin".to_string(),
        mime_type: None,
        size: 2,
        data: STANDARD.encode(b"x"),
    }], ValidationError::InvalidData)]
    fn rejects_oversized_or_invalid_data(
        #[case] attachments: Vec<Request>,
        #[case] expected: ValidationError,
    ) {
        assert_eq!(
            validate(attachments, vec![]).unwrap_err().to_string(),
            expected.to_string()
        );
    }

    #[test]
    fn validates_legacy_image_payloads_during_page_restarts() {
        let attachments = validate(
            vec![],
            vec![LegacyImageRequest {
                mime_type: "image/gif".to_string(),
                size: 6,
                data: STANDARD.encode(b"GIF89a"),
            }],
        )
        .unwrap();

        assert_eq!(
            attachments,
            vec![Attachment {
                name: "legacy-image.gif".to_string(),
                bytes: b"GIF89a".to_vec(),
                kind: AttachmentKind::Image,
            }]
        );
    }

    #[test]
    fn save_uses_unique_sanitized_names_without_leaving_the_attachment_directory() {
        let directory = tempfile::tempdir().unwrap();
        let paths = super::save(
            vec![
                Attachment {
                    name: "../design mock.html".to_string(),
                    bytes: b"one".to_vec(),
                    kind: AttachmentKind::File,
                },
                Attachment {
                    name: "../design mock.html".to_string(),
                    bytes: b"two".to_vec(),
                    kind: AttachmentKind::File,
                },
            ],
            Some(directory.path().to_path_buf()),
        )
        .unwrap();
        let canonical_directory = directory.path().canonicalize().unwrap();
        let snapshot = paths
            .iter()
            .map(|path| {
                let filename = path.file_name().unwrap().to_string_lossy();
                let sanitized_name = filename.split_once("~file~").unwrap().1.to_string();
                (
                    path.is_absolute(),
                    path.parent() == Some(canonical_directory.as_path()),
                    sanitized_name,
                    std::fs::read(path).unwrap(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            (snapshot, paths[0] != paths[1]),
            (
                vec![
                    (true, true, "design_mock.html".to_string(), b"one".to_vec(),),
                    (true, true, "design_mock.html".to_string(), b"two".to_vec(),),
                ],
                true
            )
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
            .set_len(super::MAX_STORAGE_BYTES - super::STORAGE_FILE_OVERHEAD_BYTES - 1)
            .unwrap();

        let error = super::save(
            vec![Attachment {
                name: "sample.bin".to_string(),
                bytes: b"data".to_vec(),
                kind: AttachmentKind::File,
            }],
            Some(directory.path().to_path_buf()),
        )
        .unwrap_err();

        assert_eq!(error.to_string(), "file attachment storage is full");
    }

    fn request(name: &str, bytes: &[u8]) -> Request {
        Request {
            name: name.to_string(),
            mime_type: None,
            size: bytes.len(),
            data: STANDARD.encode(bytes),
        }
    }

    fn request_with_mime_type(name: &str, mime_type: &str, bytes: &[u8]) -> Request {
        Request {
            name: name.to_string(),
            mime_type: Some(mime_type.to_string()),
            size: bytes.len(),
            data: STANDARD.encode(bytes),
        }
    }
}
