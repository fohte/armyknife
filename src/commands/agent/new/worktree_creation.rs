use anyhow::{Context, Result};
use std::path::Path;

use super::NewArgs;
use super::run_pre_session_start_hook;
use super::session_context::resolve_prompt;
use super::tmux::{TmuxWindowSpec, setup_tmux_window};
use super::tmux_launch_inputs;
use super::worktree::{
    BranchRollback, WorktreeAddMode, add_worktree_for_branch, git_worktree_add, repo_branch_exists,
    rollback_worktree,
};
use crate::commands::agent::error::CcError;
use crate::infra::git::cmd::run_git;
use crate::infra::git::fetch_with_prune;
use crate::infra::git::{WorktreeCreationLock, get_main_branch_for_repo, open_repo_at};
use crate::shared::config::{Config, ResolvedLaunchOptions};
use crate::shared::env_var::EnvVars;
use crate::shared::hooks;
use crate::shared::worktree::{LinkedWorktree, branch_to_worktree_name, list_linked_worktrees};

struct WorktreeHookContext<'a> {
    worktree_dir: Option<&'a Path>,
    actual_branch: &'a str,
    repo_root: &'a str,
}

fn run_worktree_hooks(
    context: WorktreeHookContext<'_>,
    mut run_hook: impl FnMut(&str, &[(&str, &str)]) -> Result<()>,
    rollback: impl FnOnce(&str),
) -> Result<()> {
    let WorktreeHookContext {
        worktree_dir,
        actual_branch,
        repo_root,
    } = context;

    let Some(worktree_dir) = worktree_dir else {
        eprintln!("Skipping post-worktree-create hook (--skip-hooks)");
        return run_pre_session_start_hook(None, run_hook);
    };

    // `worktree_dir` is built from the absolute repository root. Keep that
    // path as a fallback when canonicalization fails, matching the old hook behavior.
    let worktree_abs =
        std::fs::canonicalize(worktree_dir).unwrap_or_else(|_| worktree_dir.to_path_buf());
    let worktree_path = worktree_abs.to_string_lossy();
    let post_worktree_env = [
        (EnvVars::worktree_path_name(), worktree_path.as_ref()),
        (EnvVars::branch_name_name(), actual_branch),
        (EnvVars::repo_root_name(), repo_root),
    ];

    if let Err(error) = run_hook("post-worktree-create", &post_worktree_env) {
        rollback("post-worktree-create");
        return Err(error).context("Failed to run post-worktree-create hook");
    }

    if let Err(error) = run_pre_session_start_hook(Some(&worktree_abs), &mut run_hook) {
        rollback("pre-session-start");
        return Err(error);
    }

    Ok(())
}

fn find_linked_worktree_for_branch(
    repo: &crate::infra::git::GitRepo,
    name: &str,
    branch_prefix: &str,
) -> Result<Option<LinkedWorktree>> {
    let branch = if repo_branch_exists(repo, name) {
        Some(name.to_string())
    } else {
        let name_no_prefix = name.strip_prefix(branch_prefix).unwrap_or(name);
        let prefixed_branch = format!("{branch_prefix}{name_no_prefix}");
        repo_branch_exists(repo, &prefixed_branch).then_some(prefixed_branch)
    };
    let Some(branch) = branch else {
        return Ok(None);
    };

    Ok(list_linked_worktrees(repo)?
        .into_iter()
        .find(|worktree| worktree.branch == branch))
}

fn run_reused_worktree_hook(
    worktree_dir: &Path,
    run_hook: impl FnOnce(&str, &[(&str, &str)]) -> Result<()>,
) -> Result<()> {
    let worktree_abs =
        std::fs::canonicalize(worktree_dir).unwrap_or_else(|_| worktree_dir.to_path_buf());
    run_pre_session_start_hook(Some(&worktree_abs), run_hook)
}

