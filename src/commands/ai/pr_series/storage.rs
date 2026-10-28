use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use uuid::Uuid;

use super::model::PrSeriesPlan;
use crate::infra::git::file_lock::{lock_exclusive, open_lock_file};

const PLAN_DIRECTORY: &str = "pr-series";

pub fn save_plan_for_task(task_id: &str, task_number: u32, plan: &PrSeriesPlan) -> Result<()> {
    plan_storage()?.save_plan_for_task(task_id, task_number, plan)
}

pub fn load_plan(task_id: &str) -> Result<PrSeriesPlan> {
    plan_storage()?.load_plan(task_id)
}

pub fn load_plan_if_present(task_id: &str) -> Result<Option<PrSeriesPlan>> {
    plan_storage()?.load_plan_if_present(task_id)
}

fn plan_storage() -> Result<PrSeriesPlanStorage> {
    Ok(PrSeriesPlanStorage::new(
        state_dir()?.join("armyknife").join(PLAN_DIRECTORY),
    ))
}

pub struct PrSeriesPlanStorage {
    directory: PathBuf,
}

impl PrSeriesPlanStorage {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn save_plan(&self, task_id: &str, plan: &PrSeriesPlan) -> Result<()> {
        let task_id = canonical_task_id(task_id)?;
        save_plan_at(&self.directory, &task_id, plan)
    }

    pub fn save_plan_for_task(
        &self,
        task_id: &str,
        task_number: u32,
        plan: &PrSeriesPlan,
    ) -> Result<()> {
        let task_id = canonical_task_id(task_id)?;
        self.save_plan(&task_id, plan)?;
        save_task_number_alias_at(&self.directory, task_number, &task_id)
    }

    pub fn load_plan(&self, task_id: &str) -> Result<PrSeriesPlan> {
        let task_id = canonical_task_id(task_id)?;
        load_plan_at(&self.directory, &task_id)
    }

    pub fn load_plan_if_present(&self, task_id: &str) -> Result<Option<PrSeriesPlan>> {
        let task_id = match Uuid::parse_str(task_id) {
            Ok(task_id) => task_id.to_string(),
            Err(_) => {
                let task_number = task_id
                    .parse::<u32>()
                    .with_context(|| format!("invalid tq task ID `{task_id}`"))?;
                let Some(task_id) = load_task_number_alias_at(&self.directory, task_number)? else {
                    return Ok(None);
                };
                task_id
            }
        };
        load_plan_if_present_at(&self.directory, &task_id)
    }
}

fn save_task_number_alias_at(directory: &Path, task_number: u32, task_id: &str) -> Result<()> {
    create_dir_secure(directory)?;
    let path = task_number_alias_path(directory, task_number);
    let lock_path = directory.join(format!("task-number-{task_number}.lock"));
    let lock_file = open_lock_file(&lock_path, "PR series task number alias")?;
    lock_exclusive(&lock_file, "PR series task number alias")?;
    reject_symlink(&path)?;

    let mut temp_file = tempfile::NamedTempFile::new_in(directory).with_context(|| {
        format!(
            "failed to create a temporary task number alias in {}",
            directory.display()
        )
    })?;
    temp_file.write_all(task_id.as_bytes())?;
    temp_file.as_file().sync_all()?;
    temp_file
        .persist(&path)
        .with_context(|| format!("failed to save task number alias: {}", path.display()))?;
    Ok(())
}

fn load_task_number_alias_at(directory: &Path, task_number: u32) -> Result<Option<String>> {
    let path = task_number_alias_path(directory, task_number);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "refusing to use a symlink for PR series task number alias: {}",
                path.display()
            );
        }
        Ok(metadata) if !metadata.is_file() => {
            bail!(
                "PR series task number alias is not a regular file: {}",
                path.display()
            );
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to inspect PR series task number alias: {}",
                    path.display()
                )
            });
        }
    }
    let lock_path = directory.join(format!("task-number-{task_number}.lock"));
    let lock_file = open_lock_file(&lock_path, "PR series task number alias")?;
    lock_exclusive(&lock_file, "PR series task number alias")?;
    reject_symlink(&path)?;
    let task_id = fs::read_to_string(&path).with_context(|| {
        format!(
            "failed to read PR series task number alias: {}",
            path.display()
        )
    })?;
    Ok(Some(canonical_task_id(task_id.trim())?))
}

fn task_number_alias_path(directory: &Path, task_number: u32) -> PathBuf {
    directory.join(format!("task-number-{task_number}.alias"))
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
    load_plan_if_present_at(directory, task_id)?
        .with_context(|| format!("no PR series plan is saved for tq task `{task_id}`"))
}

fn load_plan_if_present_at(directory: &Path, task_id: &str) -> Result<Option<PrSeriesPlan>> {
    let path = plan_path(directory, task_id)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "refusing to use a symlink for PR series plan: {}",
                path.display()
            );
        }
        Ok(metadata) if !metadata.is_file() => {
            bail!("PR series plan is not a regular file: {}", path.display());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect PR series plan: {}", path.display()));
        }
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
    Ok(Some(plan))
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
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;

    struct TestPlanStorage {
        _directory: TempDir,
        storage: PrSeriesPlanStorage,
    }

    #[fixture]
    fn plan_storage() -> TestPlanStorage {
        let directory = TempDir::new().unwrap();
        let storage = PrSeriesPlanStorage::new(directory.path().to_path_buf());
        TestPlanStorage {
            _directory: directory,
            storage,
        }
    }

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

    #[rstest]
    fn saves_and_loads_a_plan_by_task_uuid(plan_storage: TestPlanStorage) {
        let expected = plan();
        plan_storage
            .storage
            .save_plan("00000000-0000-4000-8000-000000000001", &expected)
            .unwrap();

        assert_eq!(
            plan_storage
                .storage
                .load_plan("00000000-0000-4000-8000-000000000001")
                .unwrap(),
            expected,
        );
    }

    #[rstest]
    #[case::uuid("00000000-0000-4000-8000-000000000001")]
    #[case::task_number("4242")]
    fn returns_none_when_a_task_has_no_saved_plan(
        plan_storage: TestPlanStorage,
        #[case] task_id: &str,
    ) {
        assert_eq!(
            plan_storage.storage.load_plan_if_present(task_id).unwrap(),
            None,
        );
    }

    #[rstest]
    fn loads_a_plan_by_its_task_number_alias(plan_storage: TestPlanStorage) {
        let expected = plan();
        plan_storage
            .storage
            .save_plan_for_task("00000000-0000-4000-8000-000000000001", 4242, &expected)
            .unwrap();

        assert_eq!(
            plan_storage.storage.load_plan_if_present("4242").unwrap(),
            Some(expected),
        );
    }

    #[rstest]
    fn rejects_an_invalid_replacement_without_changing_the_saved_plan(
        plan_storage: TestPlanStorage,
    ) {
        let expected = plan();
        plan_storage
            .storage
            .save_plan("00000000-0000-4000-8000-000000000001", &expected)
            .unwrap();
        let invalid = PrSeriesPlan {
            pull_requests: Vec::new(),
            ..expected.clone()
        };

        let result = plan_storage
            .storage
            .save_plan("00000000-0000-4000-8000-000000000001", &invalid)
            .err()
            .map(|error| error.to_string());

        assert_eq!(
            (
                result,
                plan_storage
                    .storage
                    .load_plan("00000000-0000-4000-8000-000000000001")
                    .unwrap(),
            ),
            (
                Some("plan must contain at least one pull request".to_string()),
                expected,
            ),
        );
    }
}
