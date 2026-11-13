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
use super::store::{self, AgentFile};
use crate::shared::env_var::EnvVars;

const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
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

pub(super) fn record_files(
    session_id: &str,
    paths: &[PathBuf],
    caption: Option<&str>,
    sessions_dir: &Path,
    storage_root: &Path,
) -> Result<Vec<AgentFile>> {
    ensure!(!paths.is_empty(), "at least one file is required");
    ensure!(
        paths.len() <= MAX_FILES_PER_SEND,
        "a send can include at most {MAX_FILES_PER_SEND} files"
    );
    let mut pending_files = Vec::with_capacity(paths.len());
    let mut total_size = 0_u64;
    for source in paths {
        let mut file =
            File::open(source).with_context(|| format!("opening file {}", source.display()))?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .with_context(|| format!("reading file {}", source.display()))?;
        ensure!(
            bytes.len() as u64 <= MAX_FILE_BYTES,
            "file exceeds the 10 MiB limit: {}",
            source.display()
        );
        total_size = total_size
            .checked_add(bytes.len() as u64)
            .context("file size overflow")?;
        ensure!(
            total_size <= MAX_SEND_BYTES,
            "files exceed the 50 MiB limit"
        );
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .map(sanitize_name)
            .filter(|name| !name.is_empty())
            .context("file name is not valid UTF-8")?;
        let mime_type = image_mime_type(&bytes).map(str::to_owned);
        pending_files.push((name, mime_type, bytes));
    }

    let directory = session_directory(storage_root, session_id)?;
    let mut stored = Vec::with_capacity(pending_files.len());
    let created_at = Utc::now();
    for (position, (name, mime_type, bytes)) in pending_files.into_iter().enumerate() {
        let id = Uuid::new_v4().to_string();
        let path = directory.join(&id);
        write_new_file(&path, &bytes)?;
        let record = AgentFile {
            id,
            position: position as i64,
            name,
            caption: caption.map(str::to_owned),
            mime_type,
            size: bytes.len() as i64,
            created_at,
        };
        if let Err(error) = store::insert_agent_file_from(sessions_dir, session_id, &record) {
            let _ = fs::remove_file(path);
            return Err(error);
        }
        stored.push(record);
    }
    Ok(stored)
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
    std::io::Write::write_all(&mut file, bytes)?;
    file.sync_all()?;
    Ok(())
}

fn set_private_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn sanitize_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim_matches('.');
    trimmed.chars().take(180).collect()
}

fn image_mime_type(bytes: &[u8]) -> Option<&'static str> {
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

#[cfg(test)]
mod tests {
    use std::fs;

    use clap::Parser;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::{read_stored_file, record_files};
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
