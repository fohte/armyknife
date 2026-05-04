use clap::Args;
use indoc::formatdoc;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::common::{DraftFile, Frontmatter, RepoInfo};
use super::hooks::{HookContext, HookRunner, PRE_PR_REVIEW_HOOK, default_runner, run_pr_hook};
use crate::shared::config::load_config;
use crate::shared::human_in_the_loop::{
    Document, DocumentSchema, FifoSignalGuard, Result as HilResult, ReviewHandler, complete_review,
    start_review,
};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReviewArgs {
    /// Path to the draft file (auto-detected if not specified)
    pub filepath: Option<PathBuf>,
}

/// Internal command to complete the review process after Neovim exits.
/// This is called by WezTerm, not directly by users.
#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReviewCompleteArgs {
    /// Path to the draft file
    pub filepath: PathBuf,

    /// tmux session to restore after review
    #[arg(long)]
    pub tmux_target: Option<String>,

    /// Window title for Neovim
    #[arg(long)]
    pub window_title: Option<String>,

    /// Internal: FIFO path to signal completion to the waiting start_review process
    #[arg(long, hide = true)]
    pub done_fifo: Option<PathBuf>,
}

/// Handler for PR draft review sessions.
pub struct PrDraftReviewHandler;

impl ReviewHandler<Frontmatter> for PrDraftReviewHandler {
    fn build_complete_args(
        &self,
        document_path: &Path,
        tmux_target: Option<&str>,
        window_title: &str,
    ) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec![
            "ai".into(),
            "pr-draft".into(),
            "review-complete".into(),
            document_path.as_os_str().to_os_string(),
        ];

        if let Some(target) = tmux_target {
            args.push("--tmux-target".into());
            args.push(target.into());
        }

        args.push("--window-title".into());
        args.push(window_title.into());

        args
    }

    fn on_review_complete(&self, document: &Document<Frontmatter>) -> HilResult<()> {
        if document.frontmatter.is_approved() {
            document.save_approval()?;
            println!(
                "{}",
                formatdoc! {"
                    PR approved. Run the following command to create the PR:

                        a ai pr-draft submit
                "}
            );
        } else {
            document.remove_approval()?;
            println!("PR not approved. Set 'steps.submit: true' and save to approve.");
        }

        Ok(())
    }
}

pub fn run(args: &ReviewArgs) -> anyhow::Result<()> {
    run_impl(args, default_runner())
}

fn run_impl(args: &ReviewArgs, run_hook: HookRunner<'_>) -> anyhow::Result<()> {
    let (draft_path, owner, repo, branch) = match &args.filepath {
        Some(path) => {
            let (owner, repo, branch) = DraftFile::parse_path(path).ok_or_else(|| {
                let display = path.display();
                anyhow::anyhow!("Invalid draft path: {display}")
            })?;
            (path.clone(), owner, repo, branch)
        }
        None => {
            let repo_info = RepoInfo::from_git_only()?;
            let path = DraftFile::path_for(&repo_info);
            (path, repo_info.owner, repo_info.repo, repo_info.branch)
        }
    };

    let config = load_config()?;
    let window_title = format!("PR: {owner}/{repo} @ {branch}");

    // Read frontmatter before review to detect changes
    let before = Document::<Frontmatter>::from_path(draft_path.clone())?;

    // Fire pre-pr-review before opening the editor so user-defined lints
    // surface here (where the user is about to edit the file) rather than
    // later at submit time, which would force a round-trip back into review.
    let draft_for_hook = DraftFile::from_path(draft_path.clone())?;
    let context = HookContext {
        owner: &owner,
        repo: &repo,
        head_branch: &branch,
        base_branch: "",
        update_pr_number: None,
    };
    run_pr_hook(PRE_PR_REVIEW_HOOK, &draft_for_hook, &context, run_hook)?;

    use crate::shared::human_in_the_loop::exit_code;

    let document = exit_code::exit_on_terminal_launch_failure(start_review::<Frontmatter, _>(
        &draft_path,
        &window_title,
        &PrDraftReviewHandler,
        &config.editor,
    ))?;

    // If the editor was already open, exit with a distinct code so callers
    // can distinguish "already open" from "user did not approve".
    if document.is_none() {
        std::process::exit(exit_code::ALREADY_OPEN);
    }

    let steps_changed = document
        .as_ref()
        .is_some_and(|doc| doc.frontmatter.steps != before.frontmatter.steps);
    if !steps_changed {
        eprintln!("No steps changed. Review not approved.");
        std::process::exit(exit_code::NOT_APPROVED);
    }

    eprintln!("Review completed. Steps updated.");

    Ok(())
}

