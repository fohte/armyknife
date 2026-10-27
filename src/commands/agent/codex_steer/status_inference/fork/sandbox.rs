use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::path_to_string;

pub(super) fn write_srt_settings(inference_dir: &Path, relay_port: u16) -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is unavailable for Codex status sandbox settings")?;
    let settings = srt_settings(inference_dir, relay_port, &home)?;
    let path = inference_dir.join("srt-settings.json");
    std::fs::write(&path, serde_json::to_vec(&settings)?)
        .context("write Codex status sandbox settings")?;
    Ok(path)
}

fn srt_settings(inference_dir: &Path, relay_port: u16, home: &Path) -> Result<Value> {
    let inference_dir = path_to_string(inference_dir)?;
    // srt always appends these compatibility paths to filesystem.allowWrite.
    let deny_write = [
        Path::new("/dev/stdout").to_path_buf(),
        Path::new("/dev/stderr").to_path_buf(),
        Path::new("/dev/null").to_path_buf(),
        Path::new("/dev/tty").to_path_buf(),
        Path::new("/dev/dtracehelper").to_path_buf(),
        Path::new("/dev/autofs_nowait").to_path_buf(),
        Path::new("/tmp/claude").to_path_buf(),
        Path::new("/private/tmp/claude").to_path_buf(),
        home.join(".npm/_logs"),
        home.join(".claude/debug"),
    ]
    .into_iter()
    .map(|path| path_to_string(&path))
    .collect::<Result<Vec<_>>>()?;
    Ok(json!({
        "network": {
            "allowedDomains": [],
            "deniedDomains": ["*"],
            "strictAllowlist": true,
            "allowLocalBinding": false,
            "httpProxyPort": relay_port,
            "socksProxyPort": relay_port,
        },
        "filesystem": {
            "denyRead": [],
            "allowWrite": [inference_dir],
            "denyWrite": deny_write,
        },
    }))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rstest::rstest;
    use serde_json::{Value, json};

    use super::srt_settings;

    #[rstest]
    #[case::mac_tmp_fixture_path(
        Path::new("/private/tmp/status-worker-fixture"),
        Path::new("/Users/example"),
        43817,
        json!({
            "network": {
                "allowedDomains": [],
                "deniedDomains": ["*"],
                "strictAllowlist": true,
                "allowLocalBinding": false,
                "httpProxyPort": 43817,
                "socksProxyPort": 43817,
            },
            "filesystem": {
                "denyRead": [],
                "allowWrite": ["/private/tmp/status-worker-fixture"],
                "denyWrite": [
                    "/dev/stdout",
                    "/dev/stderr",
                    "/dev/null",
                    "/dev/tty",
                    "/dev/dtracehelper",
                    "/dev/autofs_nowait",
                    "/tmp/claude",
                    "/private/tmp/claude",
                    "/Users/example/.npm/_logs",
                    "/Users/example/.claude/debug",
                ],
            },
        }),
    )]
    fn srt_settings_limit_network_and_file_writes(
        #[case] inference_dir: &Path,
        #[case] home: &Path,
        #[case] relay_port: u16,
        #[case] expected: Value,
    ) {
        let actual = srt_settings(inference_dir, relay_port, home)
            .expect("fixture sandbox settings should be valid");
        assert_eq!(actual, expected);
    }
}
