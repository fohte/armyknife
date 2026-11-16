use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

pub(super) fn normalize_directory(path: &str, home: Option<&Path>) -> Result<PathBuf> {
    let candidate = if path == "~" || path.starts_with("~/") {
        let home = home.context("could not determine the home directory")?;
        if path == "~" {
            home.to_path_buf()
        } else {
            home.join(path.strip_prefix("~/").unwrap_or_default())
        }
    } else {
        PathBuf::from(path)
    };
    let canonical = candidate
        .canonicalize()
        .with_context(|| format!("directory is unavailable: {}", candidate.display()))?;
    if !canonical.is_dir() {
        bail!("directory must be an existing directory");
    }
    Ok(canonical)
}