pub(super) fn run_worktree_creation(
    args: &NewArgs,
    name: &str,
    prompt: Option<&str>,
    repo_root: &str,
    config: &Config,
) -> Result<()> {
    let repo = open_repo_at(Path::new(repo_root)).map_err(|_| CcError::NotInGitRepo)?;
    let branch_prefix = &config.agent.worktree.branch_prefix;

    // Determine worktree directory name from branch name
    let worktree_name = branch_to_worktree_name(name, branch_prefix);
    let worktrees_dir = format!("{repo_root}/{}", config.agent.worktree.dir);
    let worktree_dir = Path::new(&worktrees_dir).join(&worktree_name);

    // Fetch with prune
    fetch_with_prune(&repo).context("Failed to fetch from remote")?;

    // The hook may update shared Git config, so keep the lock through the hook and rollback.
    let creation_lock = WorktreeCreationLock::acquire(&repo)?;

    // Remove branch prefix to avoid double prefix
    let name_no_prefix = name.strip_prefix(branch_prefix).unwrap_or(name);

    // Determine action based on branch existence and flags.
    // Keep the actual branch base separate from `--from`, which only selects
    // the base used when creating or resetting a branch.
    let actual_branch;
    let actual_base: Option<String>;
    let branch_rollback;
    let mut selected_worktree_dir = worktree_dir.clone();
    let mut selected_worktree_name = worktree_name.clone();
    let mut reused_worktree = false;

    if args.force {
        std::fs::create_dir_all(&worktrees_dir).context("Failed to create worktrees directory")?;
        // Force create new branch with prefix
        let main_branch = get_main_branch_for_repo(&repo)?;
        let base_branch = args
            .from
            .clone()
            .unwrap_or_else(|| format!("origin/{main_branch}"));
        let branch = format!("{branch_prefix}{name_no_prefix}");

        // ForceNewBranch resets a pre-existing local branch's tip; capture
        // it so rollback can restore the user's branch to its previous
        // commit on hook failure. Without the prior tip we cannot undo the
        // reset safely, so refuse rather than risk silent branch loss.
        branch_rollback = if repo.local_branch_exists(&branch) {
            let tip = run_git(repo.workdir(), ["rev-parse", &branch]).with_context(|| {
                format!("Failed to capture tip of '{branch}' before force reset")
            })?;
            BranchRollback::RestoreTip(tip)
        } else {
            BranchRollback::Delete
        };

        git_worktree_add(
            &repo,
            &worktree_dir,
            WorktreeAddMode::ForceNewBranch {
                branch: &branch,
                base: &base_branch,
            },
        )?;

        actual_branch = branch;
        actual_base = Some(base_branch);
    } else {
        let branch_with_prefix = format!("{branch_prefix}{name_no_prefix}");
        if let Some(linked_worktree) = find_linked_worktree_for_branch(&repo, name, branch_prefix)?
        {
            actual_branch = linked_worktree.branch.clone();
            selected_worktree_dir = linked_worktree.path;
            selected_worktree_name = linked_worktree.name;
            reused_worktree = true;
            branch_rollback = BranchRollback::Keep;
            actual_base = if prompt.is_some() {
                let main_branch = get_main_branch_for_repo(&repo)?;
                Some(format!("origin/{main_branch}"))
            } else {
                None
            };
        } else if repo_branch_exists(&repo, name) {
            // Branch exists in the main worktree or is available to check out.
            std::fs::create_dir_all(&worktrees_dir)
                .context("Failed to create worktrees directory")?;
            add_worktree_for_branch(&repo, &selected_worktree_dir, name)?;

            actual_branch = name.to_string();
            branch_rollback = BranchRollback::Keep;
            actual_base = if prompt.is_some() {
                let main_branch = get_main_branch_for_repo(&repo)?;
                Some(format!("origin/{main_branch}"))
            } else {
                None
            };
        } else if repo_branch_exists(&repo, &branch_with_prefix) {
            // Branch exists with prefix
            std::fs::create_dir_all(&worktrees_dir)
                .context("Failed to create worktrees directory")?;
            add_worktree_for_branch(&repo, &selected_worktree_dir, &branch_with_prefix)?;

            actual_branch = branch_with_prefix;
            branch_rollback = BranchRollback::Keep;
            actual_base = if prompt.is_some() {
                let main_branch = get_main_branch_for_repo(&repo)?;
                Some(format!("origin/{main_branch}"))
            } else {
                None
            };
        } else {
            std::fs::create_dir_all(&worktrees_dir)
                .context("Failed to create worktrees directory")?;
            // Branch doesn't exist, create new one with prefix
            let main_branch = get_main_branch_for_repo(&repo)?;
            let base_branch = args
                .from
                .clone()
                .unwrap_or_else(|| format!("origin/{main_branch}"));
            let branch = format!("{branch_prefix}{name_no_prefix}");

            git_worktree_add(
                &repo,
                &worktree_dir,
                WorktreeAddMode::NewBranch {
                    branch: &branch,
                    base: &base_branch,
                },
            )?;

            actual_branch = branch;
            actual_base = Some(base_branch);
            branch_rollback = BranchRollback::Delete;
        }
    }

    let final_prompt = if prompt.is_some() {
        let delegator_cwd = std::env::current_dir()
            .context("Failed to get current directory")?
            .to_string_lossy()
            .to_string();
        let worktree_cwd_str = selected_worktree_dir
            .to_str()
            .context("Invalid worktree path")?
            .to_string();

        resolve_prompt(
            prompt,
            Some(&actual_branch),
            actual_base.as_deref(),
            &delegator_cwd,
            &worktree_cwd_str,
        )
    } else {
        prompt.map(String::from)
    };

    // Hook failures roll back the worktree and any branch created for it.
    let rollback = |failed_hook: &str| {
        rollback_worktree(
            &repo,
            &selected_worktree_name,
            &actual_branch,
            &branch_rollback,
            failed_hook,
        )
    };
    if reused_worktree && !args.skip_hooks {
        run_reused_worktree_hook(&selected_worktree_dir, hooks::run_hook)?;
    } else {
        run_worktree_hooks(
            WorktreeHookContext {
                worktree_dir: (!args.skip_hooks).then_some(selected_worktree_dir.as_path()),
                actual_branch: &actual_branch,
                repo_root,
            },
            hooks::run_hook,
            rollback,
        )?;
    }

    let (env_vars, background) = tmux_launch_inputs(&args.common, args.vibe)?;
    let env_refs: Vec<(&str, &str)> = env_vars
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let launch_options = super::launch_options::resolve_launch_options(&args.common, config)?;

    drop(creation_lock);

    // Setup tmux window using config layout
    let route = setup_tmux_window(
        worktree_window_spec(WorktreeWindowContext {
            repo_root,
            worktree_name: &selected_worktree_name,
            worktree_dir: &selected_worktree_dir,
            config,
            launch_options: &launch_options,
            prompt: final_prompt.as_deref(),
            env_vars: &env_refs,
            background,
        }),
        config,
    )?;

    let suffix = if background { " (background)" } else { "" };
    let action = if reused_worktree { "Reused" } else { "Created" };
    println!(
        "{action} worktree '{}' and opened tmux window{}{}",
        selected_worktree_name,
        suffix,
        route.display_suffix()
    );

    Ok(())
}

