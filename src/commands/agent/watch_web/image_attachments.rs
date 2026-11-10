use std::fs::{self, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::Context;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::http::header::{CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use std::sync::Arc;

pub(super) const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
pub(super) const MAX_IMAGES_PER_MESSAGE: usize = 16;
const MAX_SAVED_IMAGE_BYTES: usize = 50 * 1024 * 1024;
const ATTACHMENT_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const EMPTY_MESSAGE_HEADERS: [&str; 2] = [
    "Please inspect the attached image(s):\n",
    "Please inspect the attached file(s):\n",
];
const ATTACHMENT_SECTION_SEPARATORS: [&str; 2] = [
    concat!("\n", "\n", "Attached image(s):", "\n"),
    concat!("\n", "\n", "Attached file(s):", "\n"),
];
type DirectoryLoader = Arc<dyn Fn() -> Option<PathBuf> + Send + Sync>;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Reference {
    Image { source: String, available: bool },
    File { name: String },
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
    let section = ATTACHMENT_SECTION_SEPARATORS
        .iter()
        .filter_map(|separator| {
            content
                .rfind(separator)
                .map(|offset| (offset, &content[offset + separator.len()..]))
        })
        .max_by_key(|(offset, _)| *offset);
    let (visible_text, listed_paths) = if let Some((offset, listed_paths)) = section {
        (content[..offset].trim_end().to_owned(), listed_paths)
    } else {
        let (_, listed_paths) = EMPTY_MESSAGE_HEADERS
            .iter()
            .find_map(|header| content.strip_prefix(header).map(|paths| (*header, paths)))?;
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
            if path.parent()? != directory {
                return None;
            }
            if is_valid_image_filename(filename) {
                return Some(Reference::Image {
                    source: format!("/api/images/{filename}"),
                    available: image_file_is_safe(directory, filename),
                });
            }
            Some(Reference::File {
                name: original_filename(filename)?.to_owned(),
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
    if !is_valid_image_filename(filename) {
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
    if metadata.len() > MAX_SAVED_IMAGE_BYTES as u64
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
    file.take(MAX_SAVED_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SAVED_IMAGE_BYTES {
        return Ok(None);
    }
    let Some(mime_type) = image_mime_type_from_bytes(&bytes) else {
        return Ok(None);
    };
    Ok(Some((mime_type, bytes)))
}

fn image_file_is_safe(directory: &Path, filename: &str) -> bool {
    if !is_valid_image_filename(filename) {
        return false;
    }
    let path = directory.join(filename);
    fs::symlink_metadata(&path).is_ok_and(|metadata| {
        let expired = is_known_expired(metadata.modified(), SystemTime::now());
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() <= MAX_SAVED_IMAGE_BYTES as u64
            && !expired
            && path
                .canonicalize()
                .is_ok_and(|canonical| canonical.parent() == Some(directory))
    })
}

fn is_valid_image_filename(filename: &str) -> bool {
    let path = Path::new(filename);
    if path.file_name().and_then(|name| name.to_str()) != Some(filename) {
        return false;
    }
    if image_mime_type(filename).is_none() {
        return false;
    }
    let Some((stem, original_name)) = filename.split_once('_') else {
        let Some((stem, _)) = filename.rsplit_once('.') else {
            return false;
        };
        return is_uuid(stem);
    };
    is_uuid(stem) && is_valid_sanitized_name(original_name)
}

fn original_filename(filename: &str) -> Option<&str> {
    let (prefix, name) = filename.split_once('_')?;
    (is_uuid(prefix) && is_valid_sanitized_name(name)).then_some(name)
}

fn is_valid_sanitized_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 180
        && !name.starts_with('.')
        && !name.ends_with('.')
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_')
        })
}

fn is_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|uuid| uuid.to_string() == value)
}

fn image_mime_type(filename: &str) -> Option<&'static str> {
    match Path::new(filename)
        .extension()
        .and_then(|value| value.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn image_mime_type_from_bytes(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]) {
        Some("image/png")
    } else if bytes.starts_with(&[255, 216, 255]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

fn is_expired(modified: SystemTime, now: SystemTime) -> bool {
    now.duration_since(modified)
        .is_ok_and(|age| age >= ATTACHMENT_RETENTION)
}

fn is_known_expired(modified: std::io::Result<SystemTime>, now: SystemTime) -> bool {
    match modified {
        Ok(modified) => is_expired(modified, now),
        // The image route logs modification-time errors after the reference initiates a fetch.
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::SystemTime;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request as HttpRequest, StatusCode};
    use rstest::rstest;
    use tower::ServiceExt;

    use super::{
        ATTACHMENT_RETENTION, Reference, extract_references, is_known_expired, routes_with,
    };

    #[rstest]
    #[case::expired(
        Ok(SystemTime::UNIX_EPOCH),
        SystemTime::UNIX_EPOCH + ATTACHMENT_RETENTION,
        true,
    )]
    #[case::fresh(Ok(SystemTime::UNIX_EPOCH), SystemTime::UNIX_EPOCH, false)]
    #[case::unknown_modification_time(
        Err(std::io::Error::other("metadata unavailable")),
        SystemTime::UNIX_EPOCH,
        false
    )]
    fn only_marks_images_expired_when_their_modification_time_is_known(
        #[case] modified: std::io::Result<SystemTime>,
        #[case] now: SystemTime,
        #[case] expected: bool,
    ) {
        assert_eq!(is_known_expired(modified, now), expected);
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
                    Reference::Image {
                        source: format!("/api/images/{image_name}"),
                        available: true,
                    },
                    Reference::Image {
                        source: format!("/api/images/{missing_name}"),
                        available: false,
                    },
                    Reference::Image {
                        source: format!("/api/images/{expired_name}"),
                        available: false,
                    },
                ],
            )),
        );
    }

    #[rstest]
    #[case::legacy_image_section(
        "Attached image(s):",
        "Inspect the attachment.",
        "00000000-0000-4000-8000-000000000001.png",
        Reference::Image {
            source: "/api/images/00000000-0000-4000-8000-000000000001.png".to_string(),
            available: true,
        },
        "Inspect the attachment.",
    )]
    #[case::legacy_empty_image_header(
        "Please inspect the attached image(s):",
        "",
        "00000000-0000-4000-8000-000000000002.png",
        Reference::Image {
            source: "/api/images/00000000-0000-4000-8000-000000000002.png".to_string(),
            available: true,
        },
        "",
    )]
    #[case::new_file_section(
        "Attached file(s):",
        "Inspect the attachment.",
        "00000000-0000-4000-8000-000000000003_design-mock.html",
        Reference::File {
            name: "design-mock.html".to_string(),
        },
        "Inspect the attachment.",
    )]
    #[case::new_empty_file_header(
        "Please inspect the attached file(s):",
        "",
        "00000000-0000-4000-8000-000000000004_design-mock.html",
        Reference::File {
            name: "design-mock.html".to_string(),
        },
        "",
    )]
    fn extracts_legacy_and_generic_attachment_headings(
        #[case] header: &str,
        #[case] visible_text: &str,
        #[case] filename: &str,
        #[case] expected_reference: Reference,
        #[case] expected_text: &str,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let directory_path = directory.path().canonicalize().unwrap();
        let image_path = directory_path.join(filename);
        fs::write(&image_path, b"attachment bytes").unwrap();
        let content = if visible_text.is_empty() {
            indoc::formatdoc! {"
                {header}
                - {}
            ", image_path.display()}
        } else {
            indoc::formatdoc! {"
                {visible_text}

                {header}
                - {}
            ", image_path.display()}
        };

        assert_eq!(
            extract_references(&content, &directory_path),
            Some((expected_text.to_string(), vec![expected_reference])),
        );
    }

    #[test]
    fn extracts_images_and_files_from_a_generic_attachment_section() {
        let directory = tempfile::tempdir().unwrap();
        let directory_path = directory.path().canonicalize().unwrap();
        let image_name = "00000000-0000-4000-8000-000000000005_screenshot.jpeg";
        let file_name = "00000000-0000-4000-8000-000000000006_design-mock.html";
        fs::write(directory_path.join(image_name), b"image bytes").unwrap();
        fs::write(directory_path.join(file_name), b"<main>Mock</main>").unwrap();
        let content = indoc::formatdoc! {"
            Inspect the attachments.

            Attached file(s):
            - {}
            - {}
        ",
            directory_path.join(image_name).display(),
            directory_path.join(file_name).display(),
        };

        assert_eq!(
            extract_references(&content, &directory_path),
            Some((
                "Inspect the attachments.".to_string(),
                vec![
                    Reference::Image {
                        source: format!("/api/images/{image_name}"),
                        available: true,
                    },
                    Reference::File {
                        name: "design-mock.html".to_string(),
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
        let image_bytes = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        fs::write(directory_path.join(image_name), image_bytes).unwrap();
        let uploaded_image_name = "00000000-0000-4000-8000-000000000004_uploaded.jpeg";
        let uploaded_image_bytes = b"\xff\xd8\xfffixture";
        fs::write(
            directory_path.join(uploaded_image_name),
            uploaded_image_bytes,
        )
        .unwrap();
        let invalid_image_name = "00000000-0000-4000-8000-000000000007_invalid.png";
        fs::write(directory_path.join(invalid_image_name), b"plain text").unwrap();
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
                image_bytes.to_vec(),
            ),
        );

        let response = router
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("/images/{uploaded_image_name}"))
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
                Some("image/jpeg".parse().unwrap()),
                uploaded_image_bytes.to_vec(),
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
                    .uri(format!("/images/{invalid_image_name}"))
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
}
