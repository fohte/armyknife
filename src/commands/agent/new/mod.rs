use anyhow::{Context, Result, bail};
use clap::Args;
use std::path::{Path, PathBuf};

use crate::commands::agent::types::{ChildLaunchPermission, Engine, ReasoningEffort};
use crate::commands::agent::{ensure_repo_vibe_access, is_vibe_enabled};
use crate::infra::git::{GitRepo, get_repo_root, github_owner_and_repo, open_repo, open_repo_at};
use crate::shared::config::{Config, LayoutNode, PaneConfig, load_config};
use crate::shared::env_var::EnvVars;
use crate::shared::hooks;

mod child_launch_guard;
mod launch_options;
mod profile_type;
mod prompt;
mod session_context;
mod session_metadata;
mod session_mode;
mod session_start_hook;
mod tmux;
mod worktree;
mod worktree_creation;

use child_launch_guard::ensure_caller_may_launch_children;
use launch_options::{resolve_launch_options, validate_type_selection};
use prompt::{delete_prompt_cache, resolve_args, save_prompt_cache};
use session_context::{build_ancestor_chain, resolve_prompt};
use session_mode::{caller_repo_root, should_open_window};
use session_start_hook::run_pre_session_start_hook;
use tmux::{TmuxSplitPaneSpec, TmuxWindowSpec, setup_split_pane, setup_tmux_window};
use worktree_creation::run_worktree_creation;

/// CLI args shared by `a agent new`'s worktree and no-worktree modes.
#[derive(Args, Clone, PartialEq, Eq)]
pub struct CommonNewArgs {
    /// Initial prompt to send to the agent (Claude Code or Codex).
    /// When provided without a branch name, the branch name is auto-generated from this prompt.
    #[arg(long)]
    pub prompt: Option<String>,

    /// Legacy compatibility flag. Denies child launches from the new session.
    #[arg(long, hide = true)]
    pub agent: bool,

    /// Whether the new session may launch child sessions (default: allow).
    #[arg(long, value_enum)]
    pub child_launch: Option<ChildLaunchPermission>,

    /// Label for the new session (displayed in agent watch).
    /// When not specified, the session will get its label via the
    /// user-prompt-submit hook (auto-generation from prompt).
    #[arg(long)]
    pub label: Option<String>,

    /// Initial work type for the new session, using the workflow skill name.
    /// A matching `agent.work_types` entry controls its display metadata.
    #[arg(long)]
    pub kind: Option<String>,

    /// Launch settings type from the active `agent.profiles` profile.
    /// Explicit engine, model, and reasoning effort flags override its values.
    #[arg(long = "type")]
    pub type_name: Option<String>,

    /// Model for the new session. Passed through to `<engine> --model`.
    /// Accepts an alias or a full model name. With `--type`, defaults to its
    /// model; otherwise falls back to the selected engine's kind profile, then
    /// Codex's global `agent.codex.model` when the selected engine is Codex.
    #[arg(long)]
    pub model: Option<String>,

    /// Parent session ID for tree view hierarchy.
    /// Sets ARMYKNIFE_ANCESTOR_SESSION_IDS for the child session.
    #[arg(long)]
    pub parent_session_id: Option<String>,

    /// Path to the target repository or linked worktree.
    /// Without --worktree, the session runs from the root of the worktree containing this path.
    /// With --worktree, new worktrees are created from the main repository root.
    #[arg(short = 'R', long)]
    pub repo: Option<PathBuf>,

    /// Coding agent CLI to launch for the new session. With `--type`, defaults
    /// to its engine; otherwise falls back to `agent.launch_profiles.<kind>.engine`,
    /// then `agent.default_engine` (default: `claude`).
    /// A different engine with `--type` requires an explicit `--model`.
    ///
    /// With `--worktree`, `config.agent.worktree.layout` panes running `claude` are
    /// replaced by this engine's CLI (without their arguments) unless the
    /// layout already has a pane for it; other panes are left as written.
    #[arg(long, value_enum)]
    pub engine: Option<Engine>,

