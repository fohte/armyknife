use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use serde_json::Value;

use crate::commands::agent::codex_steer::Client;
use crate::shared::command::{self, find_command_path};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const RETRY_INTERVAL: Duration = Duration::from_millis(50);

struct AppServerProcess {
    child: Child,
    socket_path: PathBuf,
}

impl Drop for AppServerProcess {
    fn drop(&mut self) {
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

pub(super) fn list_models() -> anyhow::Result<Value> {
    let codex = find_command_path("codex").context("Could not find 'codex' command in PATH")?;
    let socket_path = std::env::temp_dir().join(format!("a-{}.sock", uuid::Uuid::new_v4()));
    let listen_url = socket_listen_url(&socket_path);
    let child = command::new(codex)
        .args(["app-server", "--listen"])
        .arg(listen_url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to start Codex app-server for model discovery")?;
    let mut server = AppServerProcess { child, socket_path };
    let stream = wait_for_socket(&mut server)?;

    let mut client = Client::connect_stream(stream)?;
    client.list_models()
}

fn socket_listen_url(socket_path: &Path) -> String {
    format!("unix://{}", socket_path.display())
}

fn wait_for_socket(server: &mut AppServerProcess) -> anyhow::Result<UnixStream> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        match UnixStream::connect(&server.socket_path) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                if let Some(status) = server
                    .child
                    .try_wait()
                    .context("Failed to check Codex app-server startup")?
                {
                    bail!("Codex app-server exited before opening its socket ({status}): {error}");
                }
                if Instant::now() >= deadline {
                    bail!("Timed out waiting for Codex app-server socket: {error}");
                }
                thread::sleep(RETRY_INTERVAL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_local_unix_socket_listen_url() {
        assert_eq!(
            socket_listen_url(Path::new("/tmp/fixture-codex.sock")),
            "unix:///tmp/fixture-codex.sock",
        );
    }
}
