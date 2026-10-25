use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::Provider;

const CONTEXT_TTL: Duration = Duration::from_secs(10 * 60);
const STALE_FILE_RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) enum ModelIdentity {
    Known { raw_slug: String },
    Unknown { reason: String },
}

impl ModelIdentity {
    pub(super) fn known(raw_slug: String) -> Result<Self> {
        if raw_slug.trim().is_empty() || raw_slug.contains(['\n', '\r', '\0']) {
            anyhow::bail!("runtime model slug is empty or cannot fit in a trailer");
        }
        Ok(Self::Known { raw_slug })
    }

    pub(super) fn unknown(reason: Option<&str>) -> Result<Self> {
        let reason = reason
            .filter(|reason| !reason.trim().is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "runtime model is unavailable and no explicit unknown reason was supplied"
                )
            })?;
        if reason.contains(['\n', '\r', '\0']) {
            anyhow::bail!("unknown reason must be a single line");
        }
        Ok(Self::Unknown {
            reason: reason.to_owned(),
        })
    }

    pub(super) fn trailer_value(&self) -> &str {
        match self {
            Self::Known { raw_slug } => raw_slug,
            Self::Unknown { .. } => "unknown",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct OperationContext {
    schema_version: u8,
    operation_id: String,
    provider: Provider,
    model: ModelIdentity,
    tool_use_id: String,
    created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct CommitTarget {
    pub(super) repository: String,
    pub(super) reference: String,
    pub(super) old_oid: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ClaimRecord {
    operation_id: String,
    #[serde(default)]
    target: Option<CommitTarget>,
}

impl OperationContext {
    pub(super) fn new(
        provider: Provider,
        model: ModelIdentity,
        tool_use_id: String,
    ) -> Result<Self> {
        if tool_use_id.trim().is_empty() || tool_use_id.contains(['\n', '\r', '\0']) {
            anyhow::bail!("tool use identifier is missing or invalid");
        }
        Ok(Self {
            schema_version: 1,
            operation_id: Uuid::new_v4().to_string(),
            provider,
            model,
            tool_use_id,
            created_at: unix_timestamp()?,
        })
    }

    pub(super) fn tool(&self) -> &'static str {
        self.provider.label()
    }

    pub(super) fn model(&self) -> &ModelIdentity {
        &self.model
    }

    fn operation_id(&self) -> &str {
        &self.operation_id
    }
}

pub(super) fn default_context_dir() -> PathBuf {
    std::env::temp_dir().join(format!("armyknife-agent-attribution-{}", current_user_id()))
}

pub(super) fn create(context: &OperationContext, directory: &Path) -> Result<PathBuf> {
    ensure_private_directory(directory)?;
    prune_stale_files(directory)?;

    let mut file = tempfile::Builder::new()
        .prefix("context-")
        .suffix(".json")
        .tempfile_in(directory)
        .context("failed to create private attribution context")?;
    serde_json::to_writer(&mut file, context).context("failed to serialize attribution context")?;
    file.flush()
        .context("failed to flush attribution context")?;
    file.as_file()
        .sync_all()
        .context("failed to sync attribution context")?;
    let (_, path) = file.keep().context("failed to keep attribution context")?;
    Ok(path)
}

pub(super) fn claim(path: &Path, now: i64) -> Result<OperationContext> {
    claim_with_target(path, now, None)
}

pub(super) fn claim_with_target(
    path: &Path,
    now: i64,
    target: Option<CommitTarget>,
) -> Result<OperationContext> {
    let context = read(path)?;
    if is_expired(&context, now) {
        anyhow::bail!("attribution context is stale");
    }

    let claim_path = claim_path(path);
    let mut claim_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&claim_path)
        .with_context(|| {
            format!(
                "attribution context was already claimed or could not be claimed: {}",
                path.display()
            )
        })?;
    if let Some(target) = target {
        let record = ClaimRecord {
            operation_id: context.operation_id().to_owned(),
            target: Some(target),
        };
        serde_json::to_writer(&mut claim_file, &record)
            .context("failed to mark attribution context as claimed")?;
    } else {
        claim_file
            .write_all(context.operation_id().as_bytes())
            .context("failed to mark attribution context as claimed")?;
    }
    claim_file
        .sync_all()
        .context("failed to sync attribution claim")?;
    Ok(context)
}

pub(super) fn claimed_context(path: &Path) -> Result<OperationContext> {
    let context = read(path)?;
    let claim = read_claim(path)?;
    verify_claim_matches(&context, &claim)?;
    Ok(context)
}

pub(super) fn claimed_final_object_context(
    path: &Path,
) -> Result<(OperationContext, CommitTarget)> {
    let context = read(path)?;
    let claim = read_claim(path)?;
    verify_claim_matches(&context, &claim)?;
    let target = claim
        .target
        .ok_or_else(|| anyhow::anyhow!("attribution claim has no final-object target"))?;
    Ok((context, target))
}

pub(super) fn is_expired(context: &OperationContext, now: i64) -> bool {
    let age = now.saturating_sub(context.created_at);
    age < 0 || Duration::from_secs(age as u64) > CONTEXT_TTL
}

pub(super) fn consume(path: &Path) -> Result<()> {
    let claim = claim_path(path);
    remove_if_present(path).context("failed to remove attribution context")?;
    remove_if_present(&claim).context("failed to remove attribution claim")
}

pub(super) fn discard(path: &Path) -> Result<()> {
    let claim = claim_path(path);
    remove_if_present(path).context("failed to discard attribution context")?;
    remove_if_present(&claim).context("failed to discard attribution claim")
}

pub(super) fn read(path: &Path) -> Result<OperationContext> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("attribution context is missing: {}", path.display()))?;
    if !metadata.file_type().is_file() {
        anyhow::bail!("attribution context is not a regular file");
    }
    verify_private_file(&metadata)?;
    let context: OperationContext = serde_json::from_slice(
        &fs::read(path)
            .with_context(|| format!("failed to read attribution context: {}", path.display()))?,
    )
    .context("attribution context is invalid")?;
    if context.schema_version != 1 {
        anyhow::bail!("unsupported attribution context version");
    }
    Ok(context)
}

