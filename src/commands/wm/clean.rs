use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use chrono::Utc;
use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};

use super::error::{Result, WmError};
use super::worktree::{LinkedWorktree, get_main_repo, list_linked_worktrees};
use crate::commands::cc::auto_pause::parse_duration;
use crate::commands::cc::store::{list_sessions, sweep_pending_bg_tasks_in_all_sessions};
use crate::commands::cc::types::Session;
use crate::infra::git::GitRepo;
use crate::infra::git::MergeStatus;
use crate::infra::git::fetch_with_prune;
use crate::infra::git::get_merge_status_for_repo;
use crate::infra::git::{github_owner_and_repo, merge_status_from_git, merge_status_from_pr};
use crate::infra::github::{BranchPrQuery, GitHubClient};
use crate::shared::active_session::{
    ActivityProbe, NoActivityProbe, TmuxActivityProbe, contains_active_session,
};
use crate::shared::config::load_config;
use crate::shared::repos_root::{discover_repos_with_worktrees, resolve_repos_root};
use crate::shared::table::{color, pad_or_truncate};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CleanArgs {
    /// Show what would be deleted without actually deleting
    #[arg(short = 'n', long)]
    pub dry_run: bool,

    /// Clean worktrees across all repositories under repos_root
    #[arg(long)]
    pub all: bool,

    /// Delete even worktrees that currently host an active Claude Code session.
    /// Without this flag, such worktrees are kept regardless of merge status.
    #[arg(long)]
    pub force: bool,
}

/// Worktree with merge status for clean command.
struct CleanWorktreeInfo {
    wt: LinkedWorktree,
    status: MergeStatus,
    /// Repository name (relative path from repos_root). Only set in --all mode.
    repo_name: Option<String>,
    /// True when this worktree contains an active Claude Code session and was
    /// kept for that reason (overrides merge-status-based deletion unless
    /// `--force` is set).
    has_active_session: bool,
}

pub async fn run(args: &CleanArgs) -> Result<()> {
    if args.all {
        return run_all(args).await;
    }

    let repo = GitRepo::open_from_env().map_err(|_| WmError::NotInGitRepo)?;
    let main_repo = get_main_repo(&repo)?;

    fetch_with_prune(&main_repo).context("Failed to fetch from remote")?;

    let (mut to_delete, mut to_keep) = collect_worktrees(&main_repo, None).await?;

    apply_active_session_protection(&mut to_delete, &mut to_keep, args.force);

    if to_delete.is_empty() && to_keep.is_empty() {
        println!("No worktrees found.");
        return Ok(());
    }

    display_worktrees_table(&to_delete, &to_keep, false);

    if to_delete.is_empty() {
        println!();
        println!("No merged worktrees to delete.");
        return Ok(());
    }

    if args.dry_run {
        println!();
        println!("(dry-run mode, no changes made)");
        return Ok(());
    }

    if !confirm_deletion() {
        println!("Cancelled.");
        return Ok(());
    }

    println!();
    delete_worktrees_single_repo(&main_repo, &to_delete)?;

    Ok(())
}

/// Collected local data for a single repository (no network required).
struct RepoWorktreeData {
    repo_path: PathBuf,
    repo_name: String,
    /// GitHub owner/repo from remote URL. None if not a GitHub repo.
    github_id: Option<(String, String)>,
    worktrees: Vec<LinkedWorktree>,
}

