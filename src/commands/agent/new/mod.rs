use anyhow::{Context, Result};
use clap::Args;
use std::path::PathBuf;

use crate::commands::agent::types::{Engine, ReasoningEffort};
use crate::infra::git::{get_repo_root, get_repo_root_in};
use crate::shared::config::{Config, LayoutNode, PaneConfig, load_config};
use crate::shared::env_var::EnvVars;

mod delegation;
mod prompt;
mod session_mode;
mod tmux;
mod worktree;
mod worktree_creation;

use delegation::{build_ancestor_chain, resolve_prompt};
use prompt::{delete_prompt_cache, resolve_args, save_prompt_cache};
use session_mode::{caller_repo_root, should_open_window};
use tmux::{TmuxSplitPaneSpec, TmuxWindowSpec, setup_split_pane, setup_tmux_window};
use worktree_creation::run_worktree_creation;

/// CLI args shared by `a agent new`'s worktree and no-worktree modes.
#[derive(Args, Clone, PartialEq, Eq)]
pub struct CommonNewArgs {
    /// Initial prompt to send to the agent (Claude Code or Codex).
    /// When provided without a branch name, the branch name is auto-generated from this prompt.
    #[arg(long)]
    pub prompt: Option<String>,

    /// Mark this invocation as coming from another Claude Code session.
    /// Wraps the prompt with delegation context (branch, base, directories).
    #[arg(long)]
    pub agent: bool,

    /// Label for the new session (displayed in agent watch).
    /// When not specified, the session will get its label via the
    /// user-prompt-submit hook (auto-generation from prompt).
    #[arg(long)]
    pub label: Option<String>,

    /// Model for the new session. Passed through to `<engine> --model`.
    /// Accepts an alias (e.g. "opus", "sonnet") or a full model name
    /// (e.g. "claude-fable-5"). For `--engine codex`, falls back to
    /// `agent.codex.model` when omitted.
    #[arg(long)]
    pub model: Option<String>,

    /// Parent session ID for tree view hierarchy.
    /// Sets ARMYKNIFE_ANCESTOR_SESSION_IDS for the child session.
    #[arg(long)]
    pub parent_session_id: Option<String>,

    /// Path to the target repository.
    /// When specified, operates on the given repository instead of the current directory.
    #[arg(short = 'R', long)]
    pub repo: Option<PathBuf>,

    /// Coding agent CLI to launch for the new session. Falls back to
    /// `config.agent.default_engine` (default: `claude`) when omitted.
    ///
    /// With `--worktree`, `config.wm.layout` panes running `claude` are
    /// replaced by this engine's CLI (without their arguments) unless the
    /// layout already has a pane for it; other panes are left as written.
    #[arg(long, value_enum)]
    pub engine: Option<Engine>,

    /// Reasoning effort for the new session. Passed as `claude --effort`.
    /// For Codex, sent in the first `turn/start` on the daemon route and as
    /// `-c model_reasoning_effort=...` otherwise. Falls back to
    /// `agent.codex.reasoning_effort` when omitted.
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
    /// session runs in the current directory (or the target repo root when
    /// -R is given). If that target is the same repo as the invoking Claude
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

    #[command(flatten)]
    pub common: CommonNewArgs,

    /// Skip the post-worktree-create hook (requires --worktree).
    /// Useful when the hook itself is broken and needs to be fixed inside the new worktree.
    #[arg(long, requires = "worktree")]
    pub skip_hooks: bool,
}

pub fn run(args: &NewArgs) -> Result<()> {
    run_inner(args)
}

