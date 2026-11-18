use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{HookRunner, Relay, RelayKey};

const RELAY_REGISTRATIONS_FILE: &str = "web_ui_relays.state";
pub(super) const WEB_UI_URL_ENV: &str = "ARMYKNIFE_WEB_UI_URL";
pub(super) const WEB_UI_PORT_ENV: &str = "ARMYKNIFE_WEB_UI_PORT";
pub(super) const WEB_UI_EXPOSED_URL_ENV: &str = "ARMYKNIFE_WEB_UI_EXPOSED_URL";

#[derive(Debug, Error)]
pub(super) enum RelayRegistrationsLoadError {
    #[error("failed to read relay registrations from {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse relay registrations from {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub(super) struct RelayRegistration {
    pub(super) session_id: String,
    pub(super) name: String,
    pub(super) url: String,
    pub(super) port: u16,
    pub(super) exposed_url: Option<String>,
}

impl RelayRegistration {
    pub(super) fn from_relay(key: &RelayKey, relay: &Relay) -> Self {
        Self {
            session_id: key.session_id.clone(),
            name: key.name.clone(),
            url: relay.url.clone(),
            port: relay.port,
            exposed_url: relay.exposed_url.clone(),
        }
    }

    fn hook_variables(&self) -> Vec<(String, String)> {
        vec![
            (WEB_UI_URL_ENV.to_owned(), self.url.clone()),
            (WEB_UI_PORT_ENV.to_owned(), self.port.to_string()),
            (
                WEB_UI_EXPOSED_URL_ENV.to_owned(),
                self.exposed_url.clone().unwrap_or_default(),
            ),
        ]
    }
}

pub(super) async fn unexpose_registration(
    hook_runner: Arc<HookRunner>,
    registration: &RelayRegistration,
) -> Result<()> {
    super::invoke_hook(
        hook_runner,
        "web-ui-unexpose",
        registration.hook_variables(),
    )
    .await?;
    Ok(())
}

pub(super) async fn expose_and_record_registration(
    sessions_dir: &Path,
    hook_runner: Arc<HookRunner>,
    key: &RelayKey,
    relay: &mut Relay,
) {
    let should_rollback_on_save_failure = !relay.exposed;
    if !relay.exposed {
        match super::invoke_hook(
            hook_runner.clone(),
            "web-ui-expose",
            vec![
                (WEB_UI_URL_ENV.to_owned(), relay.url.clone()),
                (WEB_UI_PORT_ENV.to_owned(), relay.port.to_string()),
            ],
        )
        .await
        {
            Ok(exposed_url) => {
                relay.exposed = true;
                relay.exposed_url = exposed_url.and_then(super::valid_exposed_url);
            }
            Err(error) => {
                tracing::warn!(
                    session = %key.session_id,
                    web_ui = %key.name,
                    %error,
                    "web-ui-expose hook failed"
                );
                return;
            }
        }
    }

    if relay.registration_saved {
        return;
    }

    let registration = RelayRegistration::from_relay(key, relay);
    if let Err(error) = add_relay_registration(sessions_dir, &registration) {
        tracing::warn!(
            session = %key.session_id,
            web_ui = %key.name,
            %error,
            "failed to save web UI relay registration"
        );
        if should_rollback_on_save_failure {
            if let Err(error) = unexpose_registration(hook_runner, &registration).await {
                tracing::warn!(
                    session = %key.session_id,
                    web_ui = %key.name,
                    %error,
                    "failed to undo unrecorded web UI relay registration"
                );
            } else {
                relay.exposed = false;
                relay.exposed_url = None;
            }
        }
        return;
    }
    relay.registration_saved = true;
}

pub(super) fn load_relay_registrations(
    sessions_dir: &Path,
) -> Result<Vec<RelayRegistration>, RelayRegistrationsLoadError> {
    let path = sessions_dir.join(RELAY_REGISTRATIONS_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read(&path).map_err(|source| RelayRegistrationsLoadError::Read {
        path: path.clone(),
        source,
    })?;
    serde_json::from_slice(&contents)
        .map_err(|source| RelayRegistrationsLoadError::Parse { path, source })
}

pub(super) fn save_relay_registrations(
    sessions_dir: &Path,
    registrations: &[RelayRegistration],
) -> Result<()> {
    fs::create_dir_all(sessions_dir)?;
    let path = sessions_dir.join(RELAY_REGISTRATIONS_FILE);
    let temp_path = path.with_extension("tmp");
    let contents = serde_json::to_vec_pretty(registrations)?;
    fs::write(&temp_path, contents).with_context(|| {
        format!(
            "failed to write relay registrations to {}",
            temp_path.display()
        )
    })?;
    fs::rename(&temp_path, &path)
        .with_context(|| format!("failed to save relay registrations to {}", path.display()))?;
    Ok(())
}

pub(super) fn remove_relay_registration(
    sessions_dir: &Path,
    registration: &RelayRegistration,
) -> Result<()> {
    let mut registrations = load_relay_registrations(sessions_dir)?;
    let previous_len = registrations.len();
    registrations.retain(|stored| stored != registration);
    if registrations.len() != previous_len {
        save_relay_registrations(sessions_dir, &registrations)?;
    }
    Ok(())
}

pub(super) fn remove_and_save_relay_registration(
    sessions_dir: &Path,
    registrations: &mut Vec<RelayRegistration>,
    registration: &RelayRegistration,
) -> Result<()> {
    registrations.retain(|stored| stored != registration);
    save_relay_registrations(sessions_dir, registrations)
}

pub(super) fn add_relay_registration(
    sessions_dir: &Path,
    registration: &RelayRegistration,
) -> Result<()> {
    let mut registrations = match load_relay_registrations(sessions_dir) {
        Ok(registrations) => registrations,
        Err(error @ RelayRegistrationsLoadError::Parse { .. }) => {
            tracing::warn!(%error, "replacing unreadable web UI relay registrations");
            Vec::new()
        }
        Err(error) => return Err(error.into()),
    };
    registrations.retain(|stored| stored != registration);
    registrations.push(registration.clone());
    save_relay_registrations(sessions_dir, &registrations)
}