/// Run clean across all repositories under repos_root.
///
/// Optimized flow:
/// 1. Collect local data (worktrees, owner/repo) without network
/// 2. Batch GraphQL query for all branch PR statuses in one call
/// 3. Determine merge status from PR info (branches without PR are kept)
async fn run_all(args: &CleanArgs) -> Result<()> {
    let config = load_config()?;
    let repos_root = resolve_repos_root(config.wm.repos_root.as_deref())
        .context("Failed to resolve repos root")?;

    let repo_paths = discover_repos_with_worktrees(&repos_root, &config.wm.worktrees_dir);
    if repo_paths.is_empty() {
        println!(
            "No repositories with worktrees found under {}",
            repos_root.display()
        );
        return Ok(());
    }

    // Phase 1: Collect local data (no network)
    let mut repo_data: Vec<RepoWorktreeData> = Vec::new();
    for repo_path in &repo_paths {
        let repo_name = repo_path
            .strip_prefix(&repos_root)
            .unwrap_or(repo_path)
            .to_string_lossy()
            .to_string();

        let repo = match GitRepo::open_at(repo_path) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Warning: Failed to open {repo_name}: {e}");
                continue;
            }
        };

        let worktrees: Vec<LinkedWorktree> = match list_linked_worktrees(&repo) {
            Ok(wts) => wts
                .into_iter()
                .filter(|wt| !wt.branch.is_empty() && wt.branch != "(unknown)")
                .collect(),
            Err(e) => {
                eprintln!("Warning: Failed to list worktrees for {repo_name}: {e}");
                continue;
            }
        };

        if worktrees.is_empty() {
            continue;
        }

        let github_id = github_owner_and_repo(&repo).ok();

        repo_data.push(RepoWorktreeData {
            repo_path: repo_path.clone(),
            repo_name,
            github_id,
            worktrees,
        });
    }

    if repo_data.iter().all(|rd| rd.worktrees.is_empty()) {
        println!("No worktrees found across all repositories.");
        return Ok(());
    }

    // Phase 2: Batch GraphQL query for PR statuses
    let spinner = create_spinner("Checking PR status...");

    let queries: Vec<BranchPrQuery> = repo_data
        .iter()
        .filter_map(|rd| {
            let (owner, repo) = rd.github_id.as_ref()?;
            Some(rd.worktrees.iter().map(|wt| BranchPrQuery {
                owner: owner.clone(),
                repo: repo.clone(),
                branch: wt.branch.clone(),
            }))
        })
        .flatten()
        .collect();

    let pr_map = if !queries.is_empty() {
        match GitHubClient::get() {
            Ok(client) => client.get_prs_for_branches_batch(&queries).await.ok(),
            Err(_) => None,
        }
    } else {
        None
    };
    let pr_map = pr_map.unwrap_or_default();

    // Phase 3: Determine merge status
    let mut all_to_delete = Vec::new();
    let mut all_to_keep = Vec::new();

    for rd in &repo_data {
        let repo = match GitRepo::open_at(&rd.repo_path) {
            Ok(r) => r,
            Err(_) => continue,
        };

        for wt in &rd.worktrees {
            let status = if let Some((owner, repo_gh)) = &rd.github_id
                && let Some(Some(pr_info)) =
                    pr_map.get(&(owner.clone(), repo_gh.clone(), wt.branch.clone()))
            {
                merge_status_from_pr(pr_info)
            } else {
                merge_status_from_git(&repo, &wt.branch)
            };

            let should_delete = status.should_cleanup();
            let info = CleanWorktreeInfo {
                wt: wt.clone(),
                status,
                repo_name: Some(rd.repo_name.clone()),
                has_active_session: false,
            };

            if should_delete {
                all_to_delete.push(info);
            } else {
                all_to_keep.push(info);
            }
        }
    }

    spinner.finish_and_clear();

    apply_active_session_protection(&mut all_to_delete, &mut all_to_keep, args.force);

    if all_to_delete.is_empty() && all_to_keep.is_empty() {
        println!("No worktrees found across all repositories.");
        return Ok(());
    }

    display_worktrees_table(&all_to_delete, &all_to_keep, true);

    if all_to_delete.is_empty() {
        println!();
        println!("No merged worktrees to delete.");
        return Ok(());
    }

    if args.dry_run {
        println!();
        println!("(dry-run mode, no changes made)");
        return Ok(());
    }

    if !confirm_deletion() {
        println!("Cancelled.");
        return Ok(());
    }

    println!();
    delete_worktrees_all_repos(&repos_root, &all_to_delete)?;

    Ok(())
}

