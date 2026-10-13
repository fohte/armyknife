use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

pub(super) fn session_ids_in(sessions_dir: &Path) -> Result<Vec<String>> {
    if !sessions_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(sessions_dir)
        .with_context(|| format!("reading sessions dir {}", sessions_dir.display()))?
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();

    Ok(entries
        .into_iter()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                return None;
            }
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_owned)
        })
        .collect())
}
