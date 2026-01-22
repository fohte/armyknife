use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::{Context, bail, ensure};
use clap::Args;

use crate::shared::human_in_the_loop::{LaunchOptions, launch_wezterm};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct DraftArgs {
    /// File path to open in editor for review
    pub path: PathBuf,
}

pub fn run(args: &DraftArgs) -> anyhow::Result<()> {
    ensure!(
        args.path.exists(),
        "File not found: {}",
        args.path.display()
    );

    let path = args
        .path
        .canonicalize()
        .with_context(|| format!("Failed to resolve path: {}", args.path.display()))?;

    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());

    let window_title = format!("Draft: {}", file_name);

    let options = LaunchOptions {
        window_title: window_title.clone(),
        ..Default::default()
    };

    // Build nvim args with titlestring
    let escaped_title = window_title.replace('\'', "''");
    let nvim_args: Vec<OsString> = vec![
        "-c".into(),
        format!("let &titlestring = '{escaped_title}'").into(),
        path.as_os_str().to_os_string(),
    ];

    let status = launch_wezterm(&options, "nvim", &nvim_args)
        .context("Failed to launch WezTerm with Neovim")?;

    if !status.success() {
        bail!("WezTerm exited with status: {}", status);
    }

    println!("Opened draft in editor: {}", path.display());

    Ok(())
}