/// Move worktrees that currently host an active Claude Code session out of
/// `to_delete` and into `to_keep`, tagging each moved entry as
/// `has_active_session = true` so the table renderer can show the reason.
///
/// `--force` skips this protection entirely. Best-effort: if sessions cannot
/// be loaded (e.g., the sessions dir does not exist), no protection is
/// applied -- a user without any session state cannot have anything to
/// protect.
fn apply_active_session_protection(
    to_delete: &mut Vec<CleanWorktreeInfo>,
    to_keep: &mut Vec<CleanWorktreeInfo>,
    force: bool,
) {
    if force {
        return;
    }

    // Drop stale pending_bg_task_ids before listing so a session whose only
    // remaining liveness signal is a long-dead bg id stops protecting its
    // worktree. The launchd-driven `cc sweep` runs every 5 min, which is too
    // coarse for an interactive `wm clean` invocation.
    if let Err(e) = sweep_pending_bg_tasks_in_all_sessions() {
        eprintln!("Warning: failed to sweep stale bg tasks: {e}");
    }

    let sessions = match list_sessions() {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "Warning: failed to list Claude Code sessions, active session protection disabled: {e}"
            );
            return;
        }
    };
    if sessions.is_empty() {
        return;
    }

    let timeout = load_config()
        .ok()
        .and_then(|c| parse_duration(&c.cc.auto_pause.timeout).ok())
        .unwrap_or(Duration::from_secs(30 * 60));

    // wm clean may be invoked from a non-tmux context (cron, plain shell);
    // fall back to the no-op probe so we never block on tmux calls.
    if std::env::var_os("TMUX").is_some() {
        protect_active_worktrees(to_delete, to_keep, &sessions, timeout, &TmuxActivityProbe);
    } else {
        protect_active_worktrees(to_delete, to_keep, &sessions, timeout, &NoActivityProbe);
    }
}

/// Pure core of [`apply_active_session_protection`]. Exposed for tests so the
/// session list, timeout, and activity probe can be injected without
/// touching disk or tmux.
fn protect_active_worktrees<P: ActivityProbe>(
    to_delete: &mut Vec<CleanWorktreeInfo>,
    to_keep: &mut Vec<CleanWorktreeInfo>,
    sessions: &[Session],
    timeout: Duration,
    probe: &P,
) {
    let now = Utc::now();

    let mut kept = Vec::new();
    to_delete.retain_mut(|info| {
        if contains_active_session(&info.wt.path, sessions, probe, now, timeout) {
            kept.push(CleanWorktreeInfo {
                wt: info.wt.clone(),
                status: info.status.clone(),
                repo_name: info.repo_name.clone(),
                has_active_session: true,
            });
            false
        } else {
            true
        }
    });
    to_keep.extend(kept);

    // Tag worktrees already in to_keep so the reason column shows
    // "active session" alongside their merge reason.
    for info in to_keep.iter_mut() {
        if !info.has_active_session
            && contains_active_session(&info.wt.path, sessions, probe, now, timeout)
        {
            info.has_active_session = true;
        }
    }
}

/// Create a stderr spinner with braille animation for long-running operations.
fn create_spinner(message: &str) -> ProgressBar {
    if std::io::stderr().is_terminal() {
        let s = ProgressBar::new_spinner();
        s.set_style(
            ProgressStyle::default_spinner()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
                .template("{spinner} {msg}")
                .unwrap_or(ProgressStyle::default_spinner()),
        );
        s.set_message(message.to_string());
        s.enable_steady_tick(Duration::from_millis(80));
        s
    } else {
        ProgressBar::hidden()
    }
}

/// Get the display color for a merge status (GitHub-style)
fn status_color(status: &MergeStatus) -> &'static str {
    match status {
        MergeStatus::Merged { .. } => color::MAGENTA, // Purple for merged
        MergeStatus::Closed { .. } => color::RED,     // Red for closed PR (not merged)
        MergeStatus::NotMerged { .. } => color::GREEN, // Green for open PR or not merged
    }
}

