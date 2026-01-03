use clap::Args;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::common::{DraftFile, PrDraftError, RepoInfo, Result};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReviewArgs {
    /// Path to the draft file (auto-detected if not specified)
    pub filepath: Option<PathBuf>,
}

pub fn run(args: &ReviewArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let draft_path = match &args.filepath {
        Some(path) => path.clone(),
        None => {
            let repo_info = RepoInfo::from_current_dir()?;
            DraftFile::path_for(&repo_info)
        }
    };

    if !draft_path.exists() {
        return Err(Box::new(PrDraftError::FileNotFound(draft_path)));
    }

    // Check for existing lock
    let lock_path = DraftFile::lock_path(&draft_path);
    if lock_path.exists() {
        eprintln!("Skipped: Editor is already open for this file.");
        return Ok(());
    }

    let repo_info = RepoInfo::from_current_dir()?;
    let window_title = format!(
        "PR: {}/{} @ {}",
        repo_info.owner, repo_info.repo, repo_info.branch
    );

    // Save tmux session info for later restoration
    let tmux_info = get_tmux_info();

    // Create nvim wrapper script
    let wrapper_script = create_nvim_wrapper(&draft_path, &window_title, tmux_info.as_ref())?;

    // Create lock file
    fs::write(&lock_path, "")?;

    // Launch WezTerm with nvim
    let status = Command::new("open")
        .args([
            "-n",
            "-a",
            "WezTerm",
            "--args",
            "--config",
            "window_decorations=\"TITLE | RESIZE\"",
            "start",
            "--",
            "bash",
            wrapper_script.to_str().unwrap(),
        ])
        .status();

    if let Err(e) = status {
        // Cleanup lock on error
        let _ = fs::remove_file(&lock_path);
        return Err(Box::new(PrDraftError::CommandFailed(format!(
            "Failed to launch WezTerm: {}",
            e
        ))));
    }

    Ok(())
}

#[derive(Debug)]
struct TmuxInfo {
    session: String,
    window: String,
    pane: String,
}

fn get_tmux_info() -> Option<TmuxInfo> {
    if std::env::var("TMUX").is_err() {
        return None;
    }

    let session = Command::new("tmux")
        .args(["display-message", "-p", "#{session_name}"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })?;

    let window = Command::new("tmux")
        .args(["display-message", "-p", "#{window_index}"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })?;

    let pane = Command::new("tmux")
        .args(["display-message", "-p", "#{pane_index}"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
            } else {
                None
            }
        })?;

    Some(TmuxInfo {
        session,
        window,
        pane,
    })
}

fn create_nvim_wrapper(
    draft_path: &Path,
    window_title: &str,
    tmux_info: Option<&TmuxInfo>,
) -> Result<PathBuf> {
    let wrapper = tempfile::Builder::new()
        .prefix("nvim-wrapper-")
        .suffix(".sh")
        .tempfile()
        .map_err(PrDraftError::Io)?;

    let (_, wrapper_path) = wrapper.keep().map_err(|e| PrDraftError::Io(e.error))?;

    let draft_path_str = draft_path.display();
    let lock_path = DraftFile::lock_path(draft_path);
    let lock_path_str = lock_path.display();
    let approve_path = DraftFile::approve_path(draft_path);
    let approve_path_str = approve_path.display();

    let tmux_restore = if let Some(info) = tmux_info {
        format!(
            r#"
# Restore tmux session
if command -v tmux &>/dev/null && [ -n "$TMUX_PANE" ]; then
    tmux switch-client -t "{}:{}.{}"
fi
"#,
            info.session, info.window, info.pane
        )
    } else {
        String::new()
    };

    let script = format!(
        r#"#!/bin/bash
set -e

DRAFT_PATH="{draft_path_str}"
LOCK_PATH="{lock_path_str}"
APPROVE_PATH="{approve_path_str}"
WINDOW_TITLE="{window_title}"

cleanup() {{
    rm -f "$LOCK_PATH"
    rm -f "$0"  # Remove this wrapper script
    {tmux_restore}
}}

trap cleanup EXIT

# Set window title via nvim
nvim -c "set titlestring=$WINDOW_TITLE" -c "set title" "$DRAFT_PATH"

# After nvim exits, check if submit was approved
if command -v yq &>/dev/null; then
    SUBMIT_VALUE=$(sed -n '/^---$/,/^---$/p' "$DRAFT_PATH" | yq -r '.steps.submit // false')
else
    # Fallback: simple grep-based check
    SUBMIT_VALUE=$(sed -n '/^---$/,/^---$/{{/submit:/p}}' "$DRAFT_PATH" | grep -o 'true\|false' | head -1)
fi

if [ "$SUBMIT_VALUE" = "true" ]; then
    # Compute hash and save to approve file
    shasum -a 256 "$DRAFT_PATH" | cut -d' ' -f1 > "$APPROVE_PATH"
    echo "PR approved. Run 'a ai pr-draft submit' to create the PR."
else
    rm -f "$APPROVE_PATH"
    echo "PR not approved. Set 'steps.submit: true' and save to approve."
fi
"#
    );

    fs::write(&wrapper_path, script)?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&wrapper_path)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&wrapper_path, perms)?;
    }

    Ok(wrapper_path)
}
