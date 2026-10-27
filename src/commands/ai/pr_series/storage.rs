use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use uuid::Uuid;

use super::model::PrSeriesPlan;
use crate::infra::git::file_lock::{lock_exclusive, open_lock_file};

const PLAN_DIRECTORY: &str = "pr-series";

pub fn save_plan(task_id: &str, plan: &PrSeriesPlan) -> Result<()> {
    let directory = state_dir()?.join("armyknife").join(PLAN_DIRECTORY);
    let task_id = canonical_task_id(task_id)?;
    save_plan_at(&directory, &task_id, plan)
}

pub fn load_plan(task_id: &str) -> Result<PrSeriesPlan> {
    let directory = state_dir()?.join("armyknife").join(PLAN_DIRECTORY);
    let task_id = canonical_task_id(task_id)?;
    load_plan_at(&directory, &task_id)
}

fn save_plan_at(directory: &Path, task_id: &str, plan: &PrSeriesPlan) -> Result<()> {
    plan.validate()?;
    create_dir_secure(directory)?;
    let path = plan_path(directory, task_id)?;
    let lock_path = directory.join(format!("{task_id}.lock"));
    let lock_file = open_lock_file(&lock_path, "PR series plan")?;
    lock_exclusive(&lock_file, "PR series plan")?;
    reject_symlink(&path)?;

    // The lock covers the replacement so concurrent worktrees cannot interleave writes.
    let mut temp_file = tempfile::NamedTempFile::new_in(directory).with_context(|| {
        format!(
            "failed to create a temporary plan in {}",
            directory.display()
        )
    })?;
    serde_json::to_writer_pretty(&mut temp_file, plan).context("failed to serialize plan")?;
    temp_file.write_all(b"\n")?;
    temp_file.as_file().sync_all()?;
    temp_file
        .persist(&path)
        .with_context(|| format!("failed to save plan: {}", path.display()))?;
    Ok(())
}

fn load_plan_at(directory: &Path, task_id: &str) -> Result<PrSeriesPlan> {
    let path = plan_path(directory, task_id)?;
    if !directory.exists() {
        bail!("no PR series plan is saved for tq task `{task_id}`");
    }
    let lock_path = directory.join(format!("{task_id}.lock"));
    let lock_file = open_lock_file(&lock_path, "PR series plan")?;
    lock_exclusive(&lock_file, "PR series plan")?;
    reject_symlink(&path)?;
    let content = fs::read_to_string(&path)
        .with_context(|| format!("failed to read PR series plan: {}", path.display()))?;
    let plan: PrSeriesPlan = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse PR series plan: {}", path.display()))?;
    plan.validate()?;
    Ok(plan)
}

fn plan_path(directory: &Path, task_id: &str) -> Result<PathBuf> {
    let task_id = canonical_task_id(task_id)?;
    Ok(directory.join(format!("{task_id}.json")))
}

fn canonical_task_id(task_id: &str) -> Result<String> {
    Uuid::parse_str(task_id)
        .map(|id| id.to_string())
        .with_context(|| format!("invalid tq task UUID `{task_id}`"))
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "refusing to use a symlink for PR series plan: {}",
                path.display()
            );
        }
        Ok(metadata) if !metadata.is_file() => {
            bail!("PR series plan is not a regular file: {}", path.display());
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect PR series plan: {}", path.display())),
    }
}

#[cfg(target_os = "macos")]
fn state_dir() -> Result<PathBuf> {
    crate::shared::dirs::home_dir()
        .map(|home| home.join("Library").join("Application Support"))
        .context("user state directory is unavailable")
}

#[cfg(not(target_os = "macos"))]
fn state_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME")
        && !path.is_empty()
    {
        return Ok(PathBuf::from(path));
    }
    crate::shared::dirs::home_dir()
        .map(|home| home.join(".local").join("state"))
        .context("user state directory is unavailable")
}

fn create_dir_secure(directory: &Path) -> std::io::Result<()> {
    if !directory.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
        return Ok(());
    }
    let metadata = fs::metadata(directory)?;
    let mut permissions = metadata.permissions();
    if permissions.mode() & 0o077 != 0 {
        permissions.set_mode(0o700);
        fs::set_permissions(directory, permissions)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn plan() -> PrSeriesPlan {
        PrSeriesPlan {
            name: "Notification preferences".to_string(),
            purpose: "Let users choose which notifications they receive.".to_string(),
            pull_requests: vec![super::super::model::PrSeriesEntry {
                id: "schema".to_string(),
                summary: "Add preference tables".to_string(),
                depends_on: Vec::new(),
                repository: "acme/notify-api".to_string(),
                tq_leaf_id: "leaf-schema".to_string(),
            }],
        }
    }

    #[test]
    fn saves_and_loads_a_plan_by_task_uuid() {
        let directory = TempDir::new().unwrap();
        let expected = plan();
        save_plan_at(
            directory.path(),
            "00000000-0000-4000-8000-000000000001",
            &expected,
        )
        .unwrap();

        assert_eq!(
            load_plan_at(directory.path(), "00000000-0000-4000-8000-000000000001").unwrap(),
            expected,
        );
    }

    #[test]
    fn rejects_an_invalid_replacement_without_changing_the_saved_plan() {
        let directory = TempDir::new().unwrap();
        let expected = plan();
        save_plan_at(
            directory.path(),
            "00000000-0000-4000-8000-000000000001",
            &expected,
        )
        .unwrap();
        let invalid = PrSeriesPlan {
            pull_requests: Vec::new(),
            ..expected.clone()
        };

        let result = save_plan_at(
            directory.path(),
            "00000000-0000-4000-8000-000000000001",
            &invalid,
        )
        .err()
        .map(|error| error.to_string());

        assert_eq!(
            (
                result,
                load_plan_at(directory.path(), "00000000-0000-4000-8000-000000000001").unwrap(),
            ),
            (
                Some("plan must contain at least one pull request".to_string()),
                expected,
            ),
        );
    }
}
