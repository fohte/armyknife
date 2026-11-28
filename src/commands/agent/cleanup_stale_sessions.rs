use anyhow::Result;

use super::store;

pub(super) fn run() -> Result<()> {
    if let Err(error) = store::cleanup_stale_sessions() {
        tracing::warn!(
            event = "agent.cleanup_stale_sessions.failed",
            error = %error,
            "failed to clean up stale agent sessions"
        );
        return Err(error);
    }
    Ok(())
}
