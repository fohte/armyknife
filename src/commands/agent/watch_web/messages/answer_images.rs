use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::commands::agent::file::{self, storage_root};
use crate::commands::agent::file_attachment_support::{
    MAX_AGENT_FILE_BYTES, inspect_bounded_image, read_bounded_image,
};

const MAX_IMAGES_PER_ANSWER: usize = 16;
const MAX_ANSWER_IMAGE_BYTES: u64 = 50 * 1024 * 1024;

pub(super) fn attach(session_id: &str, conversation: &mut super::ConversationResponse) {
    attach_with_storage_root_result(session_id, conversation, storage_root());
}

fn attach_with_storage_root_result(
    session_id: &str,
    conversation: &mut super::ConversationResponse,
    storage_root: Result<PathBuf>,
) {
    match storage_root {
        Ok(storage_root) => attach_with_storage_root(session_id, conversation, &storage_root),
        Err(error) => {
            conversation.message_image_sources.clear();
            tracing::warn!(%error, "failed to locate assistant answer image storage");
            attach_unavailable_images(conversation);
        }
    }
}

fn attach_with_storage_root(
    session_id: &str,
    conversation: &mut super::ConversationResponse,
    storage_root: &Path,
) {
    conversation.message_image_sources.clear();
    for (message_index, message) in conversation.messages.iter().enumerate() {
        let super::ChatItem::Assistant { text } = message else {
            continue;
        };
        let paths = markdown_image_paths(text);
        if paths.is_empty() {
            continue;
        }
        let mut sources = BTreeMap::new();
        let mut total_size = 0_u64;
        for (path_index, path) in paths.iter().enumerate() {
            if path_index >= MAX_IMAGES_PER_ANSWER || has_markdown_destination_escape(path) {
                sources.insert(path.clone(), None);
                continue;
            }
            let key = image_key(session_id, text, path);
            let copied = match snapshot_image(
                storage_root,
                session_id,
                &key,
                Path::new(path),
                MAX_ANSWER_IMAGE_BYTES - total_size,
            ) {
                Ok(copied) => copied,
                Err(error) => {
                    tracing::warn!(%error, path, "failed to snapshot assistant answer image");
                    None
                }
            };
            let source = copied.and_then(|(size, _mime_type)| {
                let next_total = total_size.checked_add(size as u64)?;
                if next_total > MAX_ANSWER_IMAGE_BYTES {
                    return None;
                }
                total_size = next_total;
                Some(format!("/api/sessions/{session_id}/answer-images/{key}"))
            });
            sources.insert(path.clone(), source);
        }
        conversation
            .message_image_sources
            .insert(conversation.start + message_index, sources);
    }
}

fn attach_unavailable_images(conversation: &mut super::ConversationResponse) {
    for (message_index, message) in conversation.messages.iter().enumerate() {
        let super::ChatItem::Assistant { text } = message else {
            continue;
        };
        let sources: BTreeMap<String, Option<String>> = markdown_image_paths(text)
            .into_iter()
            .map(|path| (path, None))
            .collect();
        if sources.is_empty() {
            continue;
        }
        conversation
            .message_image_sources
            .insert(conversation.start + message_index, sources);
    }
}

fn image_key(session_id: &str, answer: &str, path: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(session_id.as_bytes());
    digest.update([0]);
    digest.update(answer.as_bytes());
    digest.update([0]);
    digest.update(path.as_bytes());
    crate::shared::hex::encode(&digest.finalize())
}