fn run_inner(args: &NewArgs) -> Result<()> {
    let config = load_config()?;

    let repo_root = match &args.common.repo {
        Some(path) => get_repo_root_in(path)?,
        None => get_repo_root()?,
    };

    match &args.worktree {
        Some(worktree_value) => {
            run_worktree_mode(args, worktree_value.as_deref(), &repo_root, &config)
        }
        None => run_session_only(args, &repo_root, &config),
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

/// Build tmux session-level env vars for the child session's `--label`,
/// ancestor-session-id chain, and tq parent session ID. Shared between the
/// worktree and no-worktree flows.
fn build_env_vars(common: &CommonNewArgs) -> Result<Vec<(String, String)>> {
    let mut env_vars: Vec<(String, String)> = Vec::new();
    if let Some(ref label) = common.label {
        env_vars.push((EnvVars::session_label_name().to_string(), label.clone()));
    }
    // Resolve parent session ID: explicit flag > ARMYKNIFE_SESSION_ID env var.
    // ARMYKNIFE_SESSION_ID is set by the SessionStart hook via CLAUDE_ENV_FILE,
    // so `a agent new` called from a Claude Code Bash tool automatically inherits
    // the parent session ID without requiring --parent-session-id. Codex has no
    // env file, so it falls back to CODEX_SESSION_ID (see `own_session_id`).
    let parent_id = common
        .parent_session_id
        .clone()
        .or(EnvVars::load().own_session_id());
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
fn tmux_launch_inputs(common: &CommonNewArgs) -> Result<(Vec<(String, String)>, bool)> {
    let env_vars = build_env_vars(common)?;
    // Avoid stealing the user's tmux focus when auto-invoked from Claude Code.
    let background = std::env::var("CLAUDECODE").is_ok();
    Ok((env_vars, background))
}

/// Resolves the engine to launch: an explicit
/// `--engine` always wins over `config.agent.default_engine`, which itself
/// already reflects any `ARMYKNIFE_AGENT__DEFAULT_ENGINE` override applied
/// while loading `config` (see `env_overlay`) and defaults to `Engine::Claude`
/// when neither is set.
fn resolve_engine(explicit: Option<Engine>, config: &Config) -> Engine {
    explicit.unwrap_or(config.agent.default_engine)
}

/// Resolves the model and reasoning effort to launch `engine` with. Explicit
/// flags win over the `agent.codex` config defaults, which only apply to
/// codex so a Claude session never receives codex-specific values.
fn resolve_launch_options(
    engine: Engine,
    common: &CommonNewArgs,
    config: &Config,
) -> (Option<String>, Option<ReasoningEffort>) {
    match engine {
        Engine::Claude => (common.model.clone(), common.reasoning_effort),
        Engine::Codex => (
            common
                .model
                .clone()
                .or_else(|| config.agent.codex.model.clone()),
            common
                .reasoning_effort
                .or(config.agent.codex.reasoning_effort),
        ),
    }
}

/// Run `a agent new` without creating a worktree, either by splitting the
/// caller's pane or opening a window in the target repo's session (see
/// `should_open_window`).
fn run_session_only(args: &NewArgs, repo_root: &str, config: &Config) -> Result<()> {
    let raw_prompt = args.common.prompt.as_deref();

    with_prompt_cache_recovery(repo_root, raw_prompt, || {
        run_session_only_inner(args, repo_root, config)
    })
}

fn run_session_only_inner(args: &NewArgs, repo_root: &str, config: &Config) -> Result<()> {
    let current_dir = std::env::current_dir()
        .context("Failed to get current directory")?
        .to_string_lossy()
        .to_string();

    let cwd = if args.common.repo.is_some() {
        repo_root.to_string()
    } else {
        current_dir.clone()
    };

    // No worktree exists for this session, so there is no branch/base to
    // report in the delegation context.
    let prompt = if args.common.agent {
        resolve_prompt(
            true,
            args.common.prompt.as_deref(),
            None,
            None,
            &current_dir,
            &cwd,
        )
    } else {
        args.common.prompt.clone()
    };

    let (env_vars, background) = tmux_launch_inputs(&args.common)?;
    let env_refs: Vec<(&str, &str)> = env_vars
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let suffix = if background { " (background)" } else { "" };
    let engine = resolve_engine(args.common.engine, config);
    let (model, reasoning_effort) = resolve_launch_options(engine, &args.common, config);

    let differs = should_open_window(
        repo_root,
        &caller_repo_root(&current_dir),
        &config.wm.worktrees_dir,
    );

    // Window mode doesn't need a pane, so it's also the fallback when the
    // caller isn't running inside one ($TMUX_PANE unset).
    match crate::infra::tmux::current_pane_id_from_env() {
        Some(target_pane) if !differs => {
            let route = setup_split_pane(TmuxSplitPaneSpec {
                target_pane: &target_pane,
                cwd: &cwd,
                model: model.as_deref(),
                reasoning_effort,
                prompt: prompt.as_deref(),
                engine,
                env_vars: &env_refs,
                background,
            })?;
            println!(
                "Split tmux pane in '{cwd}'{suffix}{}",
                route.display_suffix()
            );
        }
        _ => {
            // PID-based placeholder: there's no worktree/branch name to use
            // here, and `restore_automatic_rename` below lets tmux relabel
            // the window once the engine's process starts instead of keeping
            // this name displayed.
            let window_name = format!("{}-{}", engine.process_name(), std::process::id());
            let layout = LayoutNode::Pane(PaneConfig {
                command: engine.process_name().to_string(),
                focus: true,
            });

            let route = setup_tmux_window(
                TmuxWindowSpec {
                    repo_root,
                    cwd: &cwd,
                    window_name: &window_name,
                    layout: &layout,
                    model: model.as_deref(),
                    reasoning_effort,
                    prompt: prompt.as_deref(),
                    engine,
                    env_vars: &env_refs,
                    background,
                    restore_automatic_rename: true,
                },
                config,
            )?;
            println!(
                "Opened tmux window in '{cwd}'{suffix}{}",
                route.display_suffix()
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use rstest::rstest;

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

    #[rstest]
    #[case::omitted(&["a"], None)]
    #[case::explicit_codex(&["a", "--engine", "codex"], Some(Engine::Codex))]
    #[case::codex_with_worktree(&["a", "--worktree=my-branch", "--engine", "codex"], Some(Engine::Codex))]
    fn engine_value_parses(#[case] argv: &[&str], #[case] expected: Option<Engine>) {
        let cli = TestCli::try_parse_from(argv).unwrap();
        assert_eq!(cli.args.common.engine, expected);
    }

    #[rstest]
    #[case::explicit_wins_over_config_default(Some(Engine::Codex), Engine::Claude, Engine::Codex)]
    #[case::falls_back_to_config_default(None, Engine::Codex, Engine::Codex)]
    #[case::falls_back_to_claude_when_neither_set(None, Engine::Claude, Engine::Claude)]
    fn resolve_engine_cases(
        #[case] explicit: Option<Engine>,
        #[case] config_default: Engine,
        #[case] expected: Engine,
    ) {
        let config = Config {
            agent: crate::shared::config::AgentConfig {
                default_engine: config_default,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(resolve_engine(explicit, &config), expected);
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

    fn codex_defaults() -> Config {
        Config {
            agent: crate::shared::config::AgentConfig {
                codex: crate::shared::config::CodexConfig {
                    model: Some("gpt-5.6-luna".to_string()),
                    reasoning_effort: Some(ReasoningEffort::Max),
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[rstest]
    #[case::codex_uses_config_defaults(
        Engine::Codex,
        &["a"],
        (Some("gpt-5.6-luna"), Some(ReasoningEffort::Max))
    )]
    #[case::codex_flags_win_over_config(
        Engine::Codex,
        &["a", "--model", "gpt-5", "--reasoning-effort", "low"],
        (Some("gpt-5"), Some(ReasoningEffort::Low))
    )]
    #[case::claude_ignores_codex_config(Engine::Claude, &["a"], (None, None))]
    #[case::claude_keeps_explicit_model(
        Engine::Claude,
        &["a", "--model", "opus"],
        (Some("opus"), None)
    )]
    #[case::claude_passes_explicit_reasoning_effort(
        Engine::Claude,
        &["a", "--reasoning-effort", "max"],
        (None, Some(ReasoningEffort::Max))
    )]
    fn resolve_launch_options_cases(
        #[case] engine: Engine,
        #[case] argv: &[&str],
        #[case] expected: (Option<&str>, Option<ReasoningEffort>),
    ) {
        let cli = TestCli::try_parse_from(argv).unwrap();
        let expected = (expected.0.map(str::to_string), expected.1);
        assert_eq!(
            resolve_launch_options(engine, &cli.args.common, &codex_defaults()),
            expected
        );
    }

    #[rstest]
    #[case::from_without_worktree(&["a", "--from", "origin/master"])]
    #[case::force_without_worktree(&["a", "--force"])]
    #[case::skip_hooks_without_worktree(&["a", "--skip-hooks"])]
    #[case::unknown_reasoning_effort(&["a", "--reasoning-effort", "maxx"])]
    fn rejects_missing_or_misplaced_flags(#[case] argv: &[&str]) {
        assert!(TestCli::try_parse_from(argv).is_err());
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
}
