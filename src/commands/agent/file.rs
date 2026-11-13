use std::fs::{self, File, OpenOptions};
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use chrono::Utc;
use clap::{Args, Subcommand};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::error::CcError;
use super::file_attachment_support::{
    MAX_AGENT_FILE_BYTES as MAX_FILE_BYTES, image_format_from_bytes, sanitize_basename,
};
use super::store::{self, AgentFile};
use crate::shared::env_var::EnvVars;

const MAX_FILES_PER_SEND: usize = 16;
const MAX_SEND_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum FileCommands {
    /// Send files to the calling session's watch-web conversation
    Send(SendArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SendArgs {
    /// Files to attach to the conversation
    #[arg(required = true, num_args = 1..)]
    paths: Vec<PathBuf>,
    /// Caption shown with the attachments
    #[arg(long)]
    caption: Option<String>,
}

pub fn run(command: &FileCommands) -> Result<()> {
    match command {
        FileCommands::Send(args) => send(args),
    }
}

fn send(args: &SendArgs) -> Result<()> {
    let session_id = EnvVars::load()
        .own_session_id()
        .ok_or(CcError::SelfSessionUnknown)?;
    let records = record_files(
        &session_id,
        &args.paths,
        args.caption.as_deref(),
        &store::sessions_dir()?,
        &storage_root()?,
    )?;
    println!("Sent {} file(s) to watch-web", records.len());
    Ok(())
}

fn record_files(
    session_id: &str,
    paths: &[PathBuf],
    caption: Option<&str>,
    sessions_dir: &Path,
    storage_root: &Path,
) -> Result<Vec<AgentFile>> {
    validate_send_limits(paths.len(), &[])?;
    let mut pending_files = Vec::with_capacity(paths.len());
    let mut file_sizes = Vec::with_capacity(paths.len());
    for source in paths {
        let mut file =
            File::open(source).with_context(|| format!("opening file {}", source.display()))?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .with_context(|| format!("reading file {}", source.display()))?;
        file_sizes.push(bytes.len() as u64);
        validate_send_limits(paths.len(), &file_sizes)
            .with_context(|| format!("validating file {}", source.display()))?;
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .map(sanitize_basename)
            .context("file name is not valid UTF-8")?;
        let mime_type = image_format_from_bytes(&bytes).map(|(mime_type, _)| mime_type.to_owned());
        pending_files.push((name, mime_type, bytes));
    }

    let directory = session_directory(storage_root, session_id)?;
    let mut stored = Vec::with_capacity(pending_files.len());
    let mut created_paths = Vec::with_capacity(pending_files.len());
    let created_at = Utc::now();
    for (position, (name, mime_type, bytes)) in pending_files.into_iter().enumerate() {
        let id = Uuid::new_v4().to_string();
        let path = directory.join(&id);
        if let Err(error) = write_new_file(&path, &bytes) {
            remove_created_files(&created_paths);
            return Err(error);
        }
        created_paths.push(path);
        let record = AgentFile {
            id,
            position: position as i64,
            name,
            caption: caption.map(str::to_owned),
            mime_type,
            size: bytes.len() as i64,
            created_at,
        };
        stored.push(record);
    }
    if let Err(error) = store::insert_agent_files_from(sessions_dir, session_id, &stored) {
        remove_created_files(&created_paths);
        return Err(error);
    }
    Ok(stored)
}

fn validate_send_limits(file_count: usize, file_sizes: &[u64]) -> Result<()> {
    ensure!(file_count > 0, "at least one file is required");
    ensure!(
        file_count <= MAX_FILES_PER_SEND,
        "a send can include at most {MAX_FILES_PER_SEND} files"
    );
    let mut total_size = 0_u64;
    for size in file_sizes {
        ensure!(*size <= MAX_FILE_BYTES, "file exceeds the 10 MiB limit");
        total_size = total_size
            .checked_add(*size)
            .context("file size overflow")?;
        ensure!(
            total_size <= MAX_SEND_BYTES,
            "files exceed the 50 MiB limit"
        );
    }
    Ok(())
}

fn remove_created_files(paths: &[PathBuf]) {
    for path in paths {
        if let Err(error) = fs::remove_file(path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %path.display(), %error, "failed to remove a partial agent file send");
        }
    }
}

pub(crate) fn read_stored_file(
    storage_root: &Path,
    session_id: &str,
    file: &AgentFile,
) -> Result<Option<Vec<u8>>> {
    if Uuid::parse_str(&file.id).is_err() {
        return Ok(None);
    }
    let Ok(directory) = session_directory(storage_root, session_id) else {
        return Ok(None);
    };
    let path = directory.join(&file.id);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => metadata,
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if file.size < 0 || metadata.len() != file.size as u64 || metadata.len() > MAX_FILE_BYTES {
        return Ok(None);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    options
        .open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != file.size as u64 {
        return Ok(None);
    }
    Ok(Some(bytes))
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

fn session_directory(storage_root: &Path, session_id: &str) -> Result<PathBuf> {
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

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
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
    use std::fs;

    use chrono::Utc;
    use clap::Parser;
    use rstest::{fixture, rstest};
    use sha2::Digest;
    use tempfile::TempDir;

    use super::{MAX_FILE_BYTES, read_stored_file, record_files, validate_send_limits};
    use crate::cli::{Cli, Commands};
    use crate::commands::agent::store::{self, AgentFile};
    use crate::commands::agent::{AgentCommands, FileCommands};

    #[fixture]
    fn storage() -> (TempDir, TempDir) {
        (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
    }

    fn create_session(sessions_dir: &std::path::Path) {
        store::session_store_revision_from(sessions_dir).unwrap();
        rusqlite::Connection::open(sessions_dir.join("sessions.db"))
            .unwrap()
            .execute(
                "INSERT INTO sessions (session_id, data, rev) VALUES (?1, '{}', 0)",
                ["fixture-session"],
            )
            .unwrap();
    }

    #[test]
    fn parses_multiple_file_paths_and_a_caption() {
        let parsed = Cli::try_parse_from([
            "a",
            "agent",
            "file",
            "send",
            "./sample-image.png",
            "./sample-report.pdf",
            "--caption",
            "Review these samples",
        ])
        .unwrap();
        let command = match parsed.command {
            Commands::Agent(AgentCommands::File(FileCommands::Send(args))) => {
                (args.paths, args.caption)
            }
            _ => panic!("file send arguments should parse"),
        };

        assert_eq!(
            command,
            (
                vec![
                    std::path::PathBuf::from("./sample-image.png"),
                    std::path::PathBuf::from("./sample-report.pdf")
                ],
                Some("Review these samples".to_owned())
            )
        );
    }

    #[rstest]
    fn copies_and_records_files_for_the_calling_session(storage: (TempDir, TempDir)) {
        let (sessions, state) = storage;
        create_session(sessions.path());
        let source = state.path().join("report.txt");
        fs::write(&source, b"fixture report").unwrap();
        let records = record_files(
            "fixture-session",
            std::slice::from_ref(&source),
            Some("Fixture caption"),
            sessions.path(),
            &state.path().join("attachments"),
        )
        .unwrap();
        fs::write(&source, b"changed source").unwrap();

        assert_eq!(
            (
                records[0].name.as_str(),
                records[0].caption.as_deref(),
                store::agent_file_from(sessions.path(), "fixture-session", &records[0].id).unwrap(),
                read_stored_file(
                    &state.path().join("attachments"),
                    "fixture-session",
                    &records[0]
                )
                .unwrap()
            ),
            (
                "report.txt",
                Some("Fixture caption"),
                Some(records[0].clone()),
                Some(b"fixture report".to_vec())
            )
        );
    }

    #[cfg(unix)]
    #[rstest]
    fn preserves_unix_backslashes_in_the_sent_file_name(storage: (TempDir, TempDir)) {
        let (sessions, state) = storage;
        create_session(sessions.path());
        let source = state.path().join("design\\final.pdf");
        fs::write(&source, b"fixture report").unwrap();

        let records = record_files(
            "fixture-session",
            &[source],
            None,
            sessions.path(),
            &state.path().join("attachments"),
        )
        .unwrap();
        let names = records
            .into_iter()
            .map(|record| record.name)
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["design_final.pdf"]);
    }

    #[rstest]
    fn rejects_an_oversized_input_before_persisting_files(storage: (TempDir, TempDir)) {
        let (sessions, state) = storage;
        create_session(sessions.path());
        let source = state.path().join("oversized-report.bin");
        fs::File::create(&source)
            .unwrap()
            .set_len(MAX_FILE_BYTES + 1)
            .unwrap();
        let storage_root = state.path().join("attachments");
        let session_storage = storage_root.join(crate::shared::hex::encode(&sha2::Sha256::digest(
            b"fixture-session",
        )));

        let result = record_files(
            "fixture-session",
            &[source],
            None,
            sessions.path(),
            &storage_root,
        )
        .map_err(|error| error.root_cause().to_string());
        let records = store::agent_files_from(sessions.path(), "fixture-session").unwrap();

        assert_eq!(
            (result, session_storage.exists(), records),
            (
                Err("file exceeds the 10 MiB limit".to_owned()),
                false,
                Vec::new(),
            )
        );
    }

    #[rstest]
    #[case::exact_per_file_limit(1, &[10 * 1024 * 1024], None)]
    #[case::over_per_file_limit(1, &[10 * 1024 * 1024 + 1], Some("file exceeds the 10 MiB limit"))]
    #[case::exact_combined_limit(5, &[10 * 1024 * 1024; 5], None)]
    #[case::over_combined_limit(6, &[10 * 1024 * 1024, 10 * 1024 * 1024, 10 * 1024 * 1024, 10 * 1024 * 1024, 10 * 1024 * 1024, 1], Some("files exceed the 50 MiB limit"))]
    #[case::exact_file_count_limit(16, &[], None)]
    #[case::over_file_count_limit(17, &[], Some("a send can include at most 16 files"))]
    fn validates_send_limits(
        #[case] file_count: usize,
        #[case] sizes: &[u64],
        #[case] expected_error: Option<&str>,
    ) {
        let actual = validate_send_limits(file_count, sizes).map_err(|error| error.to_string());
        let expected = expected_error.map_or(Ok(()), |error| Err(error.to_owned()));

        assert_eq!(actual, expected);
    }

    #[rstest]
    fn removes_copied_files_when_batch_registration_fails(storage: (TempDir, TempDir)) {
        let (sessions, state) = storage;
        store::session_store_revision_from(sessions.path()).unwrap();
        let first_source = state.path().join("first-report.txt");
        let second_source = state.path().join("second-report.txt");
        fs::write(&first_source, b"first report").unwrap();
        fs::write(&second_source, b"second report").unwrap();
        let storage_root = state.path().join("attachments");

        let result = record_files(
            "missing-session",
            &[first_source, second_source],
            None,
            sessions.path(),
            &storage_root,
        );
        let digest = crate::shared::hex::encode(&sha2::Sha256::digest(b"missing-session"));
        let stored_bytes = fs::read_dir(storage_root.join(digest))
            .unwrap()
            .filter_map(std::result::Result::ok)
            .count();
        let stored_records = store::agent_files_from(sessions.path(), "missing-session").unwrap();

        assert_eq!(
            (result.is_err(), stored_bytes, stored_records),
            (true, 0, Vec::new())
        );
    }

    #[rstest]
    fn batch_registration_rolls_back_when_a_file_id_conflicts(storage: (TempDir, TempDir)) {
        let (sessions, _) = storage;
        create_session(sessions.path());
        let revision_before = store::session_store_revision_from(sessions.path()).unwrap();
        let files = [
            AgentFile {
                id: "duplicate-file-id".to_owned(),
                position: 0,
                name: "first.txt".to_owned(),
                caption: None,
                mime_type: None,
                size: 1,
                created_at: Utc::now(),
            },
            AgentFile {
                id: "duplicate-file-id".to_owned(),
                position: 1,
                name: "second.txt".to_owned(),
                caption: None,
                mime_type: None,
                size: 1,
                created_at: Utc::now(),
            },
        ];

        let result = store::insert_agent_files_from(sessions.path(), "fixture-session", &files);
        let actual = (
            result.is_err(),
            store::agent_files_from(sessions.path(), "fixture-session").unwrap(),
            store::session_store_revision_from(sessions.path()).unwrap(),
        );

        assert_eq!(actual, (true, Vec::new(), revision_before));
    }

    #[rstest]
    fn rejects_files_that_are_not_registered_for_the_session(storage: (TempDir, TempDir)) {
        let (sessions, state) = storage;
        create_session(sessions.path());
        let unrecorded = AgentFile {
            id: uuid::Uuid::new_v4().to_string(),
            position: 0,
            name: "report.txt".to_owned(),
            caption: None,
            mime_type: None,
            size: 5,
            created_at: chrono::Utc::now(),
        };

        assert_eq!(
            (
                store::agent_file_from(sessions.path(), "fixture-session", &unrecorded.id).unwrap(),
                read_stored_file(
                    &state.path().join("attachments"),
                    "fixture-session",
                    &unrecorded
                )
                .unwrap()
            ),
            (None, None)
        );
    }
}
