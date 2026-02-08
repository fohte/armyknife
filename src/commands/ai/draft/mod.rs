use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::shared::config::load_config;
use crate::shared::human_in_the_loop::{
    DocumentSchema, ReviewHandler, complete_review, start_review,
};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct DraftArgs {
    /// File path to open in editor for review
    pub path: PathBuf,

    /// Window title for WezTerm (defaults to "Draft: <filename>")
    #[arg(long)]
    pub title: Option<String>,

    /// Internal: tmux pane to restore after editing
    #[arg(long, hide = true)]
    pub tmux_target: Option<String>,

    /// Internal: run in edit-complete mode (called by WezTerm)
    #[arg(long, hide = true)]
    pub complete: bool,
}

/// Permissive schema for simple file editing that accepts any frontmatter.
///
/// Uses `#[serde(flatten)]` with a catch-all map to ignore unknown fields,
/// allowing files with arbitrary YAML frontmatter to be opened without errors.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmptySchema {
    #[serde(flatten)]
    _extra: std::collections::HashMap<String, serde_yaml::Value>,
}

impl DocumentSchema for EmptySchema {
    fn is_approved(&self) -> bool {
        false
    }
}

/// Handler for simple file editing sessions.
pub struct DraftHandler;

impl ReviewHandler<EmptySchema> for DraftHandler {
    fn build_complete_args(
        &self,
        document_path: &Path,
        tmux_target: Option<&str>,
        window_title: &str,
    ) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec![
            "ai".into(),
            "draft".into(),
            "--complete".into(),
            document_path.as_os_str().to_os_string(),
        ];

        if let Some(target) = tmux_target {
            args.push("--tmux-target".into());
            args.push(target.into());
        }

        args.push("--title".into());
        args.push(window_title.into());

        args
    }

    // Uses default on_review_complete (does nothing)
}

pub fn run(args: &DraftArgs) -> anyhow::Result<()> {
    if args.complete {
        run_complete(args)
    } else {
        run_edit(args)
    }
}

fn run_edit(args: &DraftArgs) -> anyhow::Result<()> {
    ensure!(
        args.path.exists(),
        "File not found: {}",
        args.path.display()
    );

    let path = args
        .path
        .canonicalize()
        .with_context(|| format!("Failed to resolve path: {}", args.path.display()))?;

    let config = load_config()?;
    let window_title = args
        .title
        .clone()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| {
            let file_name = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".to_string());
            format!("Draft: {}", file_name)
        });

    start_review::<EmptySchema, _>(&path, &window_title, &DraftHandler, &config.editor)?;

    println!("Opened draft in editor: {}", path.display());

    Ok(())
}

fn run_complete(args: &DraftArgs) -> anyhow::Result<()> {
    let config = load_config()?;

    complete_review::<EmptySchema, _>(
        &args.path,
        args.tmux_target.as_deref(),
        args.title.as_deref(),
        &DraftHandler,
        &config.editor,
    )?;

    Ok(())
}
