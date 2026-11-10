use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use anyhow::Context;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::http::header::{CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use std::sync::Arc;
use thiserror::Error;

pub(super) const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
pub(super) const MAX_IMAGES_PER_MESSAGE: usize = 16;
const MAX_STORAGE_BYTES: u64 = 1024 * 1024 * 1024;
const STORAGE_FILE_OVERHEAD_BYTES: u64 = 64 * 1024;
const ATTACHMENT_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const EMPTY_MESSAGE_HEADER: &str = "Please inspect the attached image(s):\n";
const ATTACHMENT_SECTION_SEPARATOR: &str = concat!("\n", "\n", "Attached image(s):", "\n");
static STORAGE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
type DirectoryLoader = Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>;

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

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Reference {
    pub(super) source: String,
    pub(super) available: bool,
}

#[derive(Debug, Error)]
pub(super) enum ValidationError {
    #[error("a message cannot include more than 16 image attachments")]
    TooManyImages,
    #[error("image attachments exceed the 10 MiB limit")]
    TooLarge,
    #[error("unsupported image type: {0}")]
    UnsupportedType(String),
    #[error("image data is invalid")]
    InvalidData,
}

pub(super) fn validate(images: Vec<Request>) -> Result<Vec<Image>, ValidationError> {
    if images.len() > MAX_IMAGES_PER_MESSAGE {
        return Err(ValidationError::TooManyImages);
    }
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
    let directory = match directory {
        Some(directory) => directory,
        None => storage_directory()?,
    };
    create_private_directory(&directory)?;
    prune_expired(&directory)?;
    let existing_usage = directory_usage_bytes(&directory)?;
    let incoming_usage = images.iter().try_fold(0_u64, |total, image| {
        let image_usage = (image.bytes.len() as u64).checked_add(STORAGE_FILE_OVERHEAD_BYTES)?;
        total.checked_add(image_usage)
    });
    let Some(incoming_usage) = incoming_usage else {
        anyhow::bail!("image attachment storage size overflow");
    };
    if existing_usage.saturating_add(incoming_usage) > MAX_STORAGE_BYTES {
        anyhow::bail!("image attachment storage is full");
    }
    let mut paths = Vec::with_capacity(images.len());
    for image in images {
        match save_one(&directory, image) {
            Ok(path) => paths.push(path),
            Err(error) => {
                remove_files_unlocked(&paths);
                return Err(error);
            }
        }
    }
    Ok(paths)
}

pub(super) fn storage_directory() -> anyhow::Result<PathBuf> {
    let directory = crate::shared::dirs::state_dir()
        .context("user state directory is unavailable")?
        .join("armyknife")
        .join("agent-watch-images");
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            Ok(directory.canonicalize()?)
        }
        Ok(_) => anyhow::bail!("image attachment directory is not a real directory"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(directory),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn extract_references(
    content: &str,
    directory: &Path,
) -> Option<(String, Vec<Reference>)> {
    let (visible_text, listed_paths) = if let Some((visible_text, listed_paths)) =
        content.rsplit_once(ATTACHMENT_SECTION_SEPARATOR)
    {
        (visible_text.trim_end().to_owned(), listed_paths)
    } else {
        let listed_paths = content.strip_prefix(EMPTY_MESSAGE_HEADER)?;
        (String::new(), listed_paths)
    };
    let listed_paths = listed_paths.lines().collect::<Vec<_>>();
    if listed_paths.is_empty() || listed_paths.iter().any(|line| !line.starts_with("- ")) {
        return None;
    }

    let references = listed_paths
        .into_iter()
        .filter_map(|line| {
            let path = Path::new(line.strip_prefix("- ")?);
            let filename = path.file_name()?.to_str()?;
            if path.parent()? != directory || !is_valid_filename(filename) {
                return None;
            }
            Some(Reference {
                source: format!("/api/images/{filename}"),
                available: image_file_is_safe(directory, filename),
            })
        })
        .collect();
    Some((visible_text, references))
}

pub(super) fn routes() -> Router {
    routes_with(|| {
        storage_directory()
            .map_err(|error| {
                tracing::warn!(%error, "failed to resolve watch-web image storage directory");
            })
            .ok()
    })
}

fn routes_with(directory: impl Fn() -> Option<PathBuf> + Send + Sync + 'static) -> Router {
    Router::new()
        .route("/images/{name}", get(get_image))
        .with_state(Arc::new(directory) as DirectoryLoader)
}

async fn get_image(
    State(directory): State<DirectoryLoader>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    let Ok(result) = tokio::task::spawn_blocking(move || {
        let Some(directory) = directory() else {
            return Ok(None);
        };
        read_image(&directory, &name)
    })
    .await
    else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    match result {
        Ok(Some((mime_type, bytes))) => (
            StatusCode::OK,
            [
                (CONTENT_TYPE, mime_type),
                (X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            Body::from(bytes),
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::warn!(%error, "failed to read watch-web image attachment");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn read_image(directory: &Path, filename: &str) -> anyhow::Result<Option<(&'static str, Vec<u8>)>> {
    if !is_valid_filename(filename) {
        return Ok(None);
    }
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let directory = directory.canonicalize()?;
    let path = directory.join(filename);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.len() > MAX_IMAGE_BYTES as u64
        || is_expired(metadata.modified()?, SystemTime::now())
        || path.canonicalize()?.parent() != Some(&directory)
    {
        return Ok(None);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Ok(None);
    }
    let mime_type = match Path::new(filename)
        .extension()
        .and_then(|value| value.to_str())
    {
        Some("png") => "image/png",
        Some("jpg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => return Ok(None),
    };
    Ok(Some((mime_type, bytes)))
}

fn image_file_is_safe(directory: &Path, filename: &str) -> bool {
    if !is_valid_filename(filename) {
        return false;
    }
    let path = directory.join(filename);
    fs::symlink_metadata(&path).is_ok_and(|metadata| {
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() <= MAX_IMAGE_BYTES as u64
            && metadata
                .modified()
                .is_ok_and(|modified| !is_expired(modified, SystemTime::now()))
            && path
                .canonicalize()
                .is_ok_and(|canonical| canonical.parent() == Some(directory))
    })
}

fn is_valid_filename(filename: &str) -> bool {
    let path = Path::new(filename);
    if path.file_name().and_then(|name| name.to_str()) != Some(filename) {
        return false;
    }
    let Some((stem, extension)) = filename.rsplit_once('.') else {
        return false;
    };
    if !matches!(extension, "png" | "jpg" | "gif" | "webp") {
        return false;
    }
    uuid::Uuid::parse_str(stem).is_ok_and(|uuid| uuid.to_string() == stem)
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
        if is_expired(metadata.modified()?, now) {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn is_expired(modified: SystemTime, now: SystemTime) -> bool {
    now.duration_since(modified)
        .is_ok_and(|age| age >= ATTACHMENT_RETENTION)
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
    use std::fs::{self, OpenOptions};
    use std::time::SystemTime;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request as HttpRequest, StatusCode};
    use base64::Engine as _;
    use rstest::rstest;
    use tower::ServiceExt;

    use super::{
        Image, Reference, Request, ValidationError, extract_references, routes_with, validate,
    };

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
            .set_len(super::MAX_STORAGE_BYTES - super::STORAGE_FILE_OVERHEAD_BYTES - 1)
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

    #[test]
    fn extracts_only_image_paths_from_the_watch_web_storage_directory() {
        let directory = tempfile::tempdir().unwrap();
        let directory_path = directory.path().canonicalize().unwrap();
        let image_name = "00000000-0000-4000-8000-000000000001.png";
        let image_path = directory_path.join(image_name);
        fs::write(&image_path, b"fixture image bytes").unwrap();
        let missing_name = "00000000-0000-4000-8000-000000000002.webp";
        let expired_name = "00000000-0000-4000-8000-000000000004.gif";
        let expired_path = directory_path.join(expired_name);
        fs::write(&expired_path, b"expired fixture image").unwrap();
        fs::File::open(&expired_path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH)
            .unwrap();
        let external_path = directory_path
            .parent()
            .unwrap()
            .join("00000000-0000-4000-8000-000000000003.png");
        let content = format!(
            indoc::indoc! {"
                Inspect these images.

                Attached image(s):
                - {}
                - {}
                - {}
                - {}
            "},
            image_path.display(),
            directory_path.join(missing_name).display(),
            expired_path.display(),
            external_path.display(),
        );

        assert_eq!(
            extract_references(&content, &directory_path),
            Some((
                "Inspect these images.".to_string(),
                vec![
                    Reference {
                        source: format!("/api/images/{image_name}"),
                        available: true,
                    },
                    Reference {
                        source: format!("/api/images/{missing_name}"),
                        available: false,
                    },
                    Reference {
                        source: format!("/api/images/{expired_name}"),
                        available: false,
                    },
                ],
            )),
        );
    }

    #[tokio::test]
    async fn image_route_serves_saved_images_and_rejects_other_paths() {
        let directory = tempfile::tempdir().unwrap();
        let directory_path = directory.path().canonicalize().unwrap();
        let image_name = "00000000-0000-4000-8000-000000000001.png";
        fs::write(directory_path.join(image_name), b"fixture image bytes").unwrap();
        let expired_name = "00000000-0000-4000-8000-000000000003.png";
        let expired_path = directory_path.join(expired_name);
        fs::write(&expired_path, b"expired fixture image").unwrap();
        fs::File::open(&expired_path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH)
            .unwrap();
        let route_directory = directory_path.clone();
        let router = routes_with(move || Some(route_directory.clone()));
        let response = router
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("/images/{image_name}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let actual = (
            response.status(),
            response.headers().get("content-type").cloned(),
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        );

        assert_eq!(
            actual,
            (
                StatusCode::OK,
                Some("image/png".parse().unwrap()),
                b"fixture image bytes".to_vec(),
            ),
        );

        let response = router
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("/images/{expired_name}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = router
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri("/images/../../outside.png")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        #[cfg(unix)]
        {
            let outside_directory = tempfile::tempdir().unwrap();
            let outside_path = outside_directory.path().join("fixture.png");
            fs::write(&outside_path, b"outside image bytes").unwrap();
            let symlink_name = "00000000-0000-4000-8000-000000000002.png";
            std::os::unix::fs::symlink(outside_path, directory_path.join(symlink_name)).unwrap();
            let response = router
                .oneshot(
                    HttpRequest::builder()
                        .uri(format!("/images/{symlink_name}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
    }

    fn request(mime_type: &str, bytes: &[u8]) -> Request {
        Request {
            mime_type: mime_type.to_string(),
            size: bytes.len(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        }
    }
}
