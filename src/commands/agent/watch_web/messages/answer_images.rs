use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::commands::agent::file::{self, storage_root};
use crate::commands::agent::file_attachment_support::{
    MAX_AGENT_FILE_BYTES, inspect_bounded_image, read_bounded_image,
};
use crate::commands::agent::watch_web::image_attachments::MAX_IMAGES_PER_MESSAGE;

const MAX_MESSAGE_IMAGE_BYTES: u64 = 50 * 1024 * 1024;

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
            tracing::warn!(%error, "failed to locate conversation image storage");
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
        let mut seen_paths = HashSet::new();
        let mut paths = Vec::new();
        for text in markdown_image_texts(message) {
            for path in markdown_image_paths(text) {
                if seen_paths.insert(path.clone()) {
                    paths.push((text, path));
                }
            }
        }
        if paths.is_empty() {
            continue;
        }
        let mut sources = BTreeMap::new();
        let mut total_size = 0_u64;
        for (path_index, (text, path)) in paths.iter().enumerate() {
            if path_index >= MAX_IMAGES_PER_MESSAGE || has_markdown_destination_escape(path) {
                sources.insert(path.clone(), None);
                continue;
            }
            let Some(key) = image_key(session_id, message, path) else {
                sources.insert(path.clone(), None);
                continue;
            };
            let legacy_key = legacy_image_key(session_id, text, path);
            let copied = match snapshot_image(
                storage_root,
                session_id,
                &key,
                &legacy_key,
                Path::new(path),
                MAX_MESSAGE_IMAGE_BYTES - total_size,
            ) {
                Ok(copied) => copied,
                Err(error) => {
                    tracing::warn!(%error, path, "failed to snapshot conversation message image");
                    None
                }
            };
            let source = copied.and_then(|(size, _mime_type)| {
                let next_total = total_size.checked_add(size as u64)?;
                if next_total > MAX_MESSAGE_IMAGE_BYTES {
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
        let mut seen_paths = HashSet::new();
        let sources: BTreeMap<String, Option<String>> = markdown_image_texts(message)
            .into_iter()
            .flat_map(markdown_image_paths)
            .filter(|path| seen_paths.insert(path.clone()))
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

fn markdown_image_texts(message: &super::ChatItem) -> Vec<&str> {
    match message {
        super::ChatItem::Assistant { text } => vec![text],
        super::ChatItem::Todo {
            detail, comment, ..
        } => detail
            .iter()
            .chain(comment.iter())
            .map(String::as_str)
            .collect(),
        super::ChatItem::Proposal {
            summary,
            options,
            answer,
            comment,
            ..
        } => summary
            .iter()
            .chain(
                options
                    .iter()
                    .filter_map(|option| option.description.as_ref()),
            )
            .chain(answer.iter().filter_map(|answer| answer.comment.as_ref()))
            .chain(comment.iter())
            .map(String::as_str)
            .collect(),
        _ => Vec::new(),
    }
}

fn image_key(session_id: &str, message: &super::ChatItem, path: &str) -> Option<String> {
    match message {
        super::ChatItem::Assistant { text } => Some(legacy_image_key(session_id, text, path)),
        super::ChatItem::Todo { seq, .. } => Some(card_image_key(session_id, b"todo", *seq, path)),
        super::ChatItem::Proposal { seq, .. } => {
            Some(card_image_key(session_id, b"proposal", *seq, path))
        }
        _ => None,
    }
}

fn card_image_key(session_id: &str, card_type: &[u8], seq: i64, path: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(session_id.as_bytes());
    digest.update([0]);
    digest.update(card_type);
    digest.update(seq.to_be_bytes());
    digest.update([0]);
    digest.update(path.as_bytes());
    crate::shared::hex::encode(&digest.finalize())
}

fn legacy_image_key(session_id: &str, text: &str, path: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(session_id.as_bytes());
    digest.update([0]);
    digest.update(text.as_bytes());
    digest.update([0]);
    digest.update(path.as_bytes());
    crate::shared::hex::encode(&digest.finalize())
}

fn snapshot_image(
    storage_root: &Path,
    session_id: &str,
    key: &str,
    legacy_key: &str,
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
        Ok(None) => {
            return restore_legacy_snapshot(&directory, key, legacy_key, remaining_bytes);
        }
        Err(error) => {
            tracing::warn!(%error, "failed to read message image source");
            return restore_legacy_snapshot(&directory, key, legacy_key, remaining_bytes);
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
        Err(error) => Err(error).with_context(|| "saving message image snapshot"),
    }
}

fn restore_legacy_snapshot(
    directory: &Path,
    key: &str,
    legacy_key: &str,
    remaining_bytes: u64,
) -> Result<Option<(usize, &'static str)>> {
    if key == legacy_key {
        return Ok(None);
    }
    let legacy_path = stored_path(directory, legacy_key);
    let Some(image) = read_bounded_image(&legacy_path, MAX_AGENT_FILE_BYTES)? else {
        return Ok(None);
    };
    if image.bytes.len() as u64 > remaining_bytes {
        return Ok(None);
    }
    let new_path = stored_path(directory, key);
    match file::write_new_file(&new_path, &image.bytes) {
        Ok(()) => Ok(Some((image.bytes.len(), image.mime_type))),
        Err(error) if is_already_exists(&error) => {
            Ok(inspect_bounded_image(&new_path, MAX_AGENT_FILE_BYTES)?
                .map(|saved| (saved.size as usize, saved.mime_type)))
        }
        Err(error) => Err(error).with_context(|| "migrating legacy message image snapshot"),
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
                && !value.starts_with("/api/images/")
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

    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use crate::commands::agent::proposal::{ProposalAnswer, ProposalOption};

    use super::{
        MAX_AGENT_FILE_BYTES, MAX_IMAGES_PER_MESSAGE, attach_with_storage_root,
        attach_with_storage_root_result, image_key, legacy_image_key, markdown_image_paths,
    };
    use crate::commands::agent::file::{self, remove_session_storage};
    use crate::commands::agent::watch_web::messages::{ChatItem, ConversationResponse};

    const PNG_HEADER: &[u8] = &[137, 80, 78, 71, 13, 10, 26, 10];

    #[fixture]
    fn image_directories() -> (TempDir, TempDir) {
        (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
    }

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

    fn todo(seq: i64, detail: Option<String>, comment: Option<String>) -> ChatItem {
        ChatItem::Todo {
            seq,
            title: "A task".to_owned(),
            detail,
            folded: false,
            added_folded: false,
            completed: false,
            resolution: None,
            session_ended_at: None,
            comment,
        }
    }

    fn proposal(
        seq: i64,
        summary: Option<String>,
        answer_comment: Option<String>,
        comment: Option<String>,
    ) -> ChatItem {
        ChatItem::Proposal {
            seq,
            title: "A choice".to_owned(),
            summary,
            options: Vec::new(),
            answer: answer_comment.map(|comment| ProposalAnswer {
                choice: Some(0),
                comment: Some(comment),
            }),
            resolved: true,
            resolution: None,
            session_ended_at: None,
            comment,
        }
    }

    fn normalized_sources(
        conversation: &ConversationResponse,
    ) -> BTreeMap<usize, BTreeMap<String, Option<&'static str>>> {
        conversation
            .message_image_sources
            .iter()
            .map(|(index, sources)| {
                (
                    *index,
                    sources
                        .iter()
                        .map(|(path, source)| (path.clone(), source.as_ref().map(|_| "served")))
                        .collect::<BTreeMap<_, _>>(),
                )
            })
            .collect::<BTreeMap<_, _>>()
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

    #[rstest]
    fn keeps_the_first_snapshot_for_the_same_answer_and_path(
        image_directories: (TempDir, TempDir),
    ) {
        let (sources, storage) = image_directories;
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

    #[rstest]
    fn keys_card_snapshots_by_card_identity_and_path(image_directories: (TempDir, TempDir)) {
        let (sources, storage) = image_directories;
        let path = image_file(&sources, "shared.png", 8);
        let path_text = path.display().to_string();
        let original = fs::read(&path).unwrap();
        let repeated_text = format!("Shared detail ![](<{path_text}>)");
        let first_card = todo(1, Some(repeated_text.clone()), None);
        let mut first_conversation = ConversationResponse {
            messages: vec![first_card.clone()],
            ..ConversationResponse::default()
        };

        attach_with_storage_root("fixture-session", &mut first_conversation, storage.path());

        let updated = [PNG_HEADER, b"updated"].concat();
        fs::write(&path, &updated).unwrap();
        let second_card = todo(2, Some(repeated_text), None);
        let mut second_conversation = ConversationResponse {
            messages: vec![second_card.clone()],
            ..ConversationResponse::default()
        };

        attach_with_storage_root("fixture-session", &mut second_conversation, storage.path());

        fs::remove_file(&path).unwrap();
        let edited_card = todo(2, Some(format!("Edited detail ![](<{path_text}>)")), None);
        let mut edited_conversation = ConversationResponse {
            messages: vec![edited_card],
            ..ConversationResponse::default()
        };

        attach_with_storage_root("fixture-session", &mut edited_conversation, storage.path());

        let source_for = |conversation: &ConversationResponse| {
            conversation.message_image_sources[&0][&path_text].clone()
        };
        let first_source = source_for(&first_conversation);
        let second_source = source_for(&second_conversation);
        let edited_source = source_for(&edited_conversation);
        let snapshot_for = |source: Option<&String>| {
            source
                .and_then(|source| source.rsplit('/').next())
                .and_then(|key| {
                    super::read_snapshot(storage.path(), "fixture-session", key).unwrap()
                })
        };
        let expected_sources = BTreeMap::from([(0, BTreeMap::from([(path_text, Some("served"))]))]);

        assert_eq!(
            (
                normalized_sources(&first_conversation),
                normalized_sources(&second_conversation),
                normalized_sources(&edited_conversation),
                first_source.is_some(),
                second_source.is_some(),
                edited_source.is_some(),
                first_source != second_source,
                second_source == edited_source,
                (
                    snapshot_for(first_source.as_ref()),
                    snapshot_for(second_source.as_ref())
                ),
            ),
            (
                expected_sources.clone(),
                expected_sources.clone(),
                expected_sources,
                true,
                true,
                true,
                true,
                true,
                (Some((original, "image/png")), Some((updated, "image/png"))),
            ),
        );
    }

    #[rstest]
    fn keys_proposal_snapshots_by_sequence_and_path(image_directories: (TempDir, TempDir)) {
        let (sources, storage) = image_directories;
        let path = image_file(&sources, "proposal-shared.png", 8);
        let path_text = path.display().to_string();
        let original = fs::read(&path).unwrap();
        let repeated_summary = format!("Shared summary ![](<{path_text}>)");
        let first_card = proposal(1, Some(repeated_summary.clone()), None, None);
        let mut first_conversation = ConversationResponse {
            messages: vec![first_card.clone()],
            ..ConversationResponse::default()
        };

        attach_with_storage_root("fixture-session", &mut first_conversation, storage.path());

        let updated = [PNG_HEADER, b"updated proposal"].concat();
        fs::write(&path, &updated).unwrap();
        let second_card = proposal(2, Some(repeated_summary), None, None);
        let mut second_conversation = ConversationResponse {
            messages: vec![second_card.clone()],
            ..ConversationResponse::default()
        };

        attach_with_storage_root("fixture-session", &mut second_conversation, storage.path());

        let first_source = first_conversation.message_image_sources[&0][&path_text]
            .as_ref()
            .cloned();
        let second_source = second_conversation.message_image_sources[&0][&path_text]
            .as_ref()
            .cloned();
        let snapshot_for = |source: Option<&String>| {
            source
                .and_then(|source| source.rsplit('/').next())
                .and_then(|key| {
                    super::read_snapshot(storage.path(), "fixture-session", key).unwrap()
                })
        };
        let expected_sources = BTreeMap::from([(0, BTreeMap::from([(path_text, Some("served"))]))]);

        assert_eq!(
            (
                normalized_sources(&first_conversation),
                normalized_sources(&second_conversation),
                first_source.is_some(),
                second_source.is_some(),
                first_source != second_source,
                (
                    snapshot_for(first_source.as_ref()),
                    snapshot_for(second_source.as_ref())
                ),
            ),
            (
                expected_sources.clone(),
                expected_sources,
                true,
                true,
                true,
                (Some((original, "image/png")), Some((updated, "image/png"))),
            ),
        );
    }

    #[rstest]
    fn restores_a_legacy_card_snapshot_when_the_source_is_missing(
        image_directories: (TempDir, TempDir),
    ) {
        let (sources, storage) = image_directories;
        let path = image_file(&sources, "legacy.png", 8);
        let path_text = path.display().to_string();
        let original = fs::read(&path).unwrap();
        let detail = format!("Legacy detail ![](<{path_text}>)");
        let card = todo(3, Some(detail.clone()), None);
        let legacy_key = legacy_image_key("fixture-session", &detail, &path_text);
        let new_key = image_key("fixture-session", &card, &path_text).unwrap();
        let session_directory = file::session_directory(storage.path(), "fixture-session").unwrap();
        let legacy_snapshot = session_directory.join(format!("answer-image-{legacy_key}"));
        fs::write(&legacy_snapshot, &original).unwrap();
        fs::remove_file(&path).unwrap();
        let mut conversation = ConversationResponse {
            messages: vec![card],
            ..ConversationResponse::default()
        };

        attach_with_storage_root("fixture-session", &mut conversation, storage.path());

        assert_eq!(
            (
                conversation.message_image_sources,
                super::read_snapshot(storage.path(), "fixture-session", &new_key).unwrap(),
                fs::read(&legacy_snapshot).unwrap(),
            ),
            (
                BTreeMap::from([(
                    0,
                    BTreeMap::from([(
                        path_text,
                        Some(format!(
                            "/api/sessions/fixture-session/answer-images/{new_key}"
                        )),
                    )]),
                )]),
                Some((original.clone(), "image/png")),
                original,
            ),
        );
    }

    #[rstest]
    fn leaves_uploaded_card_image_previews_for_the_browser(image_directories: (TempDir, TempDir)) {
        let (_sources, storage) = image_directories;
        let todo_preview = "/api/images/fixture-todo-image.png";
        let proposal_preview = "/api/images/fixture-proposal-image.png";
        let mut conversation = ConversationResponse {
            messages: vec![
                todo(
                    1,
                    None,
                    Some(format!("Resolved with ![](<{todo_preview}>)")),
                ),
                proposal(
                    2,
                    None,
                    Some(format!("Answered with ![](<{proposal_preview}>)")),
                    None,
                ),
            ],
            ..ConversationResponse::default()
        };

        attach_with_storage_root("fixture-session", &mut conversation, storage.path());

        assert_eq!(conversation.message_image_sources, BTreeMap::new());
    }

    #[test]
    fn snapshots_images_with_spaces_and_non_ascii_path_characters() {
        let sources = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let path = image_file(&sources, "画像 samples final image.png", 8);
        let answer = format!("![](<{}>)", path.display());
        let mut messages = conversation(answer.clone());

        attach_with_storage_root("fixture-session", &mut messages, storage.path());

        let key = image_key(
            "fixture-session",
            &assistant(answer.clone()),
            &path.display().to_string(),
        )
        .unwrap();
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

    #[rstest]
    fn snapshots_images_from_todo_and_proposal_card_text(image_directories: (TempDir, TempDir)) {
        let (sources, storage) = image_directories;
        let texts = [
            "todo detail",
            "todo comment",
            "proposal summary",
            "proposal option",
            "proposal answer",
            "proposal comment",
        ];
        let paths = texts
            .iter()
            .enumerate()
            .map(|(index, _)| image_file(&sources, &format!("image-{index}.png"), 8))
            .collect::<Vec<_>>();
        let markdown =
            |text: &str, path: &std::path::Path| format!("{text} ![](<{}>)", path.display());
        let todo_detail = markdown(texts[0], &paths[0]);
        let todo_comment = markdown(texts[1], &paths[1]);
        let proposal_summary = markdown(texts[2], &paths[2]);
        let proposal_option = markdown(texts[3], &paths[3]);
        let proposal_answer = markdown(texts[4], &paths[4]);
        let proposal_comment = markdown(texts[5], &paths[5]);
        let todo_message = ChatItem::Todo {
            seq: 1,
            title: "A task".to_owned(),
            detail: Some(todo_detail.clone()),
            folded: false,
            added_folded: false,
            completed: false,
            resolution: None,
            session_ended_at: None,
            comment: Some(todo_comment.clone()),
        };
        let proposal_message = ChatItem::Proposal {
            seq: 2,
            title: "A choice".to_owned(),
            summary: Some(proposal_summary.clone()),
            options: vec![ProposalOption {
                label: "Option".to_owned(),
                description: Some(proposal_option.clone()),
                pros: Vec::new(),
                cons: Vec::new(),
                recommended: false,
            }],
            answer: Some(ProposalAnswer {
                choice: Some(0),
                comment: Some(proposal_answer.clone()),
            }),
            resolved: false,
            resolution: None,
            session_ended_at: None,
            comment: Some(proposal_comment.clone()),
        };
        let mut conversation = ConversationResponse {
            messages: vec![todo_message.clone(), proposal_message.clone()],
            start: 7,
            ..ConversationResponse::default()
        };

        attach_with_storage_root("fixture-session", &mut conversation, storage.path());

        let expected_sources = [
            (7, &todo_message, &paths[0]),
            (7, &todo_message, &paths[1]),
            (8, &proposal_message, &paths[2]),
            (8, &proposal_message, &paths[3]),
            (8, &proposal_message, &paths[4]),
            (8, &proposal_message, &paths[5]),
        ]
        .into_iter()
        .fold(
            BTreeMap::new(),
            |mut sources, (message_index, message, path)| {
                sources
                    .entry(message_index)
                    .or_insert_with(BTreeMap::new)
                    .insert(
                        path.display().to_string(),
                        Some(format!(
                            "/api/sessions/fixture-session/answer-images/{}",
                            image_key("fixture-session", message, &path.display().to_string())
                                .unwrap()
                        )),
                    );
                sources
            },
        );

        assert_eq!(conversation.message_image_sources, expected_sources);
    }

    #[rstest]
    fn marks_todo_and_proposal_images_unavailable_when_storage_is_unavailable(
        image_directories: (TempDir, TempDir),
    ) {
        let (sources, storage) = image_directories;
        let invalid_storage_root = storage.path().join("not-a-directory");
        fs::write(&invalid_storage_root, b"fixture").unwrap();
        let todo_path = image_file(&sources, "todo.png", 8);
        let proposal_path = image_file(&sources, "proposal.png", 8);
        let todo_detail = format!("![](<{}>)", todo_path.display());
        let proposal_summary = format!("![](<{}>)", proposal_path.display());
        let mut conversation = ConversationResponse {
            messages: vec![
                ChatItem::Todo {
                    seq: 1,
                    title: "A task".to_owned(),
                    detail: Some(todo_detail),
                    folded: false,
                    added_folded: false,
                    completed: false,
                    resolution: None,
                    session_ended_at: None,
                    comment: None,
                },
                ChatItem::Proposal {
                    seq: 2,
                    title: "A choice".to_owned(),
                    summary: Some(proposal_summary),
                    options: Vec::new(),
                    answer: None,
                    resolved: false,
                    resolution: None,
                    session_ended_at: None,
                    comment: None,
                },
            ],
            ..ConversationResponse::default()
        };

        attach_with_storage_root("fixture-session", &mut conversation, &invalid_storage_root);

        assert_eq!(
            conversation.message_image_sources,
            BTreeMap::from([
                (0, BTreeMap::from([(todo_path.display().to_string(), None)]),),
                (
                    1,
                    BTreeMap::from([(proposal_path.display().to_string(), None)]),
                ),
            ]),
        );
    }

    #[rstest]
    fn enforces_the_image_count_limit_across_proposal_fields(
        image_directories: (TempDir, TempDir),
    ) {
        let (sources, storage) = image_directories;
        let paths = (0..=MAX_IMAGES_PER_MESSAGE)
            .map(|index| image_file(&sources, &format!("proposal-{index}.png"), 8))
            .collect::<Vec<_>>();
        let markdown = |paths: &[std::path::PathBuf]| {
            paths
                .iter()
                .map(|path| format!("![](<{}>)", path.display()))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let summary = markdown(&paths[..8]);
        let option_description = markdown(&paths[8..]);
        let proposal = ChatItem::Proposal {
            seq: 1,
            title: "A choice".to_owned(),
            summary: Some(summary.clone()),
            options: vec![ProposalOption {
                label: "Option".to_owned(),
                description: Some(option_description.clone()),
                pros: Vec::new(),
                cons: Vec::new(),
                recommended: false,
            }],
            answer: None,
            resolved: false,
            resolution: None,
            session_ended_at: None,
            comment: None,
        };
        let mut conversation = ConversationResponse {
            messages: vec![proposal.clone()],
            ..ConversationResponse::default()
        };

        attach_with_storage_root("fixture-session", &mut conversation, storage.path());

        let expected_sources = paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                let path = path.display().to_string();
                let source = (index < MAX_IMAGES_PER_MESSAGE).then(|| {
                    let key = image_key("fixture-session", &proposal, &path).unwrap();
                    format!("/api/sessions/fixture-session/answer-images/{key}")
                });
                (path, source)
            })
            .collect::<BTreeMap<_, _>>();
        let stored_images = file::session_directory(storage.path(), "fixture-session")
            .unwrap()
            .read_dir()
            .unwrap()
            .count();

        assert_eq!(
            (conversation.message_image_sources, stored_images),
            (
                BTreeMap::from([(0, expected_sources)]),
                MAX_IMAGES_PER_MESSAGE,
            ),
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

        let key = image_key(
            "fixture-session",
            &assistant(answer.clone()),
            &path.display().to_string(),
        )
        .unwrap();
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
        let count_paths = (0..=MAX_IMAGES_PER_MESSAGE)
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
                let source = (index < MAX_IMAGES_PER_MESSAGE).then(|| {
                    let key = image_key("count-session", &assistant(count_answer.clone()), &path)
                        .unwrap();
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
            (&expected_count_sources, MAX_IMAGES_PER_MESSAGE),
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
                    let key =
                        image_key("bytes-session", &assistant(byte_answer.clone()), &path).unwrap();
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
