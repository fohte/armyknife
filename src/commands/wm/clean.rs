use anyhow::Context;
use clap::Args;
use git2::Repository;
use std::io::{self, Write};
use std::path::Path;

use super::error::{Result, WmError};
use super::git::get_merge_status;
use super::worktree::{
    LinkedWorktree, delete_branch_if_exists, delete_worktree, get_main_repo, list_linked_worktrees,
};
use crate::infra::git::MergeStatus;
use crate::infra::git::fetch_with_prune;
use crate::infra::tmux;
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
}

/// Worktree with merge status and associated tmux windows for clean command.
struct CleanWorktreeInfo {
    wt: LinkedWorktree,
    status: MergeStatus,
    /// Tmux window IDs that are located in this worktree's path
    window_ids: Vec<String>,
    /// Repository name (relative path from repos_root). Only set in --all mode.
    repo_name: Option<String>,
}

pub async fn run(args: &CleanArgs) -> Result<()> {
    if args.all {
        return run_all(args).await;
    }

    let repo = Repository::open_from_env().map_err(|_| WmError::NotInGitRepo)?;
    let main_repo = get_main_repo(&repo)?;

    fetch_with_prune(&main_repo).context("Failed to fetch from remote")?;

    let (to_delete, to_keep) = collect_worktrees(&main_repo, None).await?;

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

/// Run clean across all repositories under repos_root.
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

    let mut all_to_delete = Vec::new();
    let mut all_to_keep = Vec::new();

    for repo_path in &repo_paths {
        let repo_name = repo_path
            .strip_prefix(&repos_root)
            .unwrap_or(repo_path)
            .to_string_lossy()
            .to_string();

        let repo = match Repository::open(repo_path) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Warning: Failed to open {repo_name}: {e}");
                continue;
            }
        };

        if let Err(e) = fetch_with_prune(&repo) {
            eprintln!("Warning: Failed to fetch {repo_name}: {e}");
            continue;
        }

        let (to_delete, to_keep) = collect_worktrees(&repo, Some(&repo_name)).await?;
        all_to_delete.extend(to_delete);
        all_to_keep.extend(to_keep);
    }

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
    if status.is_merged() { "✓" } else { " " }
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
        let colored_status = format!("{status_col}{icon_part}{status_text}{}", color::RESET);

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
fn delete_worktrees_single_repo(repo: &Repository, worktrees: &[CleanWorktreeInfo]) -> Result<()> {
    let mut deleted_count = 0;

    for info in worktrees {
        if delete_worktree(repo, &info.wt.name)? {
            println!("Deleted: {}", info.wt.name);
            deleted_count += 1;

            if delete_branch_if_exists(repo, &info.wt.branch) {
                println!("  Branch deleted: {}", info.wt.branch);
            }

            // Close tmux windows that were in the deleted worktree
            for window_id in &info.window_ids {
                if tmux::kill_window(window_id).is_ok() {
                    println!("  Tmux window closed: {window_id}");
                }
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
        let repo = match Repository::open(&repo_path) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Warning: Failed to open {repo_name}: {e}");
                continue;
            }
        };

        for info in infos {
            if delete_worktree(&repo, &info.wt.name)? {
                println!("Deleted: {repo_name}/{}", info.wt.name);
                deleted_count += 1;

                if delete_branch_if_exists(&repo, &info.wt.branch) {
                    println!("  Branch deleted: {}", info.wt.branch);
                }

                for window_id in &info.window_ids {
                    if tmux::kill_window(window_id).is_ok() {
                        println!("  Tmux window closed: {window_id}");
                    }
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
    repo: &Repository,
    repo_name: Option<&str>,
) -> Result<(Vec<CleanWorktreeInfo>, Vec<CleanWorktreeInfo>)> {
    let mut to_delete = Vec::new();
    let mut to_keep = Vec::new();

    for wt in list_linked_worktrees(repo)? {
        if wt.branch.is_empty() || wt.branch == "(unknown)" {
            continue;
        }

        let status = get_merge_status(&wt.branch).await;
        // Collect tmux window IDs while the worktree path still exists
        let window_ids = tmux::get_window_ids_in_path(&wt.path.to_string_lossy());
        let is_merged = status.is_merged();
        let info = CleanWorktreeInfo {
            wt,
            status,
            window_ids,
            repo_name: repo_name.map(|s| s.to_string()),
        };

        if is_merged {
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
    use crate::shared::testing::TestRepo;
    use indoc::indoc;
    use rstest::rstest;
    use std::path::PathBuf;

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
            window_ids: Vec::new(),
            repo_name: None,
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
            window_ids: Vec::new(),
            repo_name: Some(repo_name.to_string()),
        }
    }

    #[test]
    fn delete_worktrees_deletes_all_worktrees() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature-a");
        test_repo.create_worktree("feature-b");

        let repo = test_repo.open();

        let worktrees = vec![
            make_clean_info(
                "feature-a",
                test_repo.worktree_path("feature-a"),
                "feature-a",
                MergeStatus::Merged {
                    reason: "merged".to_string(),
                },
            ),
            make_clean_info(
                "feature-b",
                test_repo.worktree_path("feature-b"),
                "feature-b",
                MergeStatus::Merged {
                    reason: "merged".to_string(),
                },
            ),
        ];

        delete_worktrees_single_repo(&repo, &worktrees).unwrap();

        // Verify worktrees are deleted
        assert!(repo.find_worktree("feature-a").is_err());
        assert!(repo.find_worktree("feature-b").is_err());
    }

    #[test]
    fn delete_worktrees_handles_empty_list() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let result = delete_worktrees_single_repo(&repo, &[]);
        assert!(result.is_ok());
    }

    #[rstest]
    #[case::merged(MergeStatus::Merged { reason: "test".to_string() }, "✓")]
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
        let to_delete = vec![make_clean_info(
            "pr-merged",
            test_repo.path().join(".worktrees/pr-merged"),
            "fohte/pr-merged",
            MergeStatus::Merged {
                reason: "merged".to_string(),
            },
        )];
        let to_keep = vec![
            make_clean_info(
                "pr-open",
                test_repo.path().join(".worktrees/pr-open"),
                "fohte/pr-open",
                MergeStatus::NotMerged {
                    reason: "open".to_string(),
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

        let mut output = Vec::new();
        render_worktrees_table(&mut output, &to_delete, &to_keep, false)
            .expect("render should succeed");

        let result = String::from_utf8(output).expect("valid utf8");
        assert_eq!(
            result,
            indoc! {"
                NAME                         BRANCH                       STATUS
                pr-merged                    fohte/pr-merged              \x1b[35m✓ merged\x1b[0m
                pr-open                      fohte/pr-open                \x1b[32mopen\x1b[0m
                pr-closed                    fohte/pr-closed              \x1b[31mclosed\x1b[0m
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
