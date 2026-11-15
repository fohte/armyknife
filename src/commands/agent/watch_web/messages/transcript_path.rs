use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::commands::agent::types::Engine;

pub(super) fn codex_home_dir() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::shared::dirs::home_dir().map(|home| home.join(".codex")))
}

pub(super) fn archived_codex_transcript_path(
    engine: Engine,
    transcript_path: Option<&Path>,
    codex_home: Option<&Path>,
) -> Result<Option<PathBuf>> {
    if engine != Engine::Codex {
        return Ok(None);
    }
    let (Some(transcript_path), Some(codex_home)) = (transcript_path, codex_home) else {
        return Ok(None);
    };
    match std::fs::metadata(transcript_path) {
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!("read transcript metadata {}", transcript_path.display())
            });
        }
    }
    let Some(file_name) = transcript_path.file_name() else {
        return Ok(None);
    };
    let archived_path = codex_home.join("archived_sessions").join(file_name);
    match std::fs::metadata(&archived_path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(archived_path)),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| {
            format!(
                "read archived transcript metadata {}",
                archived_path.display()
            )
        }),
    }
}