/// Get the icon for merge status
fn status_icon(status: &MergeStatus) -> &'static str {
    match status {
        MergeStatus::Merged { .. } => "✓",
        MergeStatus::Closed { .. } => "✗",
        MergeStatus::NotMerged { .. } => " ",
    }
}

/// Column widths for table display
const REPO_WIDTH: usize = 34;
const NAME_WIDTH: usize = 28;
const BRANCH_WIDTH: usize = 28;

/// Display all worktrees in a table format (prints to stdout)
fn display_worktrees_table(
    to_delete: &[CleanWorktreeInfo],
    to_keep: &[CleanWorktreeInfo],
    show_repo: bool,
) {
    let mut stdout = io::stdout().lock();
    // Ignore write errors to stdout
    let _ = render_worktrees_table(&mut stdout, to_delete, to_keep, show_repo);
}

/// Render all worktrees in a table format to the given writer.
/// Separated from display function to enable testing.
fn render_worktrees_table<W: Write>(
    writer: &mut W,
    to_delete: &[CleanWorktreeInfo],
    to_keep: &[CleanWorktreeInfo],
    show_repo: bool,
) -> io::Result<()> {
    // Print header
    if show_repo {
        writeln!(
            writer,
            "{} {} {} STATUS",
            pad_or_truncate("REPO", REPO_WIDTH),
            pad_or_truncate("NAME", NAME_WIDTH),
            pad_or_truncate("BRANCH", BRANCH_WIDTH)
        )?;
    } else {
        writeln!(
            writer,
            "{} {} STATUS",
            pad_or_truncate("NAME", NAME_WIDTH),
            pad_or_truncate("BRANCH", BRANCH_WIDTH)
        )?;
    }

    // Combine: to_delete first, then to_keep
    for info in to_delete.iter().chain(to_keep.iter()) {
        let name_cell = pad_or_truncate(&info.wt.name, NAME_WIDTH);
        let branch_cell = pad_or_truncate(&info.wt.branch, BRANCH_WIDTH);

        let icon = status_icon(&info.status);
        let status_text = info.status.reason();
        let status_col = status_color(&info.status);
        // Format: icon + colored status (matching cc list format)
        let icon_part = if icon.trim().is_empty() {
            String::new()
        } else {
            format!("{icon} ")
        };
        let base_status = format!("{status_col}{icon_part}{status_text}{}", color::RESET);
        let colored_status = if info.has_active_session {
            format!(
                "{}{} ⏵ active session{}",
                base_status,
                color::YELLOW,
                color::RESET
            )
        } else {
            base_status
        };

        if show_repo {
            let repo_cell = pad_or_truncate(info.repo_name.as_deref().unwrap_or(""), REPO_WIDTH);
            writeln!(
                writer,
                "{repo_cell} {name_cell} {branch_cell} {colored_status}"
            )?;
        } else {
            writeln!(writer, "{name_cell} {branch_cell} {colored_status}")?;
        }
    }

    Ok(())
}

/// Prompt user for confirmation
fn confirm_deletion() -> bool {
    println!();
    print!("Delete these worktrees? [y/N] ");
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    input.trim().eq_ignore_ascii_case("y")
}

/// Delete all worktrees and their branches for a single repository.
fn delete_worktrees_single_repo(repo: &GitRepo, worktrees: &[CleanWorktreeInfo]) -> Result<()> {
    let mut deleted_count = 0;

    for info in worktrees {
        let result =
            crate::shared::cleanup::cleanup_worktree_by_name(repo, &info.wt.name, &info.wt.path)?;

        if result.worktree_deleted {
            println!("Deleted: {}", info.wt.name);
            deleted_count += 1;

            if let Some(branch) = &result.branch_deleted {
                println!("  Branch deleted: {branch}");
            }
            if result.windows_closed > 0 {
                println!("  Tmux windows closed: {}", result.windows_closed);
            }
            if result.sessions_cleaned > 0 {
                println!("  Sessions cleaned: {}", result.sessions_cleaned);
            }
        }
    }

    println!();
    println!("Done. Deleted {deleted_count} worktree(s).");

    Ok(())
}

