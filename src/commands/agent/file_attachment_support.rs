pub(crate) const MAX_AGENT_FILE_BYTES: u64 = 10 * 1024 * 1024;

const TEXT_EXTENSIONS: &[&str] = &[
    "txt",
    "text",
    "json",
    "yaml",
    "yml",
    "toml",
    "csv",
    "tsv",
    "xml",
    "rs",
    "ts",
    "tsx",
    "js",
    "jsx",
    "mjs",
    "cjs",
    "py",
    "sh",
    "bash",
    "zsh",
    "go",
    "java",
    "kt",
    "swift",
    "c",
    "h",
    "cpp",
    "hpp",
    "css",
    "scss",
    "sql",
    "graphql",
    "gql",
    "ini",
    "conf",
    "log",
    "env",
    "properties",
    "rb",
    "php",
    "cs",
    "scala",
    "clj",
    "ex",
    "exs",
    "hs",
    "pl",
    "pm",
    "r",
    "jl",
    "lua",
    "vim",
    "el",
    "sol",
    "tf",
    "hcl",
    "proto",
    "prisma",
    "patch",
    "diff",
    "lock",
];

pub(crate) fn image_format_from_bytes(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FilePreviewType {
    Image,
    Markdown,
    Text,
    Video,
    Audio,
    Pdf,
    Html,
    Download,
}

pub(crate) fn file_preview_type(name: &str, bytes: &[u8]) -> FilePreviewType {
    let extension = std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if let Some((_, image_extension)) = image_format_from_bytes(bytes)
        && (extension.is_empty()
            || matches!(
                (extension.as_str(), image_extension),
                ("png", "png")
                    | ("jpg", "jpg")
                    | ("jpeg", "jpg")
                    | ("gif", "gif")
                    | ("webp", "webp")
            ))
    {
        return FilePreviewType::Image;
    }

    if extension == "pdf" && bytes.starts_with(b"%PDF-") {
        return FilePreviewType::Pdf;
    }
    if matches!(extension.as_str(), "html" | "htm") && std::str::from_utf8(bytes).is_ok() {
        return FilePreviewType::Html;
    }
    if matches!(extension.as_str(), "mp4" | "mov") && bytes.get(4..8) == Some(b"ftyp") {
        return FilePreviewType::Video;
    }
    if matches!(extension.as_str(), "m4a" | "mp3")
        && (bytes.get(4..8) == Some(b"ftyp")
            || bytes.starts_with(b"ID3")
            || bytes
                .get(..2)
                .is_some_and(|prefix| prefix[0] == 0xff && prefix[1] & 0xe0 == 0xe0))
    {
        return FilePreviewType::Audio;
    }

    if std::str::from_utf8(bytes).is_err() {
        return FilePreviewType::Download;
    }
    if matches!(extension.as_str(), "md" | "markdown") {
        return FilePreviewType::Markdown;
    }
    if is_text_file_name(name) {
        return FilePreviewType::Text;
    }
    FilePreviewType::Download
}

pub(crate) fn preview_type_from_mime(name: &str, mime_type: Option<&str>) -> FilePreviewType {
    match mime_type {
        Some("image/png") if extension_is(name, &["png"]) || extension_is_empty(name) => {
            FilePreviewType::Image
        }
        Some("image/jpeg") if extension_is(name, &["jpg", "jpeg"]) || extension_is_empty(name) => {
            FilePreviewType::Image
        }
        Some("image/gif") if extension_is(name, &["gif"]) || extension_is_empty(name) => {
            FilePreviewType::Image
        }
        Some("image/webp") if extension_is(name, &["webp"]) || extension_is_empty(name) => {
            FilePreviewType::Image
        }
        Some("application/pdf") if extension_is(name, &["pdf"]) => FilePreviewType::Pdf,
        Some("text/html") if extension_is(name, &["html", "htm"]) => FilePreviewType::Html,
        Some("video/mp4" | "video/quicktime") if extension_is(name, &["mp4", "mov"]) => {
            FilePreviewType::Video
        }
        Some("audio/mpeg" | "audio/mp4") if extension_is(name, &["mp3", "m4a"]) => {
            FilePreviewType::Audio
        }
        Some("text/markdown") if extension_is(name, &["md", "markdown"]) => {
            FilePreviewType::Markdown
        }
        Some("text/plain") if is_text_file_name(name) => FilePreviewType::Text,
        _ => FilePreviewType::Download,
    }
}

fn extension_is(name: &str, extensions: &[&str]) -> bool {
    std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extensions.contains(&extension.to_ascii_lowercase().as_str()))
}

fn extension_is_empty(name: &str) -> bool {
    std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_none()
}

fn is_text_file_name(name: &str) -> bool {
    let basename = std::path::Path::new(name)
        .file_name()
        .and_then(|basename| basename.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension = std::path::Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        basename.as_str(),
        "dockerfile" | "makefile" | "justfile" | "containerfile"
    ) || TEXT_EXTENSIONS.contains(&extension.as_str())
}

pub(crate) fn sanitize_filename(name: &str) -> String {
    let basename = name.replace('\\', "/");
    let basename = basename.rsplit('/').next().unwrap_or_default();
    sanitize_basename(basename)
}

pub(crate) fn sanitize_basename(name: &str) -> String {
    let sanitized: String = name
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
        "attachment".to_owned()
    } else {
        sanitized.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{FilePreviewType, file_preview_type};

    #[rstest]
    #[case::markdown("notes.md", b"# Heading", FilePreviewType::Markdown)]
    #[case::source_code("main.rs", b"fn main() {}", FilePreviewType::Text)]
    #[case::html("page.html", b"<html><body>safe</body></html>", FilePreviewType::Html)]
    #[case::pdf("report.pdf", b"%PDF-1.7", FilePreviewType::Pdf)]
    #[case::mismatched_pdf("report.pdf", b"plain text", FilePreviewType::Download)]
    #[case::binary_text("source.rs", &[0xff, 0xfe], FilePreviewType::Download)]
    #[case::mismatched_image("photo.png", b"ordinary text", FilePreviewType::Download)]
    fn classifies_files_from_name_and_content(
        #[case] name: &str,
        #[case] bytes: &[u8],
        #[case] expected: FilePreviewType,
    ) {
        assert_eq!(file_preview_type(name, bytes), expected);
    }
}
