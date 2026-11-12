use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::commands::agent::types::Engine;

const TRANSCRIPT_PROBE_COUNT: u64 = 16;
const TRANSCRIPT_PROBE_BYTES: u64 = 64;
const TRANSCRIPT_FULL_PREFIX_LIMIT: u64 = 2048;

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
    prefix_probes: Vec<TranscriptProbe>,
    // ctime catches same-size rewrites even if the modification time is restored.
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    changed_at_seconds: i64,
    #[cfg(unix)]
    changed_at_nanoseconds: i64,
}

#[derive(Clone, PartialEq, Eq)]
struct TranscriptProbe {
    offset: u64,
    bytes: Vec<u8>,
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

    fn matches_file_metadata(&self, file: &File) -> anyhow::Result<bool> {
        let metadata = file
            .metadata()
            .with_context(|| format!("read transcript metadata {}", self.source_path.display()))?;
        let modified = metadata.modified().with_context(|| {
            format!(
                "read transcript modification time {}",
                self.source_path.display()
            )
        })?;
        if metadata.len() != self.len || modified != self.modified {
            return Ok(false);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;

            if metadata.dev() != self.device
                || metadata.ino() != self.inode
                || metadata.ctime() != self.changed_at_seconds
                || metadata.ctime_nsec() != self.changed_at_nanoseconds
            {
                return Ok(false);
            }
        }
        Ok(true)
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

    pub(in crate::commands::agent::watch_web::messages) fn read_delta_from(
        &self,
        previous: &Self,
    ) -> anyhow::Result<(TranscriptChange, Vec<u8>)> {
        let change = self.change_from(previous);
        match change {
            TranscriptChange::Unchanged => Ok((change, Vec::new())),
            TranscriptChange::Rewritten => Ok((change, Vec::new())),
            TranscriptChange::Appended => {
                if self.is_compressed() {
                    return Ok((TranscriptChange::Rewritten, Vec::new()));
                }
                let mut file = File::open(&self.source_path)
                    .with_context(|| format!("open transcript {}", self.source_path.display()))?;
                // Metadata cannot locate writes inside the old prefix, so check bounded samples.
                if !self.matches_file_metadata(&file)?
                    || !prefix_matches(&mut file, &self.source_path, &previous.prefix_probes)?
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
                if !self.matches_file_metadata(&file)?
                    || !prefix_matches(&mut file, &self.source_path, &self.prefix_probes)?
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
    let source_path = if engine == Engine::Codex {
        match std::fs::metadata(path) {
            Ok(_) => path.to_path_buf(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let compressed_path = super::super::compressed_path(path);
                match std::fs::metadata(&compressed_path) {
                    Ok(_) => compressed_path,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(None);
                    }
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("read transcript metadata {}", compressed_path.display())
                        });
                    }
                }
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read transcript metadata {}", path.display()));
            }
        }
    } else {
        path.to_path_buf()
    };
    let mut file = match File::open(&source_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("open transcript {}", source_path.display()));
        }
    };
    let metadata = file
        .metadata()
        .with_context(|| format!("read transcript metadata {}", source_path.display()))?;
    let modified = metadata.modified().with_context(|| {
        format!(
            "read transcript modification time {}",
            source_path.display()
        )
    })?;
    let prefix_probes = transcript_prefix_probes(&mut file, &source_path, metadata.len())?;
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
        prefix_probes,
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

fn transcript_prefix_probes(
    file: &mut File,
    path: &Path,
    len: u64,
) -> anyhow::Result<Vec<TranscriptProbe>> {
    if len == 0 || path.extension().and_then(|extension| extension.to_str()) == Some("zst") {
        return Ok(Vec::new());
    }

    let sample_bytes = if len <= TRANSCRIPT_FULL_PREFIX_LIMIT {
        len
    } else {
        TRANSCRIPT_PROBE_BYTES
    };
    let offsets = if len <= TRANSCRIPT_FULL_PREFIX_LIMIT {
        vec![0]
    } else {
        let span = len - sample_bytes;
        let denominator = TRANSCRIPT_PROBE_COUNT - 1;
        (0..TRANSCRIPT_PROBE_COUNT)
            .map(|index| (span / denominator) * index + (span % denominator) * index / denominator)
            .collect::<Vec<_>>()
    };
    let mut probes = Vec::new();
    for offset in offsets {
        file.seek(SeekFrom::Start(offset))
            .with_context(|| format!("seek transcript {}", path.display()))?;
        let mut bytes = vec![0; sample_bytes as usize];
        if let Err(error) = file.read_exact(&mut bytes) {
            if error.kind() == std::io::ErrorKind::UnexpectedEof {
                return Ok(Vec::new());
            }
            return Err(error).with_context(|| format!("read transcript {}", path.display()));
        }
        probes.push(TranscriptProbe { offset, bytes });
    }
    Ok(probes)
}

fn prefix_matches(
    file: &mut File,
    path: &Path,
    probes: &[TranscriptProbe],
) -> anyhow::Result<bool> {
    for probe in probes {
        file.seek(SeekFrom::Start(probe.offset))
            .with_context(|| format!("seek transcript prefix probe {}", path.display()))?;
        let mut bytes = vec![0; probe.bytes.len()];
        match file.read_exact(&mut bytes) {
            Ok(()) if bytes == probe.bytes => {}
            Ok(()) => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read transcript prefix probe {}", path.display()));
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use rstest::rstest;

    use super::{TranscriptChange, transcript_fingerprint};
    use crate::commands::agent::types::Engine;

    #[test]
    fn transcript_append_reads_only_bytes_after_the_cached_length() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("transcript.jsonl");
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
                .read_delta_from(&previous)
                .expect("delta should be read"),
            (TranscriptChange::Appended, b"second record\n".to_vec()),
        );
    }

    #[test]
    fn transcript_rewrite_requires_a_full_read() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("transcript.jsonl");
        std::fs::write(&path, b"first record\n").expect("transcript should be written");
        let previous = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");
        std::fs::write(&path, b"other record\n").expect("transcript should be rewritten");
        let current = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");

        assert_eq!(
            current
                .read_delta_from(&previous)
                .expect("delta should be read"),
            (TranscriptChange::Rewritten, Vec::new()),
        );
    }

    #[rstest]
    #[case::small_prefix(512, 600, 256)]
    #[case::within_full_prefix_limit(2048, 2500, 100)]
    #[case::sampled_large_prefix(4096, 4500, 1881)]
    fn transcript_longer_in_place_rewrite_requires_a_full_read(
        #[case] previous_len: usize,
        #[case] current_len: usize,
        #[case] changed_offset: usize,
    ) {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("transcript.jsonl");
        std::fs::write(&path, vec![b'a'; previous_len]).expect("transcript should be written");
        let previous = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");
        let mut rewritten = vec![b'a'; current_len];
        rewritten[changed_offset] = b'b';
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("transcript should open for rewriting");
        file.set_len(0).expect("transcript should be truncated");
        file.write_all(&rewritten)
            .expect("rewritten transcript should be written");
        drop(file);
        let current = transcript_fingerprint(Engine::Claude, Some(&path))
            .expect("transcript metadata should be read")
            .expect("transcript should exist");

        assert_eq!(
            current
                .read_delta_from(&previous)
                .expect("rewrite should be detected"),
            (TranscriptChange::Rewritten, Vec::new()),
        );
    }
}