fn claim_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".claimed");
    path.with_file_name(name)
}

fn read_claim(path: &Path) -> Result<ClaimRecord> {
    read_claim_if_present(path)?
        .ok_or_else(|| anyhow::anyhow!("attribution context has no matching commit-message claim"))
}

fn read_claim_if_present(path: &Path) -> Result<Option<ClaimRecord>> {
    let claim_path = claim_path(path);
    let raw = match fs::read_to_string(&claim_path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to read attribution claim"),
    };
    match serde_json::from_str(&raw) {
        Ok(claim) => Ok(Some(claim)),
        Err(_) => Ok(Some(ClaimRecord {
            operation_id: raw,
            target: None,
        })),
    }
}

fn verify_claim_matches(context: &OperationContext, claim: &ClaimRecord) -> Result<()> {
    if claim.operation_id != context.operation_id() {
        anyhow::bail!("attribution commit-message claim does not match its context");
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

pub(super) fn unix_timestamp() -> Result<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?;
    i64::try_from(elapsed.as_secs()).context("system timestamp exceeds the supported range")
}

fn ensure_private_directory(directory: &Path) -> Result<()> {
    fs::create_dir_all(directory).with_context(|| {
        format!(
            "failed to create attribution context directory {}",
            directory.display()
        )
    })?;
    let metadata = fs::symlink_metadata(directory).with_context(|| {
        format!(
            "failed to inspect attribution context directory {}",
            directory.display()
        )
    })?;
    if !metadata.file_type().is_dir() {
        anyhow::bail!("attribution context directory is not a directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if metadata.uid() != unsafe { libc::getuid() } {
            anyhow::bail!("attribution context directory has a different owner");
        }
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).with_context(|| {
            format!(
                "failed to protect attribution context directory {}",
                directory.display()
            )
        })?;
    }
    Ok(())
}

fn verify_private_file(metadata: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            anyhow::bail!("attribution context file is accessible by other users");
        }
    }
    #[cfg(not(unix))]
    let _ = metadata;
    Ok(())
}

fn prune_stale_files(directory: &Path) -> Result<()> {
    let now = unix_timestamp()?;
    for entry in fs::read_dir(directory).with_context(|| {
        format!(
            "failed to scan attribution context directory {}",
            directory.display()
        )
    })? {
        let entry = entry.context("failed to read attribution context directory entry")?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("context-") {
            continue;
        }
        let metadata = match fs::symlink_metadata(entry.path()) {
            Ok(metadata) if metadata.file_type().is_file() => metadata,
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("failed to inspect stale attribution context"),
        };
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        let file_age = SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default();
        let stale_context = read(&entry.path())
            .map(|context| is_expired(&context, now))
            .unwrap_or(false);
        if !stale_context && file_age <= STALE_FILE_RETENTION {
            continue;
        }
        remove_if_present(&entry.path())?;
        let mut claim = entry.path().into_os_string();
        claim.push(".claimed");
        remove_if_present(Path::new(&claim))?;
    }
    Ok(())
}

