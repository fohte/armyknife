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

enum WorktreeHookMode<'a> {
    Skipped,
    Created(&'a Path),
    Reused(&'a Path),
}

struct WorktreeHookContext<'a> {
    mode: WorktreeHookMode<'a>,
    actual_branch: &'a str,
    repo_root: &'a str,
}

fn run_worktree_hooks(
    context: WorktreeHookContext<'_>,
    mut run_hook: impl FnMut(&str, &[(&str, &str)]) -> Result<()>,
    rollback: impl FnOnce(&str),
) -> Result<()> {
    let WorktreeHookContext {
        mode,
        actual_branch,
        repo_root,
    } = context;

    let worktree_dir = match mode {
        WorktreeHookMode::Skipped => {
            eprintln!("Skipping post-worktree-create hook (--skip-hooks)");
            return run_pre_session_start_hook(None, run_hook);
        }
        WorktreeHookMode::Reused(worktree_dir) => {
            let worktree_abs =
                std::fs::canonicalize(worktree_dir).unwrap_or_else(|_| worktree_dir.to_path_buf());
            return run_pre_session_start_hook(Some(&worktree_abs), run_hook);
        }
        WorktreeHookMode::Created(worktree_dir) => worktree_dir,
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

enum WorktreeAction {
    Force,
    Reuse(LinkedWorktree),
    ExistingBranch(String),
    NewBranch(String),
}

fn select_worktree_action(
    repo: &crate::infra::git::GitRepo,
    name: &str,
    branch_prefix: &str,
    force: bool,
) -> Result<WorktreeAction> {
    if force {
        return Ok(WorktreeAction::Force);
    }

    let branch = if repo_branch_exists(repo, name) {
        Some(name.to_string())
    } else {
        let name_no_prefix = name.strip_prefix(branch_prefix).unwrap_or(name);
        let prefixed_branch = format!("{branch_prefix}{name_no_prefix}");
        repo_branch_exists(repo, &prefixed_branch).then_some(prefixed_branch)
    };

    if let Some(branch) = branch {
        if let Some(worktree) = list_linked_worktrees(repo)?
            .into_iter()
            .find(|worktree| worktree.branch == branch)
        {
            return Ok(WorktreeAction::Reuse(worktree));
        }
        return Ok(WorktreeAction::ExistingBranch(branch));
    }

    let name_no_prefix = name.strip_prefix(branch_prefix).unwrap_or(name);
    Ok(WorktreeAction::NewBranch(format!(
        "{branch_prefix}{name_no_prefix}"
    )))
}

fn worktree_hook_mode(skip_hooks: bool, reused: bool, worktree_dir: &Path) -> WorktreeHookMode<'_> {
    if skip_hooks {
        WorktreeHookMode::Skipped
    } else if reused {
        WorktreeHookMode::Reused(worktree_dir)
    } else {
        WorktreeHookMode::Created(worktree_dir)
    }
}

fn prompt_base_for_existing_branch(
    repo: &crate::infra::git::GitRepo,
    prompt: Option<&str>,
) -> Result<Option<String>> {
    prompt
        .map(|_| get_main_branch_for_repo(repo).map(|main_branch| format!("origin/{main_branch}")))
        .transpose()
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

    let action = select_worktree_action(&repo, name, branch_prefix, args.force)?;
    let reused_worktree = matches!(&action, WorktreeAction::Reuse(_));
    let mut selected_worktree_dir = worktree_dir.clone();
    let mut selected_worktree_name = worktree_name.clone();
    let (actual_branch, actual_base, branch_rollback) = match action {
        WorktreeAction::Force => {
            std::fs::create_dir_all(&worktrees_dir)
                .context("Failed to create worktrees directory")?;
            let main_branch = get_main_branch_for_repo(&repo)?;
            let base_branch = args
                .from
                .clone()
                .unwrap_or_else(|| format!("origin/{main_branch}"));
            let name_no_prefix = name.strip_prefix(branch_prefix).unwrap_or(name);
            let branch = format!("{branch_prefix}{name_no_prefix}");

            let branch_rollback = if repo.local_branch_exists(&branch) {
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

            (branch, Some(base_branch), branch_rollback)
        }
        WorktreeAction::Reuse(linked_worktree) => {
            selected_worktree_dir = linked_worktree.path;
            selected_worktree_name = linked_worktree.name;
            (
                linked_worktree.branch,
                prompt_base_for_existing_branch(&repo, prompt)?,
                BranchRollback::Keep,
            )
        }
        WorktreeAction::ExistingBranch(branch) => {
            std::fs::create_dir_all(&worktrees_dir)
                .context("Failed to create worktrees directory")?;
            add_worktree_for_branch(&repo, &worktree_dir, &branch)?;
            (
                branch,
                prompt_base_for_existing_branch(&repo, prompt)?,
                BranchRollback::Keep,
            )
        }
        WorktreeAction::NewBranch(branch) => {
            std::fs::create_dir_all(&worktrees_dir)
                .context("Failed to create worktrees directory")?;
            let main_branch = get_main_branch_for_repo(&repo)?;
            let base_branch = args
                .from
                .clone()
                .unwrap_or_else(|| format!("origin/{main_branch}"));
            git_worktree_add(
                &repo,
                &worktree_dir,
                WorktreeAddMode::NewBranch {
                    branch: &branch,
                    base: &base_branch,
                },
            )?;
            (branch, Some(base_branch), BranchRollback::Delete)
        }
    };

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

    // Hook failures roll back worktrees created by this invocation.
    let rollback = |failed_hook: &str| {
        rollback_worktree(
            &repo,
            &selected_worktree_name,
            &actual_branch,
            &branch_rollback,
            failed_hook,
        )
    };
    run_worktree_hooks(
        WorktreeHookContext {
            mode: worktree_hook_mode(args.skip_hooks, reused_worktree, &selected_worktree_dir),
            actual_branch: &actual_branch,
            repo_root,
        },
        hooks::run_hook,
        rollback,
    )?;

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

    #[fixture]
    fn test_repo() -> TestRepo {
        TestRepo::new()
    }

    #[fixture]
    fn launch_setup() -> (Config, ResolvedLaunchOptions) {
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
        (config, launch_options)
    }

    #[rstest]
    #[case::name_without_prefix("team-widget", "widget")]
    #[case::name_with_prefix("team-widget", "team-widget")]
    fn linked_worktree_selection_uses_its_path_for_the_tmux_window(
        #[case] branch: &str,
        #[case] requested_name: &str,
        test_repo: TestRepo,
        launch_setup: (Config, ResolvedLaunchOptions),
    ) {
        test_repo.create_worktree(branch);
        let repo = test_repo.open();

        let action = select_worktree_action(&repo, requested_name, "team-", false).unwrap();
        let selected = match action {
            WorktreeAction::Reuse(worktree) => worktree,
            _ => unreachable!(),
        };
        let (config, launch_options) = launch_setup;
        let repo_root = test_repo.path();
        let repo_root = repo_root.to_str().unwrap();
        let spec = worktree_window_spec(WorktreeWindowContext {
            repo_root,
            worktree_name: &selected.name,
            worktree_dir: &selected.path,
            config: &config,
            launch_options: &launch_options,
            prompt: None,
            env_vars: &[],
            background: false,
        });
        let expected_path = test_repo.worktree_path(branch);
        let expected_path_str = expected_path.to_string_lossy().to_string();

        assert_eq!(
            (
                selected.branch.clone(),
                selected.path.clone(),
                selected.name.clone(),
                spec.cwd.to_string(),
                spec.window_name.to_string(),
            ),
            (
                branch.to_string(),
                expected_path,
                branch.to_string(),
                expected_path_str,
                branch.to_string(),
            ),
        );
    }

    #[rstest]
    fn does_not_reuse_the_main_worktree_checkout(test_repo: TestRepo) {
        let repo = test_repo.open();
        let main_branch = repo.current_branch().unwrap();

        let action = select_worktree_action(&repo, &main_branch, "team-", false).unwrap();

        let selected_branch = match action {
            WorktreeAction::ExistingBranch(branch) => Some(branch),
            _ => None,
        };
        assert_eq!(selected_branch, Some(main_branch));
    }

    #[rstest]
    fn force_selects_creation_even_when_branch_has_a_linked_worktree(test_repo: TestRepo) {
        test_repo.create_worktree("team-topic");
        let repo = test_repo.open();

        let action = select_worktree_action(&repo, "topic", "team-", true).unwrap();

        let forced = match action {
            WorktreeAction::Force => Some(()),
            _ => None,
        };
        assert_eq!(forced, Some(()));
    }

    #[rstest]
    fn selects_a_new_prefixed_branch_when_no_candidate_exists(test_repo: TestRepo) {
        let repo = test_repo.open();

        let action = select_worktree_action(&repo, "topic", "team-", false).unwrap();

        let new_branch = match action {
            WorktreeAction::NewBranch(branch) => Some(branch),
            _ => None,
        };
        assert_eq!(new_branch, Some("team-topic".to_string()));
    }

    #[rstest]
    fn reused_worktree_runs_only_pre_session_hook_and_keeps_the_path_on_failure(
        test_repo: TestRepo,
    ) {
        test_repo.create_worktree("topic");
        let worktree_path = test_repo.worktree_path("topic");
        let repo = test_repo.open();
        let action = select_worktree_action(&repo, "topic", "team-", false).unwrap();
        let reused = matches!(action, WorktreeAction::Reuse(_));
        let events = RefCell::new(Vec::new());
        let rollback_events = RefCell::new(Vec::new());
        let repo_root = test_repo.path();
        let repo_root = repo_root.to_str().unwrap();

        let result = run_worktree_hooks(
            WorktreeHookContext {
                mode: worktree_hook_mode(false, reused, &worktree_path),
                actual_branch: "topic",
                repo_root,
            },
            |name, env_vars| {
                events.borrow_mut().push(capture_hook(name, env_vars));
                anyhow::bail!("hook failed")
            },
            |hook_name| rollback_events.borrow_mut().push(hook_name.to_string()),
        );

        assert_eq!(
            (
                result.err().map(|error| error.to_string()),
                events.into_inner(),
                rollback_events.into_inner(),
                worktree_path.exists(),
                repo.local_branch_exists("topic"),
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
                Vec::<String>::new(),
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
                mode: WorktreeHookMode::Created(&worktree_path),
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
                mode: WorktreeHookMode::Created(&worktree_path),
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
    fn skip_hooks_skips_worktree_hook_chain_and_rollback() {
        let events = RefCell::new(Vec::new());
        let worktree_path = Path::new("/unused/worktree");
        let result = run_worktree_hooks(
            WorktreeHookContext {
                mode: worktree_hook_mode(true, true, worktree_path),
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
