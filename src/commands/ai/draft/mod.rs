use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::shared::config::load_config;
use crate::shared::human_in_the_loop::{
    Document, DocumentSchema, FifoSignalGuard, Result as HilResult, ReviewHandler, complete_review,
    start_review,
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

    /// Internal: FIFO path to signal completion to the waiting start_review process
    #[arg(long, hide = true)]
    pub done_fifo: Option<PathBuf>,
}

/// Permissive schema for file editing that checks `submit` field for approval.
///
/// Uses `#[serde(flatten)]` with a catch-all map to ignore unknown fields,
/// allowing files with arbitrary YAML frontmatter to be opened without errors.
/// The `submit` field is used to indicate approval when present and set to `true`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EmptySchema {
    #[serde(default)]
    pub submit: bool,
    #[serde(flatten)]
    _extra: std::collections::HashMap<String, serde_yaml::Value>,
}

impl DocumentSchema for EmptySchema {
    fn is_approved(&self) -> bool {
        self.submit
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

    fn on_review_complete(&self, document: &Document<EmptySchema>) -> HilResult<()> {
        if document.frontmatter.is_approved() {
            document.save_approval()?;
            println!("Approved.");
        } else {
            document.remove_approval()?;
            println!("Not approved. Set 'submit: true' in the frontmatter to approve.");
        }
        Ok(())
    }
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

    Ok(())
}

fn run_complete(args: &DraftArgs) -> anyhow::Result<()> {
    // Create FIFO guard first to ensure signaling even if load_config fails
    let _fifo_guard = args.done_fifo.as_deref().map(FifoSignalGuard::new);

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
