use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::{HookRunner, Relay, RelayKey};

const RELAY_REGISTRATIONS_FILE: &str = "web_ui_relays.state";
pub(super) const WEB_UI_URL_ENV: &str = "ARMYKNIFE_WEB_UI_URL";
pub(super) const WEB_UI_PORT_ENV: &str = "ARMYKNIFE_WEB_UI_PORT";
pub(super) const WEB_UI_EXPOSED_URL_ENV: &str = "ARMYKNIFE_WEB_UI_EXPOSED_URL";

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

pub(super) fn load_relay_registrations(sessions_dir: &Path) -> Result<Vec<RelayRegistration>> {
    let path = sessions_dir.join(RELAY_REGISTRATIONS_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read(&path)
        .with_context(|| format!("failed to read relay registrations from {}", path.display()))?;
    serde_json::from_slice(&contents).with_context(|| {
        format!(
            "failed to parse relay registrations from {}",
            path.display()
        )
    })
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

pub(super) fn add_relay_registration(
    sessions_dir: &Path,
    registration: &RelayRegistration,
) -> Result<()> {
    let mut registrations = match load_relay_registrations(sessions_dir) {
        Ok(registrations) => registrations,
        Err(error) => {
            tracing::warn!(%error, "replacing unreadable web UI relay registrations");
            Vec::new()
        }
    };
    registrations.retain(|stored| stored != registration);
    registrations.push(registration.clone());
    save_relay_registrations(sessions_dir, &registrations)
}
