use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::Result;
use rusqlite::Connection;

use super::{migration, migrations};

#[derive(Default)]
struct InitializationState {
    migrations_complete: bool,
    legacy_data_complete: bool,
}

static INITIALIZATION_STATE: OnceLock<Mutex<HashMap<PathBuf, InitializationState>>> =
    OnceLock::new();

pub(super) fn run(
    connection: &mut Connection,
    sessions_dir: &Path,
    import_legacy_data: bool,
) -> Result<()> {
    let mut states = INITIALIZATION_STATE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let state = states.entry(sessions_dir.to_path_buf()).or_default();

    if !state.migrations_complete {
        migrations::migrate_if_needed(connection)?;
        state.migrations_complete = true;
    }

    if import_legacy_data && !state.legacy_data_complete {
        let import_succeeded = match migration::import_legacy_json(connection, sessions_dir) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(
                    target: "armyknife::commands::agent::store",
                    event = "agent.store.legacy_import_failed",
                    error = %error,
                );
                false
            }
        };
        let lock_cleanup_succeeded = migration::remove_legacy_lock_files(sessions_dir);
        let legacy_cleanup_succeeded =
            migration::cleanup_expired_legacy_json(connection, sessions_dir)?;
        state.legacy_data_complete =
            import_succeeded && lock_cleanup_succeeded && legacy_cleanup_succeeded;
    }

    Ok(())
}