pub fn run_complete(args: &ReviewCompleteArgs) -> anyhow::Result<()> {
    // Create FIFO guard first to ensure signaling even if load_config fails
    let _fifo_guard = args.done_fifo.as_deref().map(FifoSignalGuard::new);

    let config = load_config()?;

    complete_review::<Frontmatter, _>(
        &args.filepath,
        args.tmux_target.as_deref(),
        args.window_title.as_deref(),
        &PrDraftReviewHandler,
        &config.editor,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::env_var::EnvVars;
    use indoc::indoc;
    use std::fs;
    use std::sync::Mutex;

    #[cfg(unix)]
    #[test]
    fn build_review_args_should_roundtrip_non_utf_paths() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let mut filename = OsString::from_vec(vec![b'b', b'r', b'a', b'n', b'c', b'h', 0xff]);
        filename.push(".md");
        let draft_path = DraftFile::draft_dir()
            .join("owner")
            .join("repo")
            .join(std::path::PathBuf::from(filename));

        let handler = PrDraftReviewHandler;
        let args = handler.build_complete_args(&draft_path, Some("sess:1.0"), "Test Title");
        let restored = std::path::Path::new(&args[3]);
        assert_eq!(
            restored.as_os_str(),
            draft_path.as_os_str(),
            "Path should survive argument building without loss"
        );
    }

    /// Write a minimal draft file under [`DraftFile::draft_dir`] so `run_impl`
    /// can locate it via the `--filepath` argument.
    fn write_draft(owner: &str, repo: &str, branch: &str, body: &str) -> PathBuf {
        let repo_info = RepoInfo {
            owner: owner.to_string(),
            repo: repo.to_string(),
            branch: branch.to_string(),
            is_private: true,
        };
        let path = DraftFile::path_for(&repo_info);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        let content = format!(
            indoc! {r#"
                ---
                title: "Review hook test"
                steps:
                  submit: false
                ---
                {body}
            "#},
            body = body,
        );
        fs::write(&path, content).expect("write draft");
        path
    }

    #[test]
    fn run_impl_invokes_pre_pr_review_with_body_file() {
        let draft_path = write_draft(
            "owner",
            "repo_review_hook",
            "feature/review",
            "Body referencing #999",
        );

        let captured = Mutex::new(None);
        let hook = |name: &str, env: &[(&str, &str)]| -> anyhow::Result<()> {
            let body_path = env
                .iter()
                .find(|(k, _)| *k == EnvVars::pr_body_file_name())
                .map(|(_, v)| (*v).to_string())
                .expect("body file env var present");
            let body_contents = fs::read_to_string(&body_path).expect("body file readable");
            *captured.lock().expect("capture mutex") = Some((name.to_string(), body_contents));
            // Abort before reaching start_review (which would try to launch a
            // terminal in the test environment).
            anyhow::bail!("stop after hook")
        };

        let args = ReviewArgs {
            filepath: Some(draft_path.clone()),
        };
        let err = run_impl(&args, &hook).expect_err("hook bail must abort run_impl");
        assert_eq!(err.to_string(), "stop after hook");

        let (name, body_contents) = captured
            .lock()
            .expect("capture mutex")
            .clone()
            .expect("hook must have been invoked");
        assert_eq!(name, PRE_PR_REVIEW_HOOK);
        assert_eq!(body_contents, "Body referencing #999\n");

        let _ = fs::remove_file(&draft_path);
    }

    #[test]
    fn run_impl_returns_hook_error_to_caller() {
        let draft_path = write_draft("owner", "repo_review_hook_err", "feature/err", "body");

        let hook =
            |_name: &str, _env: &[(&str, &str)]| -> anyhow::Result<()> { anyhow::bail!("nope") };

        let args = ReviewArgs {
            filepath: Some(draft_path.clone()),
        };
        let err = run_impl(&args, &hook).expect_err("hook failure must propagate");
        assert_eq!(err.to_string(), "nope");

        let _ = fs::remove_file(&draft_path);
    }
}