struct WorktreeWindowContext<'a> {
    repo_root: &'a str,
    worktree_name: &'a str,
    worktree_dir: &'a Path,
    config: &'a Config,
    launch_options: &'a ResolvedLaunchOptions,
    prompt: Option<&'a str>,
    env_vars: &'a [(&'a str, &'a str)],
    background: bool,
}

fn worktree_window_spec(context: WorktreeWindowContext<'_>) -> TmuxWindowSpec<'_> {
    TmuxWindowSpec {
        repo_root: context.repo_root,
        cwd: context
            .worktree_dir
            .to_str()
            .unwrap_or(context.worktree_name),
        window_name: context.worktree_name,
        layout: &context.config.agent.worktree.layout,
        model: context.launch_options.model.as_deref(),
        reasoning_effort: context.launch_options.reasoning_effort,
        prompt: context.prompt,
        engine: context.launch_options.engine,
        env_vars: context.env_vars,
        background: context.background,
        restore_automatic_rename: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{Engine, ReasoningEffort};
    use crate::shared::testing::TestRepo;
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use std::cell::RefCell;

    #[derive(Debug, PartialEq, Eq)]
    struct HookInvocation {
        name: String,
        env_vars: Vec<(String, String)>,
    }

    fn capture_hook(name: &str, env_vars: &[(&str, &str)]) -> HookInvocation {
        HookInvocation {
            name: name.to_string(),
            env_vars: env_vars
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                .collect(),
        }
    }

    #[fixture]
    fn worktree_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[rstest]
    #[case::name_without_prefix("team-widget", "widget")]
    #[case::name_with_prefix("team-widget", "team-widget")]
    fn finds_existing_linked_worktree_with_or_without_branch_prefix(
        #[case] branch: &str,
        #[case] requested_name: &str,
    ) {
        let test_repo = TestRepo::new();
        test_repo.create_worktree(branch);
        let repo = test_repo.open();

        let linked_worktree =
            find_linked_worktree_for_branch(&repo, requested_name, "team-").unwrap();

        assert_eq!(
            linked_worktree.map(|worktree| (worktree.branch, worktree.path)),
            Some((branch.to_string(), test_repo.worktree_path(branch))),
        );
    }

    #[test]
    fn does_not_reuse_the_main_worktree_checkout() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();
        let main_branch = repo.current_branch().unwrap();

        let linked_worktree =
            find_linked_worktree_for_branch(&repo, &main_branch, "team-").unwrap();

        assert_eq!(linked_worktree.map(|worktree| worktree.branch), None);
    }

    #[test]
    fn reused_worktree_runs_only_pre_session_hook_and_keeps_the_path_on_failure() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("topic");
        let worktree_path = test_repo.worktree_path("topic");
        let events = RefCell::new(Vec::new());

        let result = run_reused_worktree_hook(&worktree_path, |name, env_vars| {
            events.borrow_mut().push(capture_hook(name, env_vars));
            anyhow::bail!("hook failed")
        });

        assert_eq!(
            (
                result.err().map(|error| error.to_string()),
                events.into_inner(),
                worktree_path.exists(),
                test_repo.open().local_branch_exists("topic"),
            ),
            (
                Some("Failed to run pre-session-start hook".to_string()),
                vec![HookInvocation {
                    name: "pre-session-start".to_string(),
                    env_vars: vec![(
                        EnvVars::session_cwd_name().to_string(),
                        worktree_path.to_string_lossy().to_string(),
                    )],
                }],
                true,
                true,
            ),
        );
    }

    #[test]
    fn worktree_window_receives_selected_profile_type_settings() {
        let config: Config = serde_yaml::from_str(indoc! {"
            agent:
              active_profile: selected-profile
              profiles:
                selected-profile:
                  review-type:
                    engine: codex
                    model: model-ember
                    reasoning_effort: high
        "})
        .unwrap();
        let common = super::super::CommonNewArgs {
            prompt: None,
            child_launch: None,
            label: None,
            kind: Some("other-kind".to_string()),
            type_name: Some("review-type".to_string()),
            model: None,
            parent_session_id: None,
            repo: None,
            engine: None,
            reasoning_effort: None,
        };
        let launch_options =
            super::super::launch_options::resolve_launch_options(&common, &config).unwrap();
        let worktree_dir = Path::new("/example/worktrees/review");
        let env_vars: &[(&str, &str)] = &[];
        let spec = worktree_window_spec(WorktreeWindowContext {
            repo_root: "/example/repo",
            worktree_name: "review",
            worktree_dir,
            config: &config,
            launch_options: &launch_options,
            prompt: None,
            env_vars,
            background: false,
        });

        assert_eq!(
            (spec.engine, spec.model, spec.reasoning_effort),
            (
                Engine::Codex,
                Some("model-ember"),
                Some(ReasoningEffort::High)
            ),
        );
    }

    #[rstest]
    fn runs_session_hook_after_worktree_hook_with_absolute_cwd(worktree_dir: tempfile::TempDir) {
        let worktree_path = worktree_dir.path().canonicalize().unwrap();
        let events = RefCell::new(Vec::new());
        let result = run_worktree_hooks(
            WorktreeHookContext {
                worktree_dir: Some(&worktree_path),
                actual_branch: "sample-branch",
                repo_root: "/repo",
            },
            |name, env_vars| {
                events.borrow_mut().push(capture_hook(name, env_vars));
                Ok(())
            },
            |_| {},
        );

        assert_eq!(
            (result.is_ok(), events.into_inner()),
            (
                true,
                vec![
                    HookInvocation {
                        name: "post-worktree-create".to_string(),
                        env_vars: vec![
                            (
                                EnvVars::worktree_path_name().to_string(),
                                worktree_path.to_string_lossy().to_string(),
                            ),
                            (
                                EnvVars::branch_name_name().to_string(),
                                "sample-branch".to_string()
                            ),
                            (EnvVars::repo_root_name().to_string(), "/repo".to_string()),
                        ],
                    },
                    HookInvocation {
                        name: "pre-session-start".to_string(),
                        env_vars: vec![(
                            EnvVars::session_cwd_name().to_string(),
                            worktree_path.to_string_lossy().to_string(),
                        )],
                    },
                ],
            ),
        );
    }

    #[rstest]
    #[case::post_worktree_create(
        "post-worktree-create",
        &["post-worktree-create"],
        "Failed to run post-worktree-create hook"
    )]
    #[case::pre_session_start(
        "pre-session-start",
        &["post-worktree-create", "pre-session-start"],
        "Failed to run pre-session-start hook"
    )]
    fn hook_failure_invokes_rollback_before_the_next_hook(
        #[case] failed_hook: &str,
        #[case] expected_hook_calls: &[&str],
        #[case] expected_error: &str,
        worktree_dir: tempfile::TempDir,
    ) {
        let worktree_path = worktree_dir.path().canonicalize().unwrap();
        let events = RefCell::new(Vec::new());
        let rollback_call = RefCell::new(None);
        let result = run_worktree_hooks(
            WorktreeHookContext {
                worktree_dir: Some(&worktree_path),
                actual_branch: "sample-branch",
                repo_root: "/repo",
            },
            |name, _env_vars| {
                events.borrow_mut().push(name.to_string());
                if name == failed_hook {
                    anyhow::bail!("hook failed")
                }
                Ok(())
            },
            |hook_name| {
                rollback_call.replace(Some(hook_name.to_string()));
            },
        );

        assert_eq!(
            (
                result.err().map(|error| error.to_string()),
                events.into_inner(),
                rollback_call.into_inner(),
            ),
            (
                Some(expected_error.to_string()),
                expected_hook_calls
                    .iter()
                    .map(|name| name.to_string())
                    .collect(),
                Some(failed_hook.to_string()),
            ),
        );
    }

    #[test]
    fn skip_hooks_skips_worktree_hook_chain() {
        let events = RefCell::new(Vec::new());
        let result = run_worktree_hooks(
            WorktreeHookContext {
                worktree_dir: None,
                actual_branch: "sample-branch",
                repo_root: "/repo",
            },
            |name, _| {
                events.borrow_mut().push(name.to_string());
                Ok(())
            },
            |_| events.borrow_mut().push("rollback".to_string()),
        );

        assert_eq!(
            (result.is_ok(), events.into_inner()),
            (true, Vec::<String>::new()),
        );
    }
}
