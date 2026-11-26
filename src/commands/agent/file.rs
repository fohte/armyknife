use std::fs::{self, File, OpenOptions};
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::file_attachment_support::{
    FILE_TYPE_PREFIX_BYTES, MAX_AGENT_FILE_BYTES as MAX_FILE_BYTES,
};
use super::store::AgentFile;

pub(crate) fn read_stored_file(
    storage_root: &Path,
    session_id: &str,
    file: &AgentFile,
) -> Result<Option<Vec<u8>>> {
    let Some((stored_file, size)) = open_stored_file(storage_root, session_id, file)? else {
        return Ok(None);
    };
    let mut bytes = Vec::with_capacity(size as usize);
    stored_file
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != size {
        return Ok(None);
    }
    Ok(Some(bytes))
}

pub(crate) fn read_stored_file_prefix(
    storage_root: &Path,
    session_id: &str,
    file: &AgentFile,
) -> Result<Option<(Vec<u8>, usize)>> {
    let Some((mut stored_file, size)) = open_stored_file(storage_root, session_id, file)? else {
        return Ok(None);
    };
    let Some(bytes) = read_file_type_prefix(&mut stored_file, size)? else {
        return Ok(None);
    };
    Ok(Some((bytes, size as usize)))
}

pub(crate) fn read_file_type_prefix(file: &mut File, file_size: u64) -> Result<Option<Vec<u8>>> {
    let expected_size = file_size.min(FILE_TYPE_PREFIX_BYTES as u64) as usize;
    let mut bytes = Vec::with_capacity(expected_size);
    file.take(FILE_TYPE_PREFIX_BYTES as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() != expected_size {
        return Ok(None);
    }
    Ok(Some(bytes))
}

fn open_stored_file(
    storage_root: &Path,
    session_id: &str,
    file: &AgentFile,
) -> Result<Option<(File, u64)>> {
    if Uuid::parse_str(&file.id).is_err() {
        return Ok(None);
    }
    let Ok(directory) = session_directory(storage_root, session_id) else {
        return Ok(None);
    };
    let path = directory.join(&file.id);
    let path_metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if file.size < 0
        || path_metadata.len() != file.size as u64
        || path_metadata.len() > MAX_FILE_BYTES
    {
        return Ok(None);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let stored_file = options.open(path)?;
    let metadata = stored_file.metadata()?;
    if !metadata.is_file() || metadata.len() != file.size as u64 || metadata.len() > MAX_FILE_BYTES
    {
        return Ok(None);
    }
    Ok(Some((stored_file, metadata.len())))
}

pub(crate) fn storage_root() -> Result<PathBuf> {
    let path = crate::shared::dirs::state_dir()
        .context("user state directory is unavailable")?
        .join("armyknife")
        .join("agent-watch-files");
    Ok(path)
}

pub(crate) fn remove_session_storage(storage_root: &Path, session_id: &str) -> Result<()> {
    let root_metadata = match fs::symlink_metadata(storage_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        root_metadata.is_dir() && !root_metadata.file_type().is_symlink(),
        "file attachment path is not a real directory"
    );
    let root = storage_root.canonicalize()?;
    let digest = crate::shared::hex::encode(&Sha256::digest(session_id.as_bytes()));
    let directory = root.join(digest);
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "file attachment session path is not a real directory"
    );
    let directory = directory.canonicalize()?;
    ensure!(
        directory.parent() == Some(root.as_path()),
        "file attachment path escaped storage directory"
    );
    fs::remove_dir_all(directory)?;
    Ok(())
}

pub(crate) fn session_directory(storage_root: &Path, session_id: &str) -> Result<PathBuf> {
    fs::create_dir_all(storage_root)?;
    let metadata = fs::symlink_metadata(storage_root)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "file attachment path is not a real directory"
    );
    set_private_permissions(storage_root)?;
    let root = storage_root.canonicalize()?;
    let digest = crate::shared::hex::encode(&Sha256::digest(session_id.as_bytes()));
    ensure_directory(&root, &digest)
}

fn ensure_directory(parent: &Path, name: &str) -> Result<PathBuf> {
    let parent = parent.canonicalize()?;
    let path = parent.join(name);
    match fs::create_dir(&path) {
        Ok(()) => set_private_permissions(&path)?,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "file attachment path is not a real directory"
    );
    let canonical = path.canonicalize()?;
    ensure!(
        canonical.parent() == Some(parent.as_path()),
        "file attachment path escaped storage directory"
    );
    Ok(canonical)
}

pub(crate) fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path)?;
    let result = std::io::Write::write_all(&mut file, bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(error) = result {
        if let Err(cleanup_error) = fs::remove_file(path)
            && cleanup_error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %path.display(), %cleanup_error, "failed to remove a partially written agent file");
        }
        return Err(error.into());
    }
    Ok(())
}

fn set_private_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use clap::Parser;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use crate::cli::Cli;

    use super::{
        FILE_TYPE_PREFIX_BYTES, read_stored_file, read_stored_file_prefix, session_directory,
        write_new_file,
    };
    use crate::commands::agent::store::AgentFile;

    #[fixture]
    fn storage() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn store_file(storage_root: &std::path::Path, bytes: &[u8]) -> AgentFile {
        let id = uuid::Uuid::new_v4().to_string();
        let directory = session_directory(storage_root, "fixture-session").unwrap();
        write_new_file(&directory.join(&id), bytes).unwrap();
        AgentFile {
            id,
            position: 0,
            name: "fixture.bin".to_owned(),
            caption: None,
            mime_type: None,
            size: bytes.len() as i64,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn rejects_the_removed_file_subcommand() {
        let result = Cli::try_parse_from(["a", "agent", concat!("fi", "le")]);

        assert_eq!(
            result.err().map(|error| error.kind()),
            Some(clap::error::ErrorKind::InvalidSubcommand)
        );
    }

    #[rstest]
    fn reads_a_stored_file(storage: TempDir) {
        let storage_root = storage.path().join("attachments");
        let file = store_file(&storage_root, b"fixture content");

        assert_eq!(
            read_stored_file(&storage_root, "fixture-session", &file).unwrap(),
            Some(b"fixture content".to_vec())
        );
    }

    #[rstest]
    fn reads_only_the_file_type_prefix_for_stored_attachments(storage: TempDir) {
        let storage_root = storage.path().join("attachments");
        let content = vec![b'x'; FILE_TYPE_PREFIX_BYTES + 32];
        let file = store_file(&storage_root, &content);

        let actual = read_stored_file_prefix(&storage_root, "fixture-session", &file).unwrap();

        assert_eq!(
            actual,
            Some((vec![b'x'; FILE_TYPE_PREFIX_BYTES], content.len()))
        );
    }
}