/// Delete worktrees across multiple repositories (--all mode).
/// Groups worktrees by repo_name to avoid reopening the same repository.
fn delete_worktrees_all_repos(repos_root: &Path, worktrees: &[CleanWorktreeInfo]) -> Result<()> {
    let mut deleted_count = 0;

    // Group worktrees by repository so input order doesn't matter
    let mut by_repo: std::collections::BTreeMap<&str, Vec<&CleanWorktreeInfo>> =
        std::collections::BTreeMap::new();
    for info in worktrees {
        let repo_name = info.repo_name.as_deref().unwrap_or("");
        by_repo.entry(repo_name).or_default().push(info);
    }

    for (repo_name, infos) in &by_repo {
        let repo_path = repos_root.join(repo_name);
        let repo = match GitRepo::open_at(&repo_path) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Warning: Failed to open {repo_name}: {e}");
                continue;
            }
        };

        for info in infos {
            let result = crate::shared::cleanup::cleanup_worktree_by_name(
                &repo,
                &info.wt.name,
                &info.wt.path,
            )?;

            if result.worktree_deleted {
                println!("Deleted: {repo_name}/{}", info.wt.name);
                deleted_count += 1;

                if let Some(branch) = &result.branch_deleted {
                    println!("  Branch deleted: {branch}");
                }
                if result.windows_closed > 0 {
                    println!("  Tmux windows closed: {}", result.windows_closed);
                }
                if result.sessions_cleaned > 0 {
                    println!("  Sessions cleaned: {}", result.sessions_cleaned);
                }
            }
        }
    }

    println!();
    println!("Done. Deleted {deleted_count} worktree(s).");

    Ok(())
}

