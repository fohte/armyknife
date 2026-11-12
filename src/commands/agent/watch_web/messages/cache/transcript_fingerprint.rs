use std::fs::{File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::commands::agent::types::Engine;

const TRANSCRIPT_TAIL_BYTES: u64 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::commands::agent::watch_web::messages) enum TranscriptChange {
    Unchanged,
    Appended,
    Rewritten,
}

#[derive(Clone, PartialEq, Eq)]
pub(in crate::commands::agent::watch_web::messages) struct TranscriptFingerprint {
    source_path: PathBuf,
    len: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed_at_seconds: i64,
    #[cfg(unix)]
    changed_at_nanoseconds: i64,
}

impl TranscriptFingerprint {
    pub(super) fn change_from(&self, previous: &Self) -> TranscriptChange {
        if self == previous {
            return TranscriptChange::Unchanged;
        }
        if self.source_path == previous.source_path
            && self.len > previous.len
            && self.has_same_file_identity(previous)
        {
            return TranscriptChange::Appended;
        }
        TranscriptChange::Rewritten
    }

    fn has_same_file_identity(&self, previous: &Self) -> bool {
        #[cfg(unix)]
        {
            self.device == previous.device && self.inode == previous.inode
        }
        #[cfg(not(unix))]
        {
            self.modified >= previous.modified
        }
    }

    pub(in crate::commands::agent::watch_web::messages) fn is_compressed(&self) -> bool {
        self.source_path
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("zst")
    }

    pub(in crate::commands::agent::watch_web::messages) fn len(&self) -> u64 {
        self.len
    }

    pub(super) fn is_present(&self) -> anyhow::Result<bool> {
        Ok(read_metadata(&self.source_path)?.is_some())
    }

    pub(in crate::commands::agent::watch_web::messages) fn tail_bytes(
        &self,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let mut file = match File::open(&self.source_path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("open transcript {}", self.source_path.display()));
            }
        };
        if !file_matches_fingerprint(&file, self)? {
            return Ok(None);
        }
        let tail_start = self.len.saturating_sub(TRANSCRIPT_TAIL_BYTES);
        let tail_len = self.len - tail_start;
        file.seek(SeekFrom::Start(tail_start))
            .with_context(|| format!("seek transcript tail {}", self.source_path.display()))?;
        let mut tail = Vec::with_capacity(tail_len as usize);
        (&mut file)
            .take(tail_len)
            .read_to_end(&mut tail)
            .with_context(|| format!("read transcript tail {}", self.source_path.display()))?;
        if tail.len() as u64 != tail_len || !file_matches_fingerprint(&file, self)? {
            return Ok(None);
        }
        Ok(Some(tail))
    }

    pub(in crate::commands::agent::watch_web::messages) fn read_delta_from(
        &self,
        previous: &Self,
        previous_tail: &[u8],
    ) -> anyhow::Result<(TranscriptChange, Vec<u8>)> {
        let change = self.change_from(previous);
        match change {
            TranscriptChange::Unchanged if self.is_present()? => Ok((change, Vec::new())),
            TranscriptChange::Unchanged => Ok((TranscriptChange::Rewritten, Vec::new())),
            TranscriptChange::Rewritten => Ok((change, Vec::new())),
            TranscriptChange::Appended => {
                if self.is_compressed() {
                    return Ok((TranscriptChange::Rewritten, Vec::new()));
                }
                let mut file = match File::open(&self.source_path) {
                    Ok(file) => file,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Ok((TranscriptChange::Rewritten, Vec::new()));
                    }
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("open transcript {}", self.source_path.display())
                        });
                    }
                };
                if !file_matches_fingerprint(&file, self)?
                    || !matches_tail(&mut file, &self.source_path, previous.len, previous_tail)?
                {
                    return Ok((TranscriptChange::Rewritten, Vec::new()));
                }
                file.seek(SeekFrom::Start(previous.len))
                    .with_context(|| format!("seek transcript {}", self.source_path.display()))?;
                let mut appended = Vec::new();
                (&mut file)
                    .take(self.len - previous.len)
                    .read_to_end(&mut appended)
                    .with_context(|| format!("read transcript {}", self.source_path.display()))?;
                if appended.len() as u64 != self.len - previous.len {
                    return Ok((TranscriptChange::Rewritten, Vec::new()));
                }
                if !file_matches_fingerprint(&file, self)?
                    || !matches_tail(&mut file, &self.source_path, previous.len, previous_tail)?
                {
                    return Ok((TranscriptChange::Rewritten, Vec::new()));
                }
                Ok((change, appended))
            }
        }
    }
}

