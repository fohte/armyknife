use std::path::Path;

use anyhow::{Context, Result};

use crate::commands::agent::types::{Engine, Session};

use super::routes::RouteLoaders;
use super::{SkillSuggestion, archived_codex_transcript_path};

pub(super) fn find_from_recent_sessions(
    engine: Engine,
    cwd: Option<&Path>,
    exclude_session_id: Option<&str>,
    loaders: &RouteLoaders,
) -> Result<Vec<SkillSuggestion>> {
    let mut candidates = loaders.sessions()?;
    candidates.retain(|candidate| {
        candidate.engine == engine && exclude_session_id != Some(candidate.session_id.as_str())
    });
    candidates.sort_by(|left, right| {
        cwd.map(|cwd| left.cwd != cwd)
            .cmp(&cwd.map(|cwd| right.cwd != cwd))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| right.created_at.cmp(&left.created_at))
    });

    let codex_home = crate::shared::dirs::codex_home_dir();
    for candidate in candidates {
        match cached_skill_listing_for_session(&candidate, loaders, codex_home.as_deref()) {
            Ok(Some(skills)) => return Ok(skills),
            Ok(None) => {}
            Err(error) => tracing::warn!(
                session_id = %candidate.session_id,
                %error,
                "failed to inspect candidate transcript skill listing"
            ),
        }
    }

    Ok(Vec::new())
}

pub(super) fn cached_skill_listing_for_session(
    session: &Session,
    loaders: &RouteLoaders,
    codex_home: Option<&Path>,
) -> Result<Option<Vec<SkillSuggestion>>> {
    let fingerprint = skill_listing_cache_fingerprint(session, codex_home)?;
    loaders.cached_skill_listing(session, &fingerprint, || {
        skill_listing_for_session(session, codex_home)
    })
}

fn skill_listing_cache_fingerprint(session: &Session, codex_home: Option<&Path>) -> Result<String> {
    let archived_path = archived_codex_transcript_path(
        session.engine,
        session.transcript_path.as_deref(),
        codex_home,
    )?;
    let Some(path) = archived_path
        .as_deref()
        .or(session.transcript_path.as_deref())
    else {
        return Ok("no-transcript-path".to_string());
    };
    for candidate in [path.to_path_buf(), super::compressed_path(path)] {
        match std::fs::metadata(&candidate) {
            Ok(metadata) => {
                return Ok(format!(
                    "{}:{}:{:?}",
                    candidate.display(),
                    metadata.len(),
                    metadata.modified().ok(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("read transcript metadata {}", candidate.display()));
            }
        }
    }
    Ok(format!("{}:missing", path.display()))
}

fn skill_listing_for_session(
    session: &Session,
    codex_home: Option<&Path>,
) -> Result<Option<Vec<SkillSuggestion>>> {
    let archived_path = archived_codex_transcript_path(
        session.engine,
        session.transcript_path.as_deref(),
        codex_home,
    )?;
    let Some(path) = archived_path
        .as_deref()
        .or(session.transcript_path.as_deref())
    else {
        return Ok(None);
    };
    let Some(reader) = super::open_transcript(path, true)? else {
        return Ok(None);
    };
    Ok(super::parser::skill_suggestions::read_transcript_listing(
        reader,
        session.engine,
    )?)
}
