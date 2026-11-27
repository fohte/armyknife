use anyhow::Result;

use super::store;

pub(super) fn run() -> Result<()> {
    store::cleanup_stale_sessions()?;
    Ok(())
}
