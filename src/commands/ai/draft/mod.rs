use std::path::PathBuf;

use anyhow::{Context, ensure};
use clap::Args;

use crate::shared::human_in_the_loop::{
    SimpleEditCompleteArgs, complete_simple_edit, start_simple_edit,
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

pub fn run(args: &DraftArgs) -> anyhow::Result<()> {
    if args.complete {
        run_edit_complete(args)
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

    let window_title = args.title.clone().unwrap_or_else(|| {
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        format!("Draft: {}", file_name)
    });

    start_simple_edit(&path, &window_title, &["ai", "draft", "--complete"])?;

    println!("Opened draft in editor: {}", path.display());

    Ok(())
}

fn run_edit_complete(args: &DraftArgs) -> anyhow::Result<()> {
    let complete_args = SimpleEditCompleteArgs {
        tmux_target: args.tmux_target.clone(),
        window_title: args.title.clone(),
    };

    complete_simple_edit(&args.path, &complete_args)?;

    Ok(())
}