    /// Reasoning effort for the new session. Passed as `claude --effort`.
    /// For Codex, sent in the first `turn/start` on the daemon route and as
    /// `-c model_reasoning_effort=...` otherwise. Falls back to the selected
    /// With `--type`, defaults to its reasoning effort. Otherwise falls back
    /// to the selected engine's kind profile, then Codex's global
    /// `agent.codex.reasoning_effort` when the selected engine is Codex.
    #[arg(long, value_enum)]
    pub reasoning_effort: Option<ReasoningEffort>,
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NewArgs {
    /// Create a worktree for the branch and run the session there in a new
    /// tmux window (existing branch will be checked out, non-existing
    /// branch will be created with fohte/ prefix). Branch name is optional:
    /// when omitted, it is auto-generated from --prompt. When the flag
    /// itself is omitted entirely, no worktree is created; instead the
    /// session runs in the current directory (or the root of the worktree
    /// containing -R when given). If that target is the same repo as the invoking Claude
    /// Code session's, this splits the tmux pane that invoked this command
    /// (requires running inside tmux); otherwise it opens a new tmux window
    /// in the target repo's own tmux session.
    #[arg(long, num_args = 0..=1, require_equals = true)]
    pub worktree: Option<Option<String>>,

    /// Base branch for new branch creation (requires --worktree;
    /// default: origin/main or origin/master)
    #[arg(long, requires = "worktree")]
    pub from: Option<String>,

    /// Force create new branch even if it already exists (requires --worktree)
    #[arg(long, requires = "worktree")]
    pub force: bool,

    /// Require vibe mode in the current or an ancestor session and an allowed target repository.
    /// This flag does not enable vibe mode.
    #[arg(long)]
    pub vibe: bool,

    #[command(flatten)]
    pub common: CommonNewArgs,

    /// Skip post-worktree-create and pre-session-start hooks.
    #[arg(long)]
    pub skip_hooks: bool,

    /// Print the selected profile and type settings without launching a session.
    #[arg(long, requires = "type_name")]
    pub dry_run: bool,
}

pub fn run(args: &NewArgs) -> Result<()> {
    profile_type::dispatch(args, profile_type::run_dry_run, run_regular)
}

fn run_regular(args: &NewArgs) -> Result<()> {
    ensure_caller_may_launch_children()?;
    run_with_vibe_access(
        args,
        is_vibe_enabled,
        |args| ensure_target_repo_vibe_access(args, resolve_target_repo_identity, load_config),
        || run_inner(args),
    )
}

fn run_with_vibe_access(
    args: &NewArgs,
    vibe_check: impl FnOnce() -> Result<bool>,
    repo_access_check: impl FnOnce(&NewArgs) -> Result<()>,
    launch: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if args.vibe {
        ensure_vibe_access(vibe_check()?)?;
        repo_access_check(args)?;
    }

    launch()
}

fn ensure_target_repo_vibe_access(
    args: &NewArgs,
    resolve_repo_identity: impl FnOnce(Option<&Path>) -> Result<(String, String)>,
    load_config: impl FnOnce() -> Result<Config>,
) -> Result<()> {
    let (owner, repo_name) = resolve_repo_identity(args.common.repo.as_deref())?;
    ensure_repo_vibe_access(&load_config()?, &owner, &repo_name)
}

fn resolve_target_repo_identity(repo_path: Option<&Path>) -> Result<(String, String)> {
    let repo = match repo_path {
        Some(path) => open_repo_at(path)?,
        None => open_repo()?,
    };
    github_owner_and_repo(&repo)
}

fn ensure_vibe_access(vibe_enabled: bool) -> Result<()> {
    if !vibe_enabled {
        bail!("Vibe mode must be enabled to use --vibe.");
    }

    Ok(())
}

fn run_inner(args: &NewArgs) -> Result<()> {
    let config = load_config()?;

    validate_type_selection(&args.common, &config)?;

    let repo = match &args.common.repo {
        Some(path) => Some(open_repo_at(path)?),
        None => None,
    };
    let repo_root = match &repo {
        Some(repo) => repo.main_workdir_string()?,
        None => get_repo_root()?,
    };

    match &args.worktree {
        Some(worktree_value) => {
            run_worktree_mode(args, worktree_value.as_deref(), &repo_root, &config)
        }
        None => run_session_only(args, repo.as_ref(), &repo_root, &config),
    }
}

fn run_worktree_mode(
    args: &NewArgs,
    worktree_value: Option<&str>,
    repo_root: &str,
    config: &Config,
) -> Result<()> {
    let resolved = resolve_args(worktree_value, args.common.prompt.as_deref())?;
    let name = resolved.branch_name;
    let prompt = resolved.prompt;

    with_prompt_cache_recovery(repo_root, prompt.as_deref(), || {
        run_worktree_creation(args, &name, prompt.as_deref(), repo_root, config)
    })
}

/// Save `prompt` to the cache before running `f`, then clean it up on
/// success or report its location on failure. Shared by the worktree and
/// no-worktree paths so `--prompt` recovery doesn't diverge between them.
fn with_prompt_cache_recovery(
    repo_root: &str,
    prompt: Option<&str>,
    f: impl FnOnce() -> Result<()>,
) -> Result<()> {
    with_prompt_cache_recovery_with_deps(
        repo_root,
        prompt,
        f,
        save_prompt_cache,
        delete_prompt_cache,
    )
}

/// Internal implementation that accepts the cache save/delete functions as
/// dependencies. Allows testing the recovery control flow without touching
/// the real cache directory on disk.
fn with_prompt_cache_recovery_with_deps(
    repo_root: &str,
    prompt: Option<&str>,
    f: impl FnOnce() -> Result<()>,
    save: impl FnOnce(&str, &str) -> Result<PathBuf>,
    delete: impl FnOnce(&str),
) -> Result<()> {
    let prompt_cache_path = prompt.map(|p| save(repo_root, p)).transpose()?;

    let result = f();

    if result.is_ok() {
        delete(repo_root);
    } else if let Some(path) = prompt_cache_path {
        eprintln!("Prompt saved to: {}", path.display());
    }

    result
}

/// Reads the parent process's `TQ_SESSION_ID` (exported by tq's own
/// SessionStart hook into `CLAUDE_ENV_FILE`, mirroring how armyknife exports
/// `ARMYKNIFE_SESSION_ID`) and returns it as the `TQ_PARENT_SESSION_ID` pair
/// for the child session. `None` when the parent has no tq hook installed.
fn tq_parent_session_env_var() -> Option<(String, String)> {
    std::env::var("TQ_SESSION_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|id| ("TQ_PARENT_SESSION_ID".to_string(), id))
}

/// Build tmux session-level env vars for the child session's launch metadata,
/// ancestor-session-id chain, and tq parent session ID. Shared between the
/// worktree and no-worktree flows.
fn build_env_vars(
    common: &CommonNewArgs,
    vibe: bool,
    own_session_id: Option<String>,
) -> Result<Vec<(String, String)>> {
    let mut env_vars: Vec<(String, String)> = Vec::new();
    if let Some(ref label) = common.label {
        env_vars.push((EnvVars::session_label_name().to_string(), label.clone()));
    }
    if let Some(ref kind) = common.kind {
        env_vars.push((EnvVars::session_work_type_name().to_string(), kind.clone()));
    }
    let child_launch = if common.agent {
        ChildLaunchPermission::Deny
    } else {
        common.child_launch.unwrap_or_default()
    };
    env_vars.push((
        EnvVars::session_child_launch_name().to_string(),
        child_launch.as_env_str().to_string(),
    ));
    if common.agent {
        env_vars.push((
            EnvVars::session_delegated_name().to_string(),
            "1".to_string(),
        ));
    }
    if vibe {
        env_vars.push((
            EnvVars::session_vibe_launch_name().to_string(),
            "1".to_string(),
        ));
    }
    // Resolve parent session ID: explicit flag > ARMYKNIFE_SESSION_ID env var.
    // ARMYKNIFE_SESSION_ID is set by the SessionStart hook via CLAUDE_ENV_FILE,
    // so `a agent new` called from a Claude Code Bash tool automatically inherits
    // the parent session ID without requiring --parent-session-id. Codex has no
    // env file, so it falls back to CODEX_SESSION_ID (see `own_session_id`).
    let parent_id = common.parent_session_id.clone().or(own_session_id);
    if let Some(ref parent_id) = parent_id {
        let ancestor_chain = build_ancestor_chain(parent_id)?;
        env_vars.push((
            EnvVars::ancestor_session_ids_name().to_string(),
            ancestor_chain,
        ));
    }
    if let Some(pair) = tq_parent_session_env_var() {
        env_vars.push(pair);
    }
    Ok(env_vars)
}

/// Build the env vars and background flag shared by both the worktree and
/// no-worktree tmux launch paths.
fn tmux_launch_inputs(common: &CommonNewArgs, vibe: bool) -> Result<(Vec<(String, String)>, bool)> {
    let own_session_id = EnvVars::load().own_session_id();
    // Agent tools can invoke this command without a tmux context, so preserve the user's focus.
    let background = std::env::var("CLAUDECODE").is_ok() || own_session_id.is_some();
    let env_vars = build_env_vars(common, vibe, own_session_id)?;
    Ok((env_vars, background))
}

/// Run `a agent new` without creating a worktree, either by splitting the
/// caller's pane or opening a window in the target repo's session (see
/// `should_open_window`).
fn run_session_only(
    args: &NewArgs,
    repo: Option<&GitRepo>,
    repo_root: &str,
    config: &Config,
) -> Result<()> {
    let raw_prompt = args.common.prompt.as_deref();

    with_prompt_cache_recovery(repo_root, raw_prompt, || {
        run_session_only_inner(args, repo, repo_root, config)
    })
}

fn run_session_only_inner(
    args: &NewArgs,
    repo: Option<&GitRepo>,
    repo_root: &str,
    config: &Config,
) -> Result<()> {
    let current_dir = std::env::current_dir()
        .context("Failed to get current directory")?
        .to_string_lossy()
        .to_string();
    let caller_repo_root = caller_repo_root(&current_dir);

    run_session_only_inner_with_launchers(
        args,
        SessionOnlyRunContext {
            repo,
            repo_root,
            caller_repo_root: &caller_repo_root,
            current_dir: &current_dir,
            config,
        },
        crate::shared::caller_pane::resolve_caller_pane_id,
        |spec| setup_split_pane(spec).map(|route| route.display_suffix()),
        |spec, config| setup_tmux_window(spec, config).map(|route| route.display_suffix()),
        hooks::run_hook,
    )
}

struct SessionOnlyRunContext<'a> {
    repo: Option<&'a GitRepo>,
    repo_root: &'a str,
    caller_repo_root: &'a str,
    current_dir: &'a str,
    config: &'a Config,
}

fn run_session_only_inner_with_launchers(
    args: &NewArgs,
    context: SessionOnlyRunContext<'_>,
    resolve_caller_pane: impl FnOnce() -> Option<String>,
    launch_split: impl FnOnce(TmuxSplitPaneSpec<'_>) -> Result<String>,
    launch_window: impl FnOnce(TmuxWindowSpec<'_>, &Config) -> Result<String>,
    mut run_hook: impl FnMut(&str, &[(&str, &str)]) -> Result<()>,
) -> Result<()> {
    let cwd = context
        .repo
        .map(GitRepo::workdir_string)
        .unwrap_or_else(|| context.current_dir.to_string());

    let prompt = resolve_prompt(
        args.common.prompt.as_deref(),
        None,
        None,
        context.current_dir,
        &cwd,
    );

    let (env_vars, background) = tmux_launch_inputs(&args.common, args.vibe)?;
    let env_refs: Vec<(&str, &str)> = env_vars
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let background_suffix = if background { " (background)" } else { "" };
    let launch_options = resolve_launch_options(&args.common, context.config)?;

    let differs = should_open_window(
        context.repo_root,
        context.caller_repo_root,
        &context.config.agent.worktree.dir,
    );

    let caller_pane = resolve_caller_pane();
    let session_cwd = if args.skip_hooks {
        None
    } else {
        Some(std::fs::canonicalize(&cwd).context("Failed to resolve session CWD")?)
    };
    run_pre_session_start_hook(session_cwd.as_deref(), &mut run_hook)?;

    // Window mode doesn't need a pane, so it remains the fallback when no
    // caller pane can be resolved.
    match caller_pane {
        Some(target_pane) if !differs => {
            let route_suffix = launch_split(TmuxSplitPaneSpec {
                target_pane: &target_pane,
                cwd: &cwd,
                model: launch_options.model.as_deref(),
                reasoning_effort: launch_options.reasoning_effort,
                prompt: prompt.as_deref(),
                engine: launch_options.engine,
                env_vars: &env_refs,
                background,
            })?;
            println!("Split tmux pane in '{cwd}'{background_suffix}{route_suffix}");
        }
        _ => {
            // PID-based placeholder: there's no worktree/branch name to use
            // here, and `restore_automatic_rename` below lets tmux relabel
            // the window once the engine's process starts instead of keeping
            // this name displayed.
            let window_name = format!(
                "{}-{}",
                launch_options.engine.process_name(),
                std::process::id()
            );
            let layout = LayoutNode::Pane(PaneConfig {
                command: launch_options.engine.process_name().to_string(),
                focus: true,
            });

            let route_suffix = launch_window(
                TmuxWindowSpec {
                    repo_root: context.repo_root,
                    cwd: &cwd,
                    window_name: &window_name,
                    layout: &layout,
                    model: launch_options.model.as_deref(),
                    reasoning_effort: launch_options.reasoning_effort,
                    prompt: prompt.as_deref(),
                    engine: launch_options.engine,
                    env_vars: &env_refs,
                    background,
                    restore_automatic_rename: true,
                },
                context.config,
            )?;
            println!("Opened tmux window in '{cwd}'{background_suffix}{route_suffix}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::TestRepo;
    use clap::{CommandFactory, Parser};
    use rstest::{fixture, rstest};
    use std::cell::{Cell, RefCell};

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        args: NewArgs,
    }

    #[rstest]
    #[case::explicit_value(&["a", "--worktree=my-branch"], Some(Some("my-branch")))]
    #[case::value_omitted(&["a", "--worktree"], Some(None))]
    #[case::flag_omitted(&["a"], None)]
    fn worktree_value_parses(#[case] argv: &[&str], #[case] expected: Option<Option<&str>>) {
        let cli = TestCli::try_parse_from(argv).unwrap();
        let actual = cli.args.worktree.as_ref().map(|inner| inner.as_deref());
        assert_eq!(actual, expected);
    }

    struct SessionOnlyRepoFixture {
        _repo: TestRepo,
        main_root: PathBuf,
        main_subdirectory: PathBuf,
        linked_worktree_root: PathBuf,
        linked_worktree_subdirectory: PathBuf,
    }

    #[fixture]
    fn session_only_repo() -> SessionOnlyRepoFixture {
        let repo = TestRepo::new();
        let main_root = repo.path();
        let main_subdirectory = main_root.join("subdirectory");
        std::fs::create_dir_all(&main_subdirectory).unwrap();

        repo.create_worktree("topic");
        let linked_worktree_root = repo.worktree_path("topic");
        let linked_worktree_subdirectory = linked_worktree_root.join("subdirectory");
        std::fs::create_dir_all(&linked_worktree_subdirectory).unwrap();

        SessionOnlyRepoFixture {
            _repo: repo,
            main_root,
            main_subdirectory,
            linked_worktree_root,
            linked_worktree_subdirectory,
        }
    }

    struct SessionOnlyRootFixture {
        _directory: tempfile::TempDir,
        cwd: String,
        config: Config,
    }

    impl SessionOnlyRootFixture {
        fn context(&self) -> SessionOnlyRunContext<'_> {
            SessionOnlyRunContext {
                repo: None,
                repo_root: &self.cwd,
                caller_repo_root: &self.cwd,
                current_dir: &self.cwd,
                config: &self.config,
            }
        }
    }

    #[fixture]
    fn session_only_root() -> SessionOnlyRootFixture {
        let directory = tempfile::tempdir().unwrap();
        let cwd = directory
            .path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();

        SessionOnlyRootFixture {
            _directory: directory,
            cwd,
            config: Config::default(),
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum CapturedSessionOnlyLaunch {
        Split {
            cwd: String,
            engine: Engine,
            model: Option<String>,
            reasoning_effort: Option<ReasoningEffort>,
        },
        Window {
            repo_root: String,
            cwd: String,
            engine: Engine,
            model: Option<String>,
            reasoning_effort: Option<ReasoningEffort>,
        },
    }

    struct ExpectedProfileLaunch {
        engine: Engine,
        model: &'static str,
        reasoning_effort: ReasoningEffort,
        engine_source: &'static str,
        model_source: &'static str,
        reasoning_effort_source: &'static str,
    }

    fn capture_session_only_launch(
        args: &NewArgs,
        context: SessionOnlyRunContext<'_>,
        caller_pane: Option<&str>,
    ) -> Option<CapturedSessionOnlyLaunch> {
        let actual = RefCell::new(None);

        run_session_only_inner_with_launchers(
            args,
            context,
            || caller_pane.map(str::to_string),
            |spec| {
                actual.replace(Some(CapturedSessionOnlyLaunch::Split {
                    cwd: spec.cwd.to_string(),
                    engine: spec.engine,
                    model: spec.model.map(str::to_string),
                    reasoning_effort: spec.reasoning_effort,
                }));
                Ok(String::new())
            },
            |spec, _| {
                actual.replace(Some(CapturedSessionOnlyLaunch::Window {
                    repo_root: spec.repo_root.to_string(),
                    cwd: spec.cwd.to_string(),
                    engine: spec.engine,
                    model: spec.model.map(str::to_string),
                    reasoning_effort: spec.reasoning_effort,
                }));
                Ok(String::new())
            },
            |_, _| Ok(()),
        )
        .unwrap();

        actual.into_inner()
    }

    #[rstest]
    fn session_only_runs_pre_start_hook_with_absolute_cwd_before_launch(
        session_only_root: SessionOnlyRootFixture,
    ) {
        let args = TestCli::try_parse_from(["a"]).unwrap().args;
        let events = RefCell::new(Vec::new());

        let result = run_session_only_inner_with_launchers(
            &args,
            session_only_root.context(),
            || Some("%caller-pane".to_string()),
            |_| {
                events.borrow_mut().push("launch".to_string());
                Ok(String::new())
            },
            |_, _| {
                events.borrow_mut().push("launch".to_string());
                Ok(String::new())
            },
            |name, env_vars| {
                let env_vars = env_vars
                    .iter()
                    .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                    .collect::<Vec<_>>();
                events.borrow_mut().push(format!("{name}:{env_vars:?}"));
                Ok(())
            },
        );

        assert_eq!(
            (result.is_ok(), events.into_inner()),
            (
                true,
                vec![
                    format!(
                        "pre-session-start:[(\"{}\", \"{}\")]",
                        EnvVars::session_cwd_name(),
                        session_only_root.cwd
                    ),
                    "launch".to_string(),
                ],
            ),
        );
    }

    #[rstest]
    fn session_only_hook_failure_prevents_launch(session_only_root: SessionOnlyRootFixture) {
        let args = TestCli::try_parse_from(["a"]).unwrap().args;
        let launched = Cell::new(false);

        let result = run_session_only_inner_with_launchers(
            &args,
            session_only_root.context(),
            || Some("%caller-pane".to_string()),
            |_| {
                launched.set(true);
                Ok(String::new())
            },
            |_, _| {
                launched.set(true);
                Ok(String::new())
            },
            |_, _| anyhow::bail!("hook failed"),
        );

        assert_eq!(
            (result.err().map(|error| error.to_string()), launched.get()),
            (
                Some("Failed to run pre-session-start hook".to_string()),
                false
            ),
        );
    }

    #[rstest]
    fn skip_hooks_skips_pre_start_hook_without_worktree(session_only_root: SessionOnlyRootFixture) {
        let args = TestCli::try_parse_from(["a", "--skip-hooks"]).unwrap().args;
        let events = RefCell::new(Vec::new());

        let result = run_session_only_inner_with_launchers(
            &args,
            session_only_root.context(),
            || Some("%caller-pane".to_string()),
            |_| {
                events.borrow_mut().push("launch".to_string());
                Ok(String::new())
            },
            |_, _| {
                events.borrow_mut().push("launch".to_string());
                Ok(String::new())
            },
            |name, _| {
                events.borrow_mut().push(name.to_string());
                Ok(())
            },
        );

        assert_eq!(
            (result.is_ok(), events.into_inner()),
            (true, vec!["launch".to_string()]),
        );
    }

    #[rstest]
    fn skip_hooks_does_not_resolve_session_cwd(session_only_root: SessionOnlyRootFixture) {
        let args = TestCli::try_parse_from(["a", "--skip-hooks"]).unwrap().args;
        let missing_cwd = Path::new(&session_only_root.cwd)
            .join("missing")
            .to_string_lossy()
            .to_string();
        let context = SessionOnlyRunContext {
            repo: None,
            repo_root: &missing_cwd,
            caller_repo_root: &missing_cwd,
            current_dir: &missing_cwd,
            config: &session_only_root.config,
        };
        let events = RefCell::new(Vec::new());

        let result = run_session_only_inner_with_launchers(
            &args,
            context,
            || Some("%caller-pane".to_string()),
            |_| {
                events.borrow_mut().push("launch".to_string());
                Ok(String::new())
            },
            |_, _| {
                events.borrow_mut().push("launch".to_string());
                Ok(String::new())
            },
            |name, _| {
                events.borrow_mut().push(name.to_string());
                Ok(())
            },
        );

        assert_eq!(
            (result.is_ok(), events.into_inner()),
            (true, vec!["launch".to_string()]),
        );
    }

    #[rstest]
    #[case::main_checkout_window(false, None, ".worktrees", false)]
    #[case::linked_worktree_window(true, None, ".worktrees", false)]
    #[case::main_checkout_split(false, Some("%caller-pane"), ".worktrees", true)]
    #[case::linked_worktree_split_when_worktrees_dir_differs(
        true,
        Some("%caller-pane"),
        "configured-worktrees",
        true
    )]
    fn repo_argument_launches_from_containing_checkout(
        #[case] linked_worktree: bool,
        #[case] caller_pane: Option<&str>,
        #[case] worktrees_dir: &str,
        #[case] split: bool,
        session_only_repo: SessionOnlyRepoFixture,
    ) {
        let target_path = if linked_worktree {
            &session_only_repo.linked_worktree_subdirectory
        } else {
            &session_only_repo.main_subdirectory
        };
        let expected_cwd = if linked_worktree {
            session_only_repo
                .linked_worktree_root
                .to_string_lossy()
                .to_string()
        } else {
            session_only_repo.main_root.to_string_lossy().to_string()
        };
        let repo = open_repo_at(target_path).unwrap();
        let repo_root = repo.main_workdir_string().unwrap();
        let caller_repo_root = session_only_repo.main_root.to_string_lossy().to_string();
        let current_dir = caller_repo_root.clone();
        let mut config = Config::default();
        config.agent.worktree.dir = worktrees_dir.to_string();
        let cli = TestCli::try_parse_from(["a", "-R", target_path.to_str().unwrap()]).unwrap();

        let actual = capture_session_only_launch(
            &cli.args,
            SessionOnlyRunContext {
                repo: Some(&repo),
                repo_root: &repo_root,
                caller_repo_root: &caller_repo_root,
                current_dir: &current_dir,
                config: &config,
            },
            caller_pane,
        );

        let expected = if split {
            CapturedSessionOnlyLaunch::Split {
                cwd: expected_cwd,
                engine: Engine::Claude,
                model: None,
                reasoning_effort: None,
            }
        } else {
            CapturedSessionOnlyLaunch::Window {
                repo_root: session_only_repo.main_root.to_string_lossy().to_string(),
                cwd: expected_cwd,
                engine: Engine::Claude,
                model: None,
                reasoning_effort: None,
            }
        };

        assert_eq!(actual, Some(expected),);
    }

    #[rstest]
    #[case::profile_values(
        &["a", "--kind", "other-kind", "--type", "review-type"],
        ExpectedProfileLaunch {
            engine: Engine::Codex,
            model: "model-ember",
            reasoning_effort: ReasoningEffort::High,
            engine_source: "agent.profiles[\"sample-profile\"][\"review-type\"].engine",
            model_source: "agent.profiles[\"sample-profile\"][\"review-type\"].model",
            reasoning_effort_source: "agent.profiles[\"sample-profile\"][\"review-type\"].reasoning_effort",
        }
    )]
    #[case::model_and_effort_overrides(
        &["a", "--type", "review-type", "--model", "model-raven", "--reasoning-effort", "low"],
        ExpectedProfileLaunch {
            engine: Engine::Codex,
            model: "model-raven",
            reasoning_effort: ReasoningEffort::Low,
            engine_source: "agent.profiles[\"sample-profile\"][\"review-type\"].engine",
            model_source: "--model",
            reasoning_effort_source: "--reasoning-effort",
        }
    )]
    #[case::engine_and_model_overrides(
        &["a", "--type", "review-type", "--engine", "claude", "--model", "model-raven"],
        ExpectedProfileLaunch {
            engine: Engine::Claude,
            model: "model-raven",
            reasoning_effort: ReasoningEffort::High,
            engine_source: "--engine",
            model_source: "--model",
            reasoning_effort_source: "agent.profiles[\"sample-profile\"][\"review-type\"].reasoning_effort",
        }
    )]
    fn profile_type_settings_reach_launcher_and_dry_run_with_kind_ignored(
        #[case] argv: &[&str],
        #[case] expected: ExpectedProfileLaunch,
        session_only_root: SessionOnlyRootFixture,
    ) {
        let config: Config = serde_yaml::from_str(indoc::indoc! {
            "
            agent:
              active_profile: sample-profile
              codex:
                model: model-global
                reasoning_effort: max
              profiles:
                sample-profile:
                  review-type:
                    engine: codex
                    model: model-ember
                    reasoning_effort: high
            "
        })
        .unwrap();
        let args = TestCli::try_parse_from(argv).unwrap().args;
        let context = SessionOnlyRunContext {
            repo: None,
            repo_root: &session_only_root.cwd,
            caller_repo_root: &session_only_root.cwd,
            current_dir: &session_only_root.cwd,
            config: &config,
        };

        let launch = capture_session_only_launch(&args, context, Some("%caller-pane"));
        let resolved = config.agent.resolve_profile_type("review-type").unwrap();
        let launch_options = resolve_launch_options(&args.common, &config).unwrap();
        let dry_run = profile_type::format_dry_run(&args.common, &resolved, &launch_options);

        assert_eq!(
            (launch, dry_run),
            (
                Some(CapturedSessionOnlyLaunch::Split {
                    cwd: session_only_root.cwd,
                    engine: expected.engine,
                    model: Some(expected.model.to_string()),
                    reasoning_effort: Some(expected.reasoning_effort),
                }),
                format!(
                    "Resolved agent launch settings:\n  profile: sample-profile\n  type: review-type\n  engine: {}\n    source: {}\n  model: {}\n    source: {}\n  reasoning effort: {}\n    source: {}\n",
                    expected.engine.process_name(),
                    expected.engine_source,
                    expected.model,
                    expected.model_source,
                    expected.reasoning_effort.as_str(),
                    expected.reasoning_effort_source,
                ),
            ),
        );
    }

    #[rstest]
    fn omitting_type_keeps_kind_launch_profile_behavior() {
        let session_only_root = session_only_root();
        let config: Config = serde_yaml::from_str(indoc::indoc! {
            "
            agent:
              launch_profiles:
                build:
                  engine: codex
                  codex:
                    model: model-legacy
                    reasoning_effort: medium
            "
        })
        .unwrap();
        let args = TestCli::try_parse_from(["a", "--kind", "build"])
            .unwrap()
            .args;
        let context = SessionOnlyRunContext {
            repo: None,
            repo_root: &session_only_root.cwd,
            caller_repo_root: &session_only_root.cwd,
            current_dir: &session_only_root.cwd,
            config: &config,
        };

        let launch = capture_session_only_launch(&args, context, Some("%caller-pane"));

        assert_eq!(
            launch,
            Some(CapturedSessionOnlyLaunch::Split {
                cwd: session_only_root.cwd,
                engine: Engine::Codex,
                model: Some("model-legacy".to_string()),
                reasoning_effort: Some(ReasoningEffort::Medium),
            }),
        );
    }

    #[test]
    fn repo_help_describes_session_only_worktree_resolution() {
        let command = TestCli::command();
        let actual = command
            .get_arguments()
            .find(|arg| arg.get_id() == "repo")
            .map(|arg| {
                (
                    arg.get_short(),
                    arg.get_long(),
                    arg.get_help().map(ToString::to_string),
                )
            });

        assert_eq!(
            actual,
            Some((
                Some('R'),
                Some("repo"),
                Some(
                    "Path to the target repository or linked worktree. Without --worktree, the session runs from the root of the worktree containing this path. With --worktree, new worktrees are created from the main repository root".to_string(),
                ),
            )),
        );
    }

    #[rstest]
    #[case::without_vibe_flag(&["a"], Ok(false), Ok(()), Ok(()), false, false, true)]
    #[case::vibe_enabled(&["a", "--vibe"], Ok(true), Ok(()), Ok(()), true, true, true)]
    #[case::repo_not_allowed(
        &["a", "--vibe"],
        Ok(true),
        Err("repository is not allowed"),
        Err("repository is not allowed"),
        true,
        true,
        false
    )]
    #[case::vibe_disabled(
        &["a", "--vibe"],
        Ok(false),
        Ok(()),
        Err("Vibe mode must be enabled to use --vibe."),
        true,
        false,
        false
    )]
    #[case::vibe_check_fails(
        &["a", "--vibe"],
        Err("session lookup failed"),
        Ok(()),
        Err("session lookup failed"),
        true,
        false,
        false
    )]
    fn vibe_flag_gates_session_launch(
        #[case] argv: &[&str],
        #[case] vibe_check_result: std::result::Result<bool, &'static str>,
        #[case] repo_access_result: std::result::Result<(), &'static str>,
        #[case] expected_result: std::result::Result<(), &'static str>,
        #[case] expected_vibe_check: bool,
        #[case] expected_repo_check: bool,
        #[case] expected_launch: bool,
    ) {
        let args = TestCli::try_parse_from(argv).unwrap().args;
        let vibe_checked = Cell::new(false);
        let repo_checked = Cell::new(false);
        let launched = Cell::new(false);

        let result = run_with_vibe_access(
            &args,
            || {
                vibe_checked.set(true);
                vibe_check_result.map_err(anyhow::Error::msg)
            },
            |_| {
                repo_checked.set(true);
                repo_access_result.map_err(anyhow::Error::msg)
            },
            || {
                launched.set(true);
                Ok(())
            },
        );

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                vibe_checked.get(),
                repo_checked.get(),
                launched.get(),
            ),
            (
                expected_result.map_err(str::to_owned),
                expected_vibe_check,
                expected_repo_check,
                expected_launch,
            ),
        );
    }

    #[test]
    fn vibe_flag_checks_the_explicit_target_repository_before_launch() {
        let args = TestCli::try_parse_from(["a", "--vibe", "-R", "/tmp/example-target"])
            .unwrap()
            .args;
        let selected_path = RefCell::new(None);
        let launched = Cell::new(false);
        let config: Config = serde_yaml::from_str(indoc::indoc! {
            "
            orgs:
              example-org:
                vibe: true
            repos:
              example-org/denied-repository:
                vibe: false
            "
        })
        .unwrap();

        let result = run_with_vibe_access(
            &args,
            || Ok(true),
            |args| {
                ensure_target_repo_vibe_access(
                    args,
                    |path| {
                        selected_path.replace(path.map(Path::to_path_buf));
                        Ok(("example-org".to_string(), "denied-repository".to_string()))
                    },
                    || Ok(config),
                )
            },
            || {
                launched.set(true);
                Ok(())
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, selected_path.into_inner(), launched.get()),
            (
                Err("Vibe mode is disabled by configuration for repository example-org/denied-repository.".to_string()),
                Some(PathBuf::from("/tmp/example-target")),
                false,
            ),
        );
    }

    #[rstest]
    #[case::omitted(&["a"], None)]
    #[case::explicit_codex(&["a", "--engine", "codex"], Some(Engine::Codex))]
    #[case::codex_with_worktree(&["a", "--worktree=my-branch", "--engine", "codex"], Some(Engine::Codex))]
    fn engine_value_parses(#[case] argv: &[&str], #[case] expected: Option<Engine>) {
        let cli = TestCli::try_parse_from(argv).unwrap();
        assert_eq!(cli.args.common.engine, expected);
    }

    #[rstest]
    #[case::omitted(&["a"], None)]
    #[case::xhigh(&["a", "--reasoning-effort", "xhigh"], Some(ReasoningEffort::XHigh))]
    #[case::with_worktree(&["a", "--worktree", "--reasoning-effort", "max"], Some(ReasoningEffort::Max))]
    #[case::max(&["a", "--reasoning-effort", "max"], Some(ReasoningEffort::Max))]
    fn reasoning_effort_value_parses(
        #[case] argv: &[&str],
        #[case] expected: Option<ReasoningEffort>,
    ) {
        let cli = TestCli::try_parse_from(argv).unwrap();
        assert_eq!(cli.args.common.reasoning_effort, expected);
    }

    #[rstest]
    #[case::omitted(&["a"], None)]
    #[case::explicit_kind(&["a", "--kind", "sample-skill"], Some("sample-skill"))]
    fn kind_value_parses(#[case] argv: &[&str], #[case] expected: Option<&str>) {
        let cli = TestCli::try_parse_from(argv).unwrap();
        assert_eq!(cli.args.common.kind.as_deref(), expected);
    }

    #[rstest]
    #[case::omitted(&["a"], None)]
    #[case::arbitrary_type(&["a", "--type", "review type"], Some("review type"))]
    fn type_value_parses(#[case] argv: &[&str], #[case] expected: Option<&str>) {
        let cli = TestCli::try_parse_from(argv).unwrap();
        assert_eq!(cli.args.common.type_name.as_deref(), expected);
    }

    #[test]
    fn dry_run_dispatch_skips_regular_launch_path() {
        let args = TestCli::try_parse_from([
            "a",
            "--type",
            "review type",
            "--dry-run",
            "--worktree=preview-branch",
        ])
        .unwrap()
        .args;
        let dry_run_called = Cell::new(false);
        let regular_called = Cell::new(false);

        let result = profile_type::dispatch(
            &args,
            |_| {
                dry_run_called.set(true);
                Ok(())
            },
            |_| {
                regular_called.set(true);
                Ok(())
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, dry_run_called.get(), regular_called.get()),
            (Ok(()), true, false),
        );
    }

    #[rstest]
    #[case::from_without_worktree(&["a", "--from", "origin/master"])]
    #[case::force_without_worktree(&["a", "--force"])]
    #[case::unknown_reasoning_effort(&["a", "--reasoning-effort", "maxx"])]
    #[case::dry_run_requires_type(&["a", "--dry-run"])]
    fn rejects_missing_or_misplaced_flags(#[case] argv: &[&str]) {
        assert!(TestCli::try_parse_from(argv).is_err());
    }

    #[test]
    fn skip_hooks_is_valid_without_worktree() {
        let cli = TestCli::try_parse_from(["a", "--skip-hooks"]).unwrap();

        assert_eq!((cli.args.worktree, cli.args.skip_hooks), (None, true));
    }

    #[rstest]
    #[case::success_deletes_cache(true, true, true)]
    #[case::failure_keeps_cache(false, true, true)]
    #[case::no_prompt_skips_save(true, false, false)]
    fn with_prompt_cache_recovery_saves_and_cleans_up(
        #[case] succeed: bool,
        #[case] has_prompt: bool,
        #[case] expect_save: bool,
    ) {
        use std::cell::Cell;

        let saved = Cell::new(false);
        let deleted = Cell::new(false);

        let result = with_prompt_cache_recovery_with_deps(
            "repo",
            has_prompt.then_some("my prompt"),
            || {
                if succeed {
                    Ok(())
                } else {
                    anyhow::bail!("boom")
                }
            },
            |_repo, prompt| {
                saved.set(true);
                Ok(PathBuf::from(format!("/cache/{prompt}")))
            },
            |_repo| deleted.set(true),
        );

        assert_eq!(
            (result.is_ok(), saved.get(), deleted.get()),
            (succeed, expect_save, succeed),
        );
    }

    #[rstest]
    #[case::forwards_when_set(Some("tq-session-1"), Some(("TQ_PARENT_SESSION_ID".to_string(), "tq-session-1".to_string())))]
    #[case::none_when_unset(None, None)]
    #[case::none_when_empty(Some(""), None)]
    fn tq_parent_session_env_var_cases(
        #[case] env_value: Option<&str>,
        #[case] expected: Option<(String, String)>,
    ) {
        temp_env::with_vars([("TQ_SESSION_ID", env_value)], || {
            assert_eq!(tq_parent_session_env_var(), expected);
        });
    }

    #[rstest]
    #[case::omitted(
        None,
        false,
        false,
        vec![(EnvVars::session_child_launch_name().to_string(), "allow".to_string())]
    )]
    #[case::explicit_kind(
        Some("sample-skill"),
        false,
        false,
        vec![
            (EnvVars::session_work_type_name().to_string(), "sample-skill".to_string()),
            (EnvVars::session_child_launch_name().to_string(), "allow".to_string()),
        ]
    )]
    #[case::agent_launch(
        None,
        true,
        false,
        vec![
            (EnvVars::session_child_launch_name().to_string(), "deny".to_string()),
            (EnvVars::session_delegated_name().to_string(), "1".to_string()),
        ]
    )]
    #[case::vibe_launch(
        None,
        false,
        true,
        vec![
            (EnvVars::session_child_launch_name().to_string(), "allow".to_string()),
            (EnvVars::session_vibe_launch_name().to_string(), "1".to_string()),
        ]
    )]
    fn build_env_vars_passes_launch_metadata(
        #[case] kind: Option<&str>,
        #[case] agent: bool,
        #[case] vibe: bool,
        #[case] expected: Vec<(String, String)>,
    ) {
        temp_env::with_vars(
            [
                ("ARMYKNIFE_SESSION_ID", None::<&str>),
                ("CODEX_SESSION_ID", None::<&str>),
                ("TQ_SESSION_ID", None::<&str>),
            ],
            || {
                let common = CommonNewArgs {
                    prompt: None,
                    agent,
                    child_launch: None,
                    label: None,
                    kind: kind.map(str::to_string),
                    type_name: None,
                    model: None,
                    parent_session_id: None,
                    repo: None,
                    engine: None,
                    reasoning_effort: None,
                };
                assert_eq!(
                    build_env_vars(&common, vibe, EnvVars::load().own_session_id()).unwrap(),
                    expected
                );
            },
        );
    }

    #[rstest]
    #[case::claude_code(
        &["a"],
        Some("1"),
        None,
        None,
        None,
        vec![(EnvVars::session_child_launch_name().to_string(), "allow".to_string())],
        true
    )]
    #[case::armyknife_session(
        &["a"],
        None,
        Some("caller"),
        None,
        None,
        vec![
            (EnvVars::session_child_launch_name().to_string(), "allow".to_string()),
            (EnvVars::ancestor_session_ids_name().to_string(), "caller".to_string()),
        ],
        true
    )]
    #[case::codex_session(
        &["a"],
        None,
        None,
        Some("caller"),
        None,
        vec![
            (EnvVars::session_child_launch_name().to_string(), "allow".to_string()),
            (EnvVars::ancestor_session_ids_name().to_string(), "caller".to_string()),
        ],
        true
    )]
    #[case::empty_session_id(
        &["a"], None, Some(""), Some(""), None,
        vec![(EnvVars::session_child_launch_name().to_string(), "allow".to_string())], false
    )]
    #[case::human_tmux_shell(
        &["a"], None, None, None, Some("fake-tmux"),
        vec![(EnvVars::session_child_launch_name().to_string(), "allow".to_string())], false
    )]
    #[case::agent_flag_alone(
        &["a", "--agent"],
        None,
        None,
        None,
        None,
        vec![
            (EnvVars::session_child_launch_name().to_string(), "deny".to_string()),
            (EnvVars::session_delegated_name().to_string(), "1".to_string())
        ],
        false
    )]
    #[case::legacy_agent_overrides_child_launch_allow(
        &["a", "--agent", "--child-launch", "allow"],
        None,
        None,
        None,
        None,
        vec![
            (EnvVars::session_child_launch_name().to_string(), "deny".to_string()),
            (EnvVars::session_delegated_name().to_string(), "1".to_string())
        ],
        false
    )]
    #[case::explicit_child_launch_deny(
        &["a", "--child-launch", "deny"], None, None, None, None,
        vec![(EnvVars::session_child_launch_name().to_string(), "deny".to_string())], false
    )]
    #[case::explicit_child_launch_allow(
        &["a", "--child-launch", "allow"], None, None, None, None,
        vec![(EnvVars::session_child_launch_name().to_string(), "allow".to_string())], false
    )]
    #[case::codex_child_from_tmux_shell(
        &["a", "--engine", "codex"], None, None, None, Some("fake-tmux"),
        vec![(EnvVars::session_child_launch_name().to_string(), "allow".to_string())], false
    )]
    fn tmux_launch_inputs_only_background_agent_invocations(
        #[case] argv: &[&str],
        #[case] claudecode: Option<&str>,
        #[case] armyknife_session_id: Option<&str>,
        #[case] codex_session_id: Option<&str>,
        #[case] tmux: Option<&str>,
        #[case] expected_env_vars: Vec<(String, String)>,
        #[case] expected_background: bool,
    ) {
        let args = TestCli::try_parse_from(argv).unwrap().args;
        let actual = temp_env::with_vars(
            [
                ("CLAUDECODE", claudecode),
                ("ARMYKNIFE_SESSION_ID", armyknife_session_id),
                ("CODEX_SESSION_ID", codex_session_id),
                ("TMUX", tmux),
                ("TMUX_PANE", tmux.map(|_| "%1")),
                ("TQ_SESSION_ID", None::<&str>),
            ],
            || tmux_launch_inputs(&args.common, args.vibe),
        )
        .unwrap();

        assert_eq!(actual, (expected_env_vars, expected_background));
    }
}