fn snapshot_image(
    storage_root: &Path,
    session_id: &str,
    key: &str,
    source: &Path,
    remaining_bytes: u64,
) -> Result<Option<(usize, &'static str)>> {
    if !source.is_absolute() {
        return Ok(None);
    }
    let directory = file::session_directory(storage_root, session_id)?;
    let stored_path = stored_path(&directory, key);
    if let Some(image) = inspect_bounded_image(&stored_path, MAX_AGENT_FILE_BYTES)? {
        if image.size > remaining_bytes {
            return Ok(None);
        }
        return Ok(Some((image.size as usize, image.mime_type)));
    }
    match fs::symlink_metadata(&stored_path) {
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let image = match read_bounded_image(source, MAX_AGENT_FILE_BYTES) {
        Ok(Some(image)) => image,
        Ok(None) => return Ok(None),
        Err(error) => {
            tracing::warn!(%error, "failed to read assistant answer image source");
            return Ok(None);
        }
    };
    if image.bytes.len() as u64 > remaining_bytes {
        return Ok(None);
    }
    match file::write_new_file(&stored_path, &image.bytes) {
        Ok(()) => Ok(Some((image.bytes.len(), image.mime_type))),
        Err(error) if is_already_exists(&error) => {
            Ok(inspect_bounded_image(&stored_path, MAX_AGENT_FILE_BYTES)?
                .map(|saved| (saved.size as usize, saved.mime_type)))
        }
        Err(error) => Err(error).with_context(|| "saving answer image snapshot"),
    }
}

fn stored_path(directory: &Path, key: &str) -> PathBuf {
    directory.join(format!("answer-image-{key}"))
}

fn has_markdown_destination_escape(path: &str) -> bool {
    path.as_bytes()
        .windows(2)
        .any(|bytes| bytes[0] == b'\\' && bytes[1].is_ascii_punctuation())
}

pub(in crate::commands::agent::watch_web) fn read_snapshot(
    storage_root: &Path,
    session_id: &str,
    key: &str,
) -> Result<Option<(Vec<u8>, &'static str)>> {
    if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(None);
    }
    let Ok(directory) = file::session_directory(storage_root, session_id) else {
        return Ok(None);
    };
    Ok(
        read_bounded_image(&stored_path(&directory, key), MAX_AGENT_FILE_BYTES)?
            .map(|image| (image.bytes, image.mime_type)),
    )
}

fn is_already_exists(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::AlreadyExists)
}

fn markdown_image_paths(text: &str) -> Vec<String> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    let mut in_fence: Option<(u8, usize)> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let marker = trimmed.as_bytes().first().copied();
        if let Some((fence_char, fence_len)) = in_fence {
            if indent <= 3
                && marker == Some(fence_char)
                && trimmed
                    .bytes()
                    .take_while(|byte| *byte == fence_char)
                    .count()
                    >= fence_len
            {
                in_fence = None;
            }
            continue;
        }
        if indent <= 3 && marker.is_some_and(|byte| byte == b'`' || byte == b'~') {
            let marker = marker.unwrap_or_default();
            let marker_len = trimmed.bytes().take_while(|byte| *byte == marker).count();
            if marker_len >= 3 {
                in_fence = Some((marker, marker_len));
                continue;
            }
        }
        extract_inline_images(line, &mut paths, &mut seen);
    }
    paths
}

fn extract_inline_images(line: &str, paths: &mut Vec<String>, seen: &mut HashSet<String>) {
    let bytes = line.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes[offset] == b'`' {
            let run = bytes[offset..]
                .iter()
                .take_while(|byte| **byte == b'`')
                .count();
            if let Some(close) = find_backtick_run(bytes, offset + run, run) {
                offset = close + run;
                continue;
            }
            offset += run;
            continue;
        }
        if !bytes[offset..].starts_with(b"![") {
            offset += 1;
            continue;
        }
        let Some(close_alt) = find_unescaped(bytes, offset + 2, b']') else {
            offset += 2;
            continue;
        };
        if bytes.get(close_alt + 1) != Some(&b'(') {
            offset = close_alt + 1;
            continue;
        }
        let mut destination_start = close_alt + 2;
        while bytes
            .get(destination_start)
            .is_some_and(u8::is_ascii_whitespace)
        {
            destination_start += 1;
        }
        let angle_destination = bytes.get(destination_start) == Some(&b'<');
        let (destination, close_destination) = if angle_destination {
            let start = destination_start + 1;
            let Some(end) = find_unescaped(bytes, start, b'>') else {
                offset = destination_start + 1;
                continue;
            };
            (start, end)
        } else {
            let start = destination_start;
            (start, find_unbracketed_destination_end(bytes, start))
        };
        let mut close = if angle_destination {
            close_destination + 1
        } else {
            close_destination
        };
        let mut has_title_separator = false;
        while bytes.get(close).is_some_and(u8::is_ascii_whitespace) {
            has_title_separator = true;
            close += 1;
        }
        if has_title_separator
            && bytes.get(close) != Some(&b')')
            && let Some(title_end) = find_markdown_title_end(bytes, close)
        {
            close = title_end;
            while bytes.get(close).is_some_and(u8::is_ascii_whitespace) {
                close += 1;
            }
        }
        if bytes.get(close) == Some(&b')') && close_destination > destination {
            if let Ok(value) = std::str::from_utf8(&bytes[destination..close_destination])
                && !value.starts_with("//")
                && Path::new(value).is_absolute()
                && seen.insert(value.to_owned())
            {
                paths.push(value.to_owned());
            }
            offset = close + 1;
        } else {
            offset = close_alt + 2;
        }
    }
}