#[cfg(unix)]
fn current_user_id() -> u32 {
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
fn current_user_id() -> u32 {
    std::process::id()
}

#[cfg(test)]
mod tests {
    use super::super::Provider;
    use super::{
        CONTEXT_TTL, ModelIdentity, OperationContext, claim, create, read, unix_timestamp,
    };
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    #[fixture]
    fn context_dir() -> TempDir {
        tempfile::tempdir().expect("create test context directory")
    }

    fn fixture_context(model: &str, tool_use_id: &str, created_at: i64) -> OperationContext {
        OperationContext {
            schema_version: 1,
            operation_id: format!("fixture-operation-{tool_use_id}"),
            provider: Provider::Codex,
            model: ModelIdentity::Known {
                raw_slug: model.to_owned(),
            },
            tool_use_id: tool_use_id.to_owned(),
            created_at,
        }
    }

    #[rstest]
    fn context_records_keep_each_operation_model_separate(context_dir: TempDir) {
        let created_at = unix_timestamp().expect("read clock");
        let first = fixture_context("fixture-model-red", "fixture-tool-red", created_at);
        let second = fixture_context("fixture-model-blue", "fixture-tool-blue", created_at);
        let first_path = create(&first, context_dir.path()).expect("create first operation");
        let second_path = create(&second, context_dir.path()).expect("create second operation");

        let actual = (
            first_path != second_path,
            [
                read(&first_path).expect("read first operation"),
                read(&second_path).expect("read second operation"),
            ],
        );
        assert_eq!(actual, (true, [first, second]),);
    }

    #[rstest]
    fn message_only_claim_keeps_the_legacy_raw_operation_id_format(context_dir: TempDir) {
        let created_at = unix_timestamp().expect("read clock");
        let context = fixture_context("fixture-model-red", "fixture-tool-red", created_at);
        let expected_operation_id = context.operation_id().to_owned();
        let path = create(&context, context_dir.path()).expect("create operation context");
        claim(&path, created_at).expect("claim message-only context");
        let mut claim_path = path.as_os_str().to_os_string();
        claim_path.push(".claimed");
        let actual = std::fs::read_to_string(claim_path).expect("read message-only claim");

        assert_eq!(actual, expected_operation_id);
    }

    #[rstest]
    fn stale_context_cannot_be_claimed(context_dir: TempDir) {
        let context = fixture_context("fixture-model-red", "fixture-tool-red", 1_000);
        let path = create(&context, context_dir.path()).expect("create operation context");
        let now =
            context.created_at + i64::try_from(CONTEXT_TTL.as_secs()).expect("ttl fits i64") + 1;
        let error = claim(&path, now).expect_err("stale context must fail closed");

        assert_eq!(error.to_string(), "attribution context is stale");
    }

    #[rstest]
    fn creating_a_new_operation_removes_expired_context_and_claim(context_dir: TempDir) {
        let ttl = i64::try_from(CONTEXT_TTL.as_secs()).expect("ttl fits i64");
        let created_at = unix_timestamp().expect("read clock") - ttl - 1;
        let stale = fixture_context("fixture-model-red", "fixture-tool-red", created_at);
        let stale_path = create(&stale, context_dir.path()).expect("create expired context");
        claim(&stale_path, created_at + ttl).expect("claim before expiration");
        let mut claim_path = stale_path.as_os_str().to_os_string();
        claim_path.push(".claimed");
        let claim_path = std::path::PathBuf::from(claim_path);
        let fresh = fixture_context(
            "fixture-model-blue",
            "fixture-tool-blue",
            unix_timestamp().expect("read clock"),
        );
        let fresh_path = create(&fresh, context_dir.path()).expect("create fresh context");

        let actual = (
            read(&stale_path).is_err(),
            claim_path.exists(),
            read(&fresh_path).expect("read fresh context"),
        );
        assert_eq!(actual, (true, false, fresh));
    }
}