pub(super) fn transcript_fingerprint(
    engine: Engine,
    path: Option<&Path>,
) -> anyhow::Result<Option<TranscriptFingerprint>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let (source_path, metadata) = if engine == Engine::Codex {
        if let Some(metadata) = read_metadata(path)? {
            (path.to_path_buf(), metadata)
        } else {
            let compressed_path = super::super::compressed_path(path);
            let Some(metadata) = read_metadata(&compressed_path)? else {
                return Ok(None);
            };
            (compressed_path, metadata)
        }
    } else {
        let Some(metadata) = read_metadata(path)? else {
            return Ok(None);
        };
        (path.to_path_buf(), metadata)
    };
    let modified = metadata.modified().with_context(|| {
        format!(
            "read transcript modification time {}",
            source_path.display()
        )
    })?;
    #[cfg(unix)]
    let (device, inode, changed_at_seconds, changed_at_nanoseconds) = {
        use std::os::unix::fs::MetadataExt;

        (
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        )
    };
    Ok(Some(TranscriptFingerprint {
        source_path,
        len: metadata.len(),
        modified,
        #[cfg(unix)]
        device,
        #[cfg(unix)]
        inode,
        #[cfg(unix)]
        changed_at_seconds,
        #[cfg(unix)]
        changed_at_nanoseconds,
    }))
}

fn read_metadata(path: &Path) -> anyhow::Result<Option<Metadata>> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("read transcript metadata {}", path.display()))
        }
    }
}

fn file_matches_fingerprint(
    file: &File,
    fingerprint: &TranscriptFingerprint,
) -> anyhow::Result<bool> {
    let metadata = file.metadata().with_context(|| {
        format!(
            "read transcript metadata {}",
            fingerprint.source_path.display()
        )
    })?;
    if metadata.len() < fingerprint.len {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        if metadata.dev() != fingerprint.device || metadata.ino() != fingerprint.inode {
            return Ok(false);
        }
    }
    #[cfg(not(unix))]
    {
        let modified = metadata.modified().with_context(|| {
            format!(
                "read transcript modification time {}",
                fingerprint.source_path.display()
            )
        })?;
        if modified < fingerprint.modified {
            return Ok(false);
        }
    }
    Ok(true)
}