fn find_unbracketed_destination_end(bytes: &[u8], start: usize) -> usize {
    let mut offset = start;
    let mut nesting = 0_usize;
    while let Some(byte) = bytes.get(offset) {
        match *byte {
            b'\\' if offset + 1 < bytes.len() => offset += 2,
            b'(' => {
                nesting += 1;
                offset += 1;
            }
            b')' if nesting > 0 => {
                nesting -= 1;
                offset += 1;
            }
            b')' => break,
            byte if byte.is_ascii_whitespace() => break,
            _ => offset += 1,
        }
    }
    offset
}

fn find_markdown_title_end(bytes: &[u8], start: usize) -> Option<usize> {
    let opener = *bytes.get(start)?;
    let quote = match opener {
        b'\'' | b'"' => Some(opener),
        b'(' => None,
        _ => return None,
    };
    let mut nesting = usize::from(opener == b'(');
    let mut offset = start + 1;
    while let Some(byte) = bytes.get(offset) {
        if *byte == b'\\' && offset + 1 < bytes.len() {
            offset += 2;
            continue;
        }
        if quote == Some(*byte) {
            return Some(offset + 1);
        }
        match *byte {
            b'(' if quote.is_none() => nesting += 1,
            b')' if quote.is_none() => {
                nesting -= 1;
                if nesting == 0 {
                    return Some(offset + 1);
                }
            }
            _ => {}
        }
        offset += 1;
    }
    None
}

fn find_unescaped(bytes: &[u8], start: usize, needle: u8) -> Option<usize> {
    let mut offset = start;
    while let Some(byte) = bytes.get(offset) {
        if *byte == needle && (offset == start || bytes[offset - 1] != b'\\') {
            return Some(offset);
        }
        offset += 1;
    }
    None
}

