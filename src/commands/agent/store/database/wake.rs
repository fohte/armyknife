use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

use super::open_connection;

const WAKE_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const WAKE_RETRY_DELAY: Duration = Duration::from_millis(25);
const WAKE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const WAKE_LEASE_SECS: i64 = 60;

pub(super) struct WakeLock {
    connection: Connection,
    key: String,
    owner: String,
    stop_heartbeat: Sender<()>,
    heartbeat: Option<JoinHandle<()>>,
}

pub(super) fn acquire(sessions_dir: &Path, session_id: &str) -> Result<WakeLock> {
    let mut connection = open_connection(sessions_dir)?;
    let key = format!("wake-lock:{session_id}");
    let started_at = Instant::now();

    loop {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT value FROM session_store_meta WHERE key = ?1",
                [&key],
                |row| row.get(0),
            )
            .optional()?;
        let now = chrono::Utc::now().timestamp();
        if let Some(value) = existing {
            let expired = value
                .rsplit_once('|')
                .and_then(|(_, expires_at)| expires_at.parse::<i64>().ok())
                .is_none_or(|expires_at| expires_at <= now);
            if expired {
                transaction.execute(
                    "DELETE FROM session_store_meta WHERE key = ?1 AND value = ?2",
                    params![key, value],
                )?;
            }
        }

        let still_claimed: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM session_store_meta WHERE key = ?1)",
            [&key],
            |row| row.get(0),
        )?;
        if !still_claimed {
            let owner = Uuid::new_v4().to_string();
            let value = format!("{owner}|{}", now + WAKE_LEASE_SECS);
            transaction.execute(
                "INSERT INTO session_store_meta (key, value) VALUES (?1, ?2)",
                params![key, value],
            )?;
            transaction.commit()?;

            let (stop_heartbeat, receiver) = mpsc::channel();
            let database_path = sessions_dir.join("sessions.db");
            let heartbeat_key = key.clone();
            let heartbeat_owner = owner.clone();
            let heartbeat = thread::spawn(move || {
                refresh_wake_lease(&database_path, &heartbeat_key, &heartbeat_owner, receiver)
            });
            return Ok(WakeLock {
                connection,
                key,
                owner,
                stop_heartbeat,
                heartbeat: Some(heartbeat),
            });
        }
        transaction.commit()?;

        if started_at.elapsed() >= WAKE_WAIT_TIMEOUT {
            anyhow::bail!("timed out waiting for session wake lock: {session_id}");
        }
        thread::sleep(WAKE_RETRY_DELAY);
    }
}

impl Drop for WakeLock {
    fn drop(&mut self) {
        let _ = self.stop_heartbeat.send(());
        if let Some(heartbeat) = self.heartbeat.take() {
            let _ = heartbeat.join();
        }
        let result = (|| -> Result<()> {
            let transaction = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute(
                "DELETE FROM session_store_meta
                 WHERE key = ?1 AND substr(value, 1, length(?2)) = ?2",
                params![self.key, format!("{}|", self.owner)],
            )?;
            transaction.commit()?;
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(
                target: "armyknife::commands::agent::store",
                event = "agent.store.wake_lock_release_failed",
                lock = %self.key,
                error = %error,
            );
        }
    }
}

fn refresh_wake_lease(database_path: &Path, key: &str, owner: &str, receiver: mpsc::Receiver<()>) {
    let prefix = format!("{owner}|");
    loop {
        match receiver.recv_timeout(WAKE_HEARTBEAT_INTERVAL) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }

        let result = (|| -> Result<()> {
            let mut connection = Connection::open(database_path)?;
            connection.busy_timeout(Duration::from_secs(5))?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let expires_at = chrono::Utc::now().timestamp() + WAKE_LEASE_SECS;
            transaction.execute(
                "UPDATE session_store_meta SET value = ?1
                 WHERE key = ?2 AND substr(value, 1, length(?3)) = ?3",
                params![format!("{owner}|{expires_at}"), key, prefix],
            )?;
            transaction.commit()?;
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(
                target: "armyknife::commands::agent::store",
                event = "agent.store.wake_lease_refresh_failed",
                lock = key,
                error = %error,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn same_session_wake_claim_waits_until_current_owner_releases_it() {
        let temp_dir = tempfile::tempdir().expect("temporary directory should be created");
        let sessions_dir = temp_dir.path().join("sessions");
        let lock = acquire(&sessions_dir, "wake-target").expect("first claim should succeed");
        let worker_dir = sessions_dir.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            started_tx.send(()).expect("worker should signal its start");
            let result = acquire(&worker_dir, "wake-target").is_ok();
            acquired_tx
                .send(result)
                .expect("worker should report its claim");
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker should start");

        let acquired_while_owned = acquired_rx.recv_timeout(Duration::from_millis(100));
        drop(lock);
        let acquired_after_release = acquired_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("waiting claim should succeed after release");
        worker.join().expect("worker should complete");

        assert_eq!(
            (acquired_while_owned.is_ok(), acquired_after_release),
            (false, true),
        );
    }
}