// shortcut: changes before the saved tail can go undetected, widen this check if transcript writers ever modify existing bytes.
fn matches_tail(
    file: &mut File,
    path: &Path,
    previous_len: u64,
    previous_tail: &[u8],
) -> anyhow::Result<bool> {
    let tail_len = previous_tail.len() as u64;
    if tail_len > previous_len {
        return Ok(false);
    }
    if previous_tail.is_empty() {
        return Ok(previous_len == 0);
    }
    file.seek(SeekFrom::Start(previous_len - tail_len))
        .with_context(|| format!("seek transcript tail {}", path.display()))?;
    let mut current_tail = vec![0; previous_tail.len()];
    match file.read_exact(&mut current_tail) {
        Ok(()) => Ok(current_tail == previous_tail),
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("read transcript tail {}", path.display()))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Seek, SeekFrom, Write};

    use indoc::indoc;
    use rstest::{fixture, rstest};

    use super::{TranscriptChange, transcript_fingerprint};
    use crate::commands::agent::types::Engine;

    #[fixture]
    fn transcript_file() -> (tempfile::TempDir, std::path::PathBuf) {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("transcript.jsonl");
        (directory, path)
    }

    #[rstest]
    fn transcript_append_reads_only_bytes_after_the_cached_length(
        transcript_file: (tempfile::TempDir, std::path::PathBuf),
    ) {
        let (_directory, path) = transcript_file;
        let previous_tail = b"first record\n";
        std::fs::write(&path, previous_tail).expect("transcript should be written");
        let previous = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("transcript should open for appending")
            .write_all(b"second record\n")
            .expect("record should be appended");
        let current = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");

        assert_eq!(
            current
                .read_delta_from(&previous, previous_tail)
                .expect("delta should be read"),
            (TranscriptChange::Appended, b"second record\n".to_vec()),
        );
    }

    #[rstest]
    fn transcript_tail_rewrite_requires_a_full_read(
        transcript_file: (tempfile::TempDir, std::path::PathBuf),
    ) {
        let (_directory, path) = transcript_file;
        let previous_tail = b"second record\n";
        std::fs::write(
            &path,
            indoc! {"
                first record
                second record
            "},
        )
        .expect("transcript should be written");
        let previous = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("transcript should open for rewriting");
        file.seek(SeekFrom::Start(b"first record\n".len() as u64))
            .expect("rewrite position should be reached");
        file.write_all(b"second rec0rd\n")
            .expect("tail should be rewritten");
        file.seek(SeekFrom::End(0))
            .expect("transcript end should be reached");
        file.write_all(b"third record\n")
            .expect("new record should be appended");
        drop(file);
        let current = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");

        assert_eq!(
            (
                current.change_from(&previous),
                current
                    .read_delta_from(&previous, previous_tail)
                    .expect("delta should be read"),
            ),
            (
                TranscriptChange::Appended,
                (TranscriptChange::Rewritten, Vec::new()),
            ),
        );
    }

    #[rstest]
    fn transcript_append_after_fingerprint_capture_keeps_the_original_delta(
        transcript_file: (tempfile::TempDir, std::path::PathBuf),
    ) {
        let (_directory, path) = transcript_file;
        let previous_tail = b"first record\n";
        std::fs::write(&path, previous_tail).expect("transcript should be written");
        let previous = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("transcript should open for appending")
            .write_all(b"middle record\n")
            .expect("first record should be appended");
        let current = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("transcript should open for appending")
            .write_all(b"later record\n")
            .expect("additional record should be appended");

        assert_eq!(
            current
                .read_delta_from(&previous, previous_tail)
                .expect("captured delta should be read"),
            (TranscriptChange::Appended, b"middle record\n".to_vec()),
        );
    }

    #[rstest]
    fn transcript_tail_bytes_stop_at_the_fingerprinted_length(
        transcript_file: (tempfile::TempDir, std::path::PathBuf),
    ) {
        let (_directory, path) = transcript_file;
        let initial = vec![b'a'; 80];
        let expected_tail = initial[16..].to_vec();
        std::fs::write(&path, &initial).expect("transcript should be written");
        let fingerprint = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("transcript should open for appending")
            .write_all(b"later bytes are outside the snapshot")
            .expect("later bytes should be appended");

        assert_eq!(
            fingerprint
                .tail_bytes()
                .expect("captured tail should be read"),
            Some(expected_tail),
        );
    }

    #[rstest]
    fn transcript_tail_bytes_are_unavailable_after_the_file_disappears(
        transcript_file: (tempfile::TempDir, std::path::PathBuf),
    ) {
        let (_directory, path) = transcript_file;
        std::fs::write(&path, b"first record\n").expect("transcript should be written");
        let fingerprint = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");
        std::fs::remove_file(&path).expect("transcript should be removed");

        assert_eq!(
            fingerprint
                .tail_bytes()
                .expect("missing transcript should not be an error"),
            None,
        );
    }

    #[rstest]
    #[case::file_is_replaced(true)]
    #[case::file_is_truncated(false)]
    fn transcript_tail_read_treats_a_changed_file_as_a_rewrite(
        transcript_file: (tempfile::TempDir, std::path::PathBuf),
        #[case] replace_file: bool,
    ) {
        let (_directory, path) = transcript_file;
        std::fs::write(&path, b"first record\n").expect("transcript should be written");
        let fingerprint = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");
        if replace_file {
            let replacement = path.with_extension("replacement");
            std::fs::write(&replacement, b"replacement record\n")
                .expect("replacement should be written");
            std::fs::rename(&replacement, &path).expect("transcript should be replaced");
        } else {
            std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("transcript should open for truncation")
                .set_len(0)
                .expect("transcript should be truncated");
        }

        assert_eq!(
            fingerprint
                .tail_bytes()
                .expect("rewrite should not be a tail read error"),
            None,
        );
    }

    #[rstest]
    fn empty_saved_tail_does_not_match_a_nonempty_transcript_prefix(
        transcript_file: (tempfile::TempDir, std::path::PathBuf),
    ) {
        let (_directory, path) = transcript_file;
        std::fs::write(&path, b"first record\n").expect("transcript should be written");
        let previous = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("transcript should open for appending")
            .write_all(b"second record\n")
            .expect("record should be appended");
        let current = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");

        assert_eq!(
            current
                .read_delta_from(&previous, &[])
                .expect("saved tail mismatch should be detected"),
            (TranscriptChange::Rewritten, Vec::new()),
        );
    }

    #[rstest]
    fn same_length_transcript_rewrite_requires_a_full_read(
        transcript_file: (tempfile::TempDir, std::path::PathBuf),
    ) {
        let (_directory, path) = transcript_file;
        let previous_tail = b"first record\n";
        std::fs::write(&path, previous_tail).expect("transcript should be written");
        let previous = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");
        std::fs::write(&path, b"other record\n").expect("transcript should be rewritten");
        let current = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");

        assert_eq!(
            current
                .read_delta_from(&previous, previous_tail)
                .expect("rewrite should be detected"),
            (TranscriptChange::Rewritten, Vec::new()),
        );
    }
}