fn find_backtick_run(bytes: &[u8], start: usize, run: usize) -> Option<usize> {
    let mut offset = start;
    while offset < bytes.len() {
        if bytes[offset] == b'`' {
            let found = bytes[offset..]
                .iter()
                .take_while(|byte| **byte == b'`')
                .count();
            if found == run {
                return Some(offset);
            }
            offset += found;
        } else {
            offset += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::Write;

    use tempfile::TempDir;

    use super::{
        MAX_AGENT_FILE_BYTES, MAX_IMAGES_PER_ANSWER, attach_with_storage_root,
        attach_with_storage_root_result, image_key, markdown_image_paths,
    };
    use crate::commands::agent::file::{self, remove_session_storage};
    use crate::commands::agent::watch_web::messages::{ChatItem, ConversationResponse};

    const PNG_HEADER: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10];

    fn image_file(directory: &TempDir, name: &str, size: u64) -> std::path::PathBuf {
        let path = directory.path().join(name);
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(PNG_HEADER).unwrap();
        file.set_len(size).unwrap();
        path
    }

    fn assistant(text: String) -> ChatItem {
        ChatItem::Assistant { text }
    }

    fn conversation(text: String) -> ConversationResponse {
        ConversationResponse {
            messages: vec![assistant(text)],
            ..ConversationResponse::default()
        }
    }

    #[test]
    fn extracts_only_absolute_markdown_image_paths_outside_code() {
        assert_eq!(
            markdown_image_paths(indoc::indoc! {"
                ![plot](</tmp/fixture image.png>) and ![](/tmp/画像.png) and ![](relative.png)
                `![](/tmp/inline.png)`
                ```md
                ![](/tmp/fenced.png)
                ```
                ![](/tmp/final.webp)
            "}),
            vec!["/tmp/fixture image.png", "/tmp/画像.png", "/tmp/final.webp"],
        );
    }

    #[test]
    fn extracts_paths_with_parentheses_and_markdown_titles() {
        assert_eq!(
            markdown_image_paths(indoc::indoc! {"
                ![](/tmp/fixture(v1).png)
                ![](/tmp/titled.png \"fixture title\")
            "}),
            vec!["/tmp/fixture(v1).png", "/tmp/titled.png"],
        );
    }

    #[test]
    fn leaves_scheme_relative_images_for_the_browser() {
        let storage = tempfile::tempdir().unwrap();
        let mut conversation =
            conversation("![](<//images.example.invalid/fixture.png>)".to_owned());

        attach_with_storage_root("fixture-session", &mut conversation, storage.path());

        assert_eq!(conversation.message_image_sources, BTreeMap::new());
    }

    #[test]
    fn keeps_the_first_snapshot_for_the_same_answer_and_path() {
        let sources = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let path = image_file(&sources, "fixture.png", 8);
        let original = fs::read(&path).unwrap();
        let answer = format!("![](<{}>)", path.display());
        let mut messages = conversation(answer.clone());

        attach_with_storage_root("fixture-session", &mut messages, storage.path());
        fs::write(&path, b"changed source").unwrap();
        let mut reloaded_messages = conversation(answer.clone());
        attach_with_storage_root("fixture-session", &mut reloaded_messages, storage.path());
        fs::remove_file(&path).unwrap();
        let mut deleted_source_messages = conversation(answer.clone());
        attach_with_storage_root(
            "fixture-session",
            &mut deleted_source_messages,
            storage.path(),
        );

        let key = deleted_source_messages.message_image_sources[&0][&path.display().to_string()]
            .as_deref()
            .and_then(|url| url.rsplit('/').next())
            .unwrap()
            .to_owned();
        let snapshot = super::read_snapshot(storage.path(), "fixture-session", &key)
            .unwrap()
            .unwrap();
        let expected_sources = messages.message_image_sources.clone();

        assert_eq!(
            (
                messages.message_image_sources,
                reloaded_messages.message_image_sources,
                deleted_source_messages.message_image_sources,
                snapshot,
            ),
            (
                expected_sources.clone(),
                expected_sources.clone(),
                expected_sources,
                (original, "image/png"),
            ),
        );
    }

    #[test]
    fn snapshots_images_with_spaces_and_non_ascii_path_characters() {
        let sources = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let path = image_file(&sources, "画像 samples final image.png", 8);
        let answer = format!("![](<{}>)", path.display());
        let mut messages = conversation(answer.clone());

        attach_with_storage_root("fixture-session", &mut messages, storage.path());

        let key = image_key("fixture-session", &answer, &path.display().to_string());
        assert_eq!(
            messages.message_image_sources,
            BTreeMap::from([(
                0,
                BTreeMap::from([(
                    path.display().to_string(),
                    Some(format!("/api/sessions/fixture-session/answer-images/{key}")),
                )]),
            )]),
        );
    }

    #[test]
    fn does_not_snapshot_markdown_escaped_destinations() {
        let sources = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let path = image_file(&sources, r"fixture\(v1\).png", 8);
        let answer = format!("![](<{}>)", path.display());
        let mut conversation = conversation(answer.clone());

        attach_with_storage_root("fixture-session", &mut conversation, storage.path());

        let key = image_key("fixture-session", &answer, &path.display().to_string());
        let snapshot = super::read_snapshot(storage.path(), "fixture-session", &key).unwrap();
        assert_eq!(
            (conversation.message_image_sources, snapshot),
            (
                BTreeMap::from([(0, BTreeMap::from([(path.display().to_string(), None)]),)]),
                None,
            ),
        );
    }

    #[test]
    fn removes_answer_images_with_session_storage() {
        let sources = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let path = image_file(&sources, "fixture.png", 8);
        let mut conversation = conversation(format!("![](<{}>)", path.display()));
        attach_with_storage_root("fixture-session", &mut conversation, storage.path());
        let session_storage = file::session_directory(storage.path(), "fixture-session").unwrap();

        remove_session_storage(storage.path(), "fixture-session").unwrap();
        let removed_storage = fs::symlink_metadata(session_storage).unwrap_err().kind();

        assert_eq!(
            (conversation.message_image_sources.len(), removed_storage,),
            (1, std::io::ErrorKind::NotFound),
        );
    }

    #[test]
    fn marks_unreadable_sources_unavailable_without_failing_the_conversation() {
        let sources = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let parent_file = sources.path().join("not-a-directory");
        fs::write(&parent_file, b"fixture").unwrap();
        let source = parent_file.join("fixture.png");
        let mut conversation = conversation(format!("![](<{}>)", source.display()));

        attach_with_storage_root("fixture-session", &mut conversation, storage.path());

        assert_eq!(
            conversation.message_image_sources,
            BTreeMap::from([(0, BTreeMap::from([(source.display().to_string(), None)]),)]),
        );
    }

    #[test]
    fn storage_errors_make_only_assistant_images_unavailable() {
        let sources = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let path = image_file(&sources, "fixture.png", 8);
        let invalid_storage_root = storage.path().join("not-a-directory");
        fs::write(&invalid_storage_root, b"fixture").unwrap();
        let mut conversation = conversation(format!("![](<{}>)", path.display()));

        attach_with_storage_root("fixture-session", &mut conversation, &invalid_storage_root);

        assert_eq!(
            conversation.message_image_sources,
            BTreeMap::from([(0, BTreeMap::from([(path.display().to_string(), None)]),)]),
        );
    }

    #[test]
    fn storage_root_errors_make_assistant_images_unavailable() {
        let mut conversation = conversation("![](/tmp/fixture.png)".to_owned());

        attach_with_storage_root_result(
            "fixture-session",
            &mut conversation,
            Err(anyhow::anyhow!("state directory unavailable")),
        );

        assert_eq!(
            conversation.message_image_sources,
            BTreeMap::from([(0, BTreeMap::from([("/tmp/fixture.png".to_owned(), None)]),)]),
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_images_oversized_files_and_symlinks() {
        use std::os::unix::fs::symlink;

        let sources = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let text_path = sources.path().join("fixture.txt");
        fs::write(&text_path, b"not an image").unwrap();
        let oversized_path = image_file(&sources, "oversized.png", super::MAX_AGENT_FILE_BYTES + 1);
        let actual_image = image_file(&sources, "actual.png", 8);
        let symlink_path = sources.path().join("symlink.png");
        symlink(&actual_image, &symlink_path).unwrap();
        let relative = "relative.png";
        let tilde = "~/fixture.png";
        let answer = format!(
            "![]({}) ![]({}) ![]({}) ![]({relative}) ![]({tilde})",
            text_path.display(),
            oversized_path.display(),
            symlink_path.display(),
        );
        let mut messages = conversation(answer);

        attach_with_storage_root("fixture-session", &mut messages, storage.path());

        let sources = &messages.message_image_sources[&0];
        assert_eq!(
            sources,
            &[
                (text_path.display().to_string(), None),
                (oversized_path.display().to_string(), None),
                (symlink_path.display().to_string(), None),
            ]
            .into(),
        );
    }

    #[test]
    fn enforces_per_answer_image_count() {
        let sources = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let count_paths = (0..=MAX_IMAGES_PER_ANSWER)
            .map(|index| image_file(&sources, &format!("count-{index}.png"), 8))
            .collect::<Vec<_>>();
        let count_answer = count_paths
            .iter()
            .map(|path| format!("![](<{}>)", path.display()))
            .collect::<Vec<_>>()
            .join(" ");
        let mut count_messages = conversation(count_answer.clone());
        attach_with_storage_root("count-session", &mut count_messages, storage.path());
        let expected_count_sources = count_paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let path = path.display().to_string();
                let source = (index < MAX_IMAGES_PER_ANSWER).then(|| {
                    let key = image_key("count-session", &count_answer, &path);
                    format!("/api/sessions/count-session/answer-images/{key}")
                });
                (path, source)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let actual_count_sources = &count_messages.message_image_sources[&0];
        let count_session_storage =
            file::session_directory(storage.path(), "count-session").unwrap();
        assert_eq!(
            (
                actual_count_sources,
                count_session_storage.read_dir().unwrap().count(),
            ),
            (&expected_count_sources, MAX_IMAGES_PER_ANSWER),
        );
    }

    #[test]
    fn enforces_per_answer_total_bytes() {
        let sources = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let byte_paths = (0..5)
            .map(|index| {
                image_file(
                    &sources,
                    &format!("bytes-{index}.png"),
                    MAX_AGENT_FILE_BYTES,
                )
            })
            .chain(std::iter::once(image_file(&sources, "over-budget.png", 8)))
            .collect::<Vec<_>>();
        let byte_answer = byte_paths
            .iter()
            .map(|path| format!("![](<{}>)", path.display()))
            .collect::<Vec<_>>()
            .join(" ");
        let mut byte_messages = conversation(byte_answer.clone());
        attach_with_storage_root("bytes-session", &mut byte_messages, storage.path());
        let expected_byte_sources = byte_paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let path = path.display().to_string();
                let source = (index < 5).then(|| {
                    let key = image_key("bytes-session", &byte_answer, &path);
                    format!("/api/sessions/bytes-session/answer-images/{key}")
                });
                (path, source)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let actual_byte_sources = &byte_messages.message_image_sources[&0];
        let byte_session_storage =
            file::session_directory(storage.path(), "bytes-session").unwrap();
        assert_eq!(
            (
                actual_byte_sources,
                byte_session_storage.read_dir().unwrap().count(),
            ),
            (&expected_byte_sources, 5),
        );
    }
}