/// Collect all worktrees and categorize them by merge status
async fn collect_worktrees(
    repo: &GitRepo,
    repo_name: Option<&str>,
) -> Result<(Vec<CleanWorktreeInfo>, Vec<CleanWorktreeInfo>)> {
    let mut to_delete = Vec::new();
    let mut to_keep = Vec::new();

    for wt in list_linked_worktrees(repo)? {
        if wt.branch.is_empty() || wt.branch == "(unknown)" {
            continue;
        }

        let status = get_merge_status_for_repo(repo, &wt.branch).await;
        let should_delete = status.should_cleanup();
        let info = CleanWorktreeInfo {
            wt,
            status,
            repo_name: repo_name.map(|s| s.to_string()),
            has_active_session: false,
        };

        if should_delete {
            to_delete.push(info);
        } else {
            to_keep.push(info);
        }
    }

    Ok((to_delete, to_keep))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::SessionStatus;
    use crate::shared::active_session::NoActivityProbe;
    use crate::shared::testing::TestRepo;
    use chrono::Utc;
    use indoc::indoc;
    use rstest::rstest;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn make_session_at(id: &str, status: SessionStatus, cwd: PathBuf) -> Session {
        let now = Utc::now();
        Session {
            session_id: id.to_string(),
            cwd,
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: now,
            updated_at: now,
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_outputs: BTreeSet::new(),
            read_at: None,
            sweep_signaled: false,
        }
    }

    fn make_clean_info(
        name: &str,
        path: PathBuf,
        branch: &str,
        status: MergeStatus,
    ) -> CleanWorktreeInfo {
        CleanWorktreeInfo {
            wt: LinkedWorktree {
                name: name.to_string(),
                path,
                branch: branch.to_string(),
                commit: "abc1234".to_string(),
            },
            status,
            repo_name: None,
            has_active_session: false,
        }
    }

    fn make_clean_info_with_repo(
        name: &str,
        path: PathBuf,
        branch: &str,
        status: MergeStatus,
        repo_name: &str,
    ) -> CleanWorktreeInfo {
        CleanWorktreeInfo {
            wt: LinkedWorktree {
                name: name.to_string(),
                path,
                branch: branch.to_string(),
                commit: "abc1234".to_string(),
            },
            status,
            repo_name: Some(repo_name.to_string()),
            has_active_session: false,
        }
    }

    // delete_worktrees_single_repo and delete_worktrees_all_repos are not
    // directly tested here because they call cleanup_worktree_by_name which
    // invokes external tmux commands. The core worktree deletion logic is
    // tested in shared::cleanup::tests::delete_worktree_and_branch_*.

    #[rstest]
    #[case::merged(MergeStatus::Merged { reason: "test".to_string() }, "✓")]
    #[case::closed(MergeStatus::Closed { reason: "test".to_string() }, "✗")]
    #[case::not_merged(MergeStatus::NotMerged { reason: "test".to_string() }, " ")]
    fn test_status_icon(#[case] status: MergeStatus, #[case] expected: &str) {
        assert_eq!(status_icon(&status), expected);
    }

    #[rstest]
    #[case::merged(MergeStatus::Merged { reason: "test".to_string() }, color::MAGENTA)]
    #[case::open_pr(MergeStatus::NotMerged { reason: "open".to_string() }, color::GREEN)]
    #[case::not_merged(MergeStatus::NotMerged { reason: "Not merged".to_string() }, color::GREEN)]
    #[case::closed_pr(MergeStatus::Closed { reason: "closed".to_string() }, color::RED)]
    fn test_status_color(#[case] status: MergeStatus, #[case] expected: &str) {
        assert_eq!(status_color(&status), expected);
    }

    // =========================================================================
    // Integration tests for table rendering
    // =========================================================================

    #[test]
    fn test_render_worktrees_table_empty() {
        let mut output = Vec::new();
        render_worktrees_table(&mut output, &[], &[], false).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        // Header only, no data rows
        assert_eq!(
            result,
            "NAME                         BRANCH                       STATUS\n"
        );
    }

    #[test]
    fn test_render_worktrees_table_merged_only() {
        let test_repo = TestRepo::new();
        let to_delete = vec![make_clean_info(
            "feature-branch",
            test_repo.path().join(".worktrees/feature-branch"),
            "fohte/feature-branch",
            MergeStatus::Merged {
                reason: "merged".to_string(),
            },
        )];

        let mut output = Vec::new();
        render_worktrees_table(&mut output, &to_delete, &[], false).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                NAME                         BRANCH                       STATUS
                feature-branch               fohte/feature-branch         \x1b[35m✓ merged\x1b[0m
            "}
        );
    }

    #[test]
    fn test_render_worktrees_table_not_merged_only() {
        let test_repo = TestRepo::new();
        let to_keep = vec![make_clean_info(
            "wip-feature",
            test_repo.path().join(".worktrees/wip-feature"),
            "fohte/wip-feature",
            MergeStatus::NotMerged {
                reason: "open".to_string(),
            },
        )];

        let mut output = Vec::new();
        render_worktrees_table(&mut output, &[], &to_keep, false).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        // Space instead of checkmark, green color for open PR
        assert_eq!(
            result,
            indoc! {"
                NAME                         BRANCH                       STATUS
                wip-feature                  fohte/wip-feature            \x1b[32mopen\x1b[0m
            "}
        );
    }

    #[test]
    fn test_render_worktrees_table_mixed() {
        let test_repo = TestRepo::new();
        let to_delete = vec![make_clean_info(
            "merged-feature",
            test_repo.path().join(".worktrees/merged-feature"),
            "fohte/merged-feature",
            MergeStatus::Merged {
                reason: "Merged (git)".to_string(),
            },
        )];
        let to_keep = vec![
            make_clean_info(
                "open-pr",
                test_repo.path().join(".worktrees/open-pr"),
                "fohte/open-pr",
                MergeStatus::NotMerged {
                    reason: "open".to_string(),
                },
            ),
            make_clean_info(
                "no-pr",
                test_repo.path().join(".worktrees/no-pr"),
                "fohte/no-pr",
                MergeStatus::NotMerged {
                    reason: "Not merged".to_string(),
                },
            ),
        ];

        let mut output = Vec::new();
        render_worktrees_table(&mut output, &to_delete, &to_keep, false)
            .expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        // to_delete comes first, then to_keep
        assert_eq!(
            result,
            indoc! {"
                NAME                         BRANCH                       STATUS
                merged-feature               fohte/merged-feature         \x1b[35m✓ Merged (git)\x1b[0m
                open-pr                      fohte/open-pr                \x1b[32mopen\x1b[0m
                no-pr                        fohte/no-pr                  \x1b[32mNot merged\x1b[0m
            "}
        );
    }

    #[test]
    fn test_render_worktrees_table_truncates_long_names() {
        let test_repo = TestRepo::new();
        let to_delete = vec![make_clean_info(
            "this-is-a-very-long-worktree-name-that-should-be-truncated",
            test_repo.path().join(".worktrees/long"),
            "fohte/this-is-a-very-long-branch-name-that-should-be-truncated",
            MergeStatus::Merged {
                reason: "merged".to_string(),
            },
        )];

        let mut output = Vec::new();
        render_worktrees_table(&mut output, &to_delete, &[], false).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                NAME                         BRANCH                       STATUS
                this-is-a-very-long-workt... fohte/this-is-a-very-long... \x1b[35m✓ merged\x1b[0m
            "}
        );
    }

    #[test]
    fn test_render_worktrees_table_all_status_colors() {
        let test_repo = TestRepo::new();
        let to_delete = vec![
            make_clean_info(
                "pr-merged",
                test_repo.path().join(".worktrees/pr-merged"),
                "fohte/pr-merged",
                MergeStatus::Merged {
                    reason: "merged".to_string(),
                },
            ),
            make_clean_info(
                "pr-closed",
                test_repo.path().join(".worktrees/pr-closed"),
                "fohte/pr-closed",
                MergeStatus::Closed {
                    reason: "closed".to_string(),
                },
            ),
        ];
        let to_keep = vec![make_clean_info(
            "pr-open",
            test_repo.path().join(".worktrees/pr-open"),
            "fohte/pr-open",
            MergeStatus::NotMerged {
                reason: "open".to_string(),
            },
        )];

        let mut output = Vec::new();
        render_worktrees_table(&mut output, &to_delete, &to_keep, false)
            .expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                NAME                         BRANCH                       STATUS
                pr-merged                    fohte/pr-merged              \x1b[35m✓ merged\x1b[0m
                pr-closed                    fohte/pr-closed              \x1b[31m✗ closed\x1b[0m
                pr-open                      fohte/pr-open                \x1b[32mopen\x1b[0m
            "}
        );
    }

    // =========================================================================
    // Tests for --all mode table rendering (with REPO column)
    // =========================================================================

    #[test]
    fn test_render_worktrees_table_with_repo_column_empty() {
        let mut output = Vec::new();
        render_worktrees_table(&mut output, &[], &[], true).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            "REPO                               NAME                         BRANCH                       STATUS\n"
        );
    }

    #[test]
    fn test_render_worktrees_table_with_repo_column() {
        let test_repo = TestRepo::new();
        let to_delete = vec![make_clean_info_with_repo(
            "merged-feature",
            test_repo.path().join(".worktrees/merged-feature"),
            "fohte/merged-feature",
            MergeStatus::Merged {
                reason: "Merged (git)".to_string(),
            },
            "github.com/fohte/armyknife",
        )];
        let to_keep = vec![make_clean_info_with_repo(
            "open-pr",
            test_repo.path().join(".worktrees/open-pr"),
            "fohte/open-pr",
            MergeStatus::NotMerged {
                reason: "open".to_string(),
            },
            "github.com/fohte/other-repo",
        )];

        let mut output = Vec::new();
        render_worktrees_table(&mut output, &to_delete, &to_keep, true)
            .expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                REPO                               NAME                         BRANCH                       STATUS
                github.com/fohte/armyknife         merged-feature               fohte/merged-feature         \x1b[35m✓ Merged (git)\x1b[0m
                github.com/fohte/other-repo        open-pr                      fohte/open-pr                \x1b[32mopen\x1b[0m
            "}
        );
    }

    // =========================================================================
    // Tests for active-session protection
    // =========================================================================

    #[rstest]
    fn protection_moves_active_worktree_from_delete_to_keep() {
        let wt_path = PathBuf::from("/tmp/wt-active");
        let mut to_delete = vec![make_clean_info(
            "wt-active",
            wt_path.clone(),
            "fohte/wt-active",
            MergeStatus::Merged {
                reason: "merged".to_string(),
            },
        )];
        let mut to_keep = Vec::new();

        let sessions = vec![make_session_at(
            "s1",
            SessionStatus::Running,
            wt_path.clone(),
        )];

        protect_active_worktrees(
            &mut to_delete,
            &mut to_keep,
            &sessions,
            Duration::from_secs(60),
            &NoActivityProbe,
        );

        assert!(to_delete.is_empty(), "active worktree must not be deleted");
        assert_eq!(to_keep.len(), 1);
        assert!(to_keep[0].has_active_session);
        assert_eq!(to_keep[0].wt.path, wt_path);
    }

    #[rstest]
    fn protection_keeps_inactive_worktrees_in_delete() {
        let wt_path = PathBuf::from("/tmp/wt-merged");
        let mut to_delete = vec![make_clean_info(
            "wt-merged",
            wt_path,
            "fohte/wt-merged",
            MergeStatus::Merged {
                reason: "merged".to_string(),
            },
        )];
        let mut to_keep = Vec::new();

        // Session for a completely different worktree.
        let sessions = vec![make_session_at(
            "s1",
            SessionStatus::Running,
            PathBuf::from("/tmp/some-other-wt"),
        )];

        protect_active_worktrees(
            &mut to_delete,
            &mut to_keep,
            &sessions,
            Duration::from_secs(60),
            &NoActivityProbe,
        );

        assert_eq!(to_delete.len(), 1);
        assert!(to_keep.is_empty());
    }

    #[rstest]
    fn protection_tags_already_kept_worktrees_with_active_session() {
        let wt_path = PathBuf::from("/tmp/wt-wip");
        let mut to_delete = Vec::new();
        let mut to_keep = vec![make_clean_info(
            "wt-wip",
            wt_path.clone(),
            "fohte/wt-wip",
            MergeStatus::NotMerged {
                reason: "open".to_string(),
            },
        )];

        let sessions = vec![make_session_at("s1", SessionStatus::WaitingInput, wt_path)];

        protect_active_worktrees(
            &mut to_delete,
            &mut to_keep,
            &sessions,
            Duration::from_secs(60),
            &NoActivityProbe,
        );

        assert_eq!(to_keep.len(), 1);
        assert!(
            to_keep[0].has_active_session,
            "already-kept worktree with active session must be tagged"
        );
    }

    #[rstest]
    fn render_shows_active_session_marker() {
        let test_repo = TestRepo::new();
        let mut info = make_clean_info(
            "wip",
            test_repo.path().join(".worktrees/wip"),
            "fohte/wip",
            MergeStatus::NotMerged {
                reason: "open".to_string(),
            },
        );
        info.has_active_session = true;

        let mut output = Vec::new();
        render_worktrees_table(&mut output, &[], &[info], false).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                NAME                         BRANCH                       STATUS
                wip                          fohte/wip                    \x1b[32mopen\x1b[0m\x1b[33m ⏵ active session\x1b[0m
            "}
        );
    }

    #[test]
    fn test_render_worktrees_table_with_repo_column_truncates_long_repo() {
        let test_repo = TestRepo::new();
        let to_delete = vec![make_clean_info_with_repo(
            "feature",
            test_repo.path().join(".worktrees/feature"),
            "fohte/feature",
            MergeStatus::Merged {
                reason: "merged".to_string(),
            },
            "github.com/very-long-org-name/very-long-repo-name",
        )];

        let mut output = Vec::new();
        render_worktrees_table(&mut output, &to_delete, &[], true).expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                REPO                               NAME                         BRANCH                       STATUS
                github.com/very-long-org-name/v... feature                      fohte/feature                \x1b[35m✓ merged\x1b[0m
            "}
        );
    }
}
