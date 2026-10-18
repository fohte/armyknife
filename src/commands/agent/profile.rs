use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Subcommand};
use serde_yaml::{Mapping, Value};

use crate::commands::agent::types::Engine;
use crate::infra::git::file_lock::{lock_exclusive, open_lock_file};
use crate::shared::config::{self, Config, ResolvedLaunchOptions};

const SELECTION_FILE: &str = "zz-agent-profiles.local.yaml";
const SELECTION_LOCK_FILE: &str = "zz-agent-profiles.lock";

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum ProfileCommands {
    /// Set the default engine for future launches of a configured kind
    Use(UseProfileArgs),

    /// Show the effective settings for the next launch of a configured kind
    Show(ShowProfileArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct UseProfileArgs {
    /// Configured `--kind` value
    pub kind: String,

    /// Default coding agent CLI for future launches
    #[arg(value_enum)]
    pub engine: Engine,
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ShowProfileArgs {
    /// Configured `--kind` value
    pub kind: String,
}

pub fn run(command: &ProfileCommands) -> Result<()> {
    print!("{}", execute(command)?);
    Ok(())
}

fn execute(command: &ProfileCommands) -> Result<String> {
    let config_dir = config_dir()?;
    let selection_path = config_dir.join(SELECTION_FILE);
    let (kind, engine) = match command {
        ProfileCommands::Use(args) => (&args.kind, Some(args.engine)),
        ProfileCommands::Show(args) => (&args.kind, None),
    };

    if engine.is_some() {
        reject_symlink(&selection_path)?;
    }
    let configured_profiles = config::load_config_without_file(SELECTION_FILE)?;
    ensure_profile_exists(&configured_profiles, kind)?;
    if let Some(engine) = engine {
        save_engine_selection(&config_dir, kind, engine)?;
    }
    let config = config::load_config()?;
    format_profile(&config, &selection_path, kind)
}

fn config_dir() -> Result<PathBuf> {
    crate::shared::dirs::config_dir()
        .map(|path| path.join("armyknife"))
        .context("configuration directory is unavailable")
}

fn ensure_profile_exists(config: &Config, kind: &str) -> Result<()> {
    ensure!(
        config.agent.launch_profiles.contains_key(kind),
        "unknown agent profile kind `{kind}`; configured launch profile kinds: {}",
        configured_kinds(config),
    );
    Ok(())
}

fn configured_kinds(config: &Config) -> String {
    let mut kinds = config
        .agent
        .launch_profiles
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    kinds.sort_unstable();
    if kinds.is_empty() {
        "(none)".to_string()
    } else {
        kinds.join(", ")
    }
}

fn format_profile(config: &Config, selection_path: &Path, kind: &str) -> Result<String> {
    let saved_engine = read_saved_engine(selection_path, kind)?;
    let options = config
        .agent
        .resolve_launch_options(Some(kind), None, None, None);
    Ok(format_profile_options(kind, saved_engine, &options))
}

fn format_profile_options(
    kind: &str,
    saved_engine: Option<Engine>,
    options: &ResolvedLaunchOptions,
) -> String {
    format!(
        "Next launch settings for `a agent new --kind {kind}`:\n  local engine selection: {}\n  effective engine: {}\n  effective model: {}\n  effective reasoning effort: {}\n",
        saved_engine.map_or("unset", |engine| engine.process_name()),
        options.engine.process_name(),
        options.model.as_deref().unwrap_or("engine default"),
        options
            .reasoning_effort
            .map_or("engine default", |effort| effort.as_str()),
    )
}

fn read_saved_engine(path: &Path, kind: &str) -> Result<Option<Engine>> {
    let document = read_selection_document(path)?;
    let Some(value) = yaml_child(&document, "agent")
        .and_then(|value| yaml_child(value, "launch_profiles"))
        .and_then(|value| yaml_child(value, kind))
        .and_then(|value| yaml_child(value, "engine"))
    else {
        return Ok(None);
    };

    serde_yaml::from_value(value.clone())
        .with_context(|| format!("invalid saved engine for profile kind `{kind}`"))
        .map(Some)
}

fn yaml_child<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.as_mapping()?.get(Value::String(key.to_string()))
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("refusing to update symlink: {}", path.display());
        }
        Ok(metadata) if !metadata.is_file() => {
            bail!(
                "profile selection is not a regular file: {}",
                path.display()
            );
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect profile selection: {}", path.display())),
    }
}

fn save_engine_selection(config_dir: &Path, kind: &str, engine: Engine) -> Result<()> {
    fs::create_dir_all(config_dir).with_context(|| {
        format!(
            "failed to create config directory: {}",
            config_dir.display()
        )
    })?;
    let path = config_dir.join(SELECTION_FILE);
    let lock_path = config_dir.join(SELECTION_LOCK_FILE);
    let lock_file = open_lock_file(&lock_path, "agent profile selection")?;
    lock_exclusive(&lock_file, "agent profile selection")?;
    reject_symlink(&path)?;

    // Keep the read-modify-write cycle exclusive across concurrent CLI processes.
    let mut document = read_selection_document(&path)?;
    set_engine(&mut document, kind, engine)?;

    // Persist on the same filesystem so the rename cannot expose a partial YAML file.
    let mut temp_file = tempfile::NamedTempFile::new_in(config_dir).with_context(|| {
        format!(
            "failed to create temporary profile selection in {}",
            config_dir.display()
        )
    })?;
    serde_yaml::to_writer(&mut temp_file, &document)
        .context("failed to serialize profile selection")?;
    temp_file.write_all(b"\n")?;
    temp_file.as_file().sync_all()?;
    temp_file
        .persist(&path)
        .with_context(|| format!("failed to replace profile selection: {}", path.display()))?;
    Ok(())
}

fn read_selection_document(path: &Path) -> Result<Value> {
    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Null),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to read profile selection: {}", path.display()));
        }
    };
    serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse profile selection: {}", path.display()))
}

fn set_engine(document: &mut Value, kind: &str, engine: Engine) -> Result<()> {
    let agent = mapping_child(document, "agent")?;
    let profiles = mapping_child(agent, "launch_profiles")?;
    let profile = mapping_child(profiles, kind)?;
    let profile = profile
        .as_mapping_mut()
        .context("profile selection must contain a mapping for each kind")?;
    profile.insert(
        Value::String("engine".to_string()),
        Value::String(engine.process_name().to_string()),
    );
    Ok(())
}

fn mapping_child<'a>(value: &'a mut Value, key: &str) -> Result<&'a mut Value> {
    if value.is_null() {
        *value = Value::Mapping(Mapping::new());
    }
    let mapping = value
        .as_mapping_mut()
        .context("profile selection must contain YAML mappings")?;
    Ok(mapping
        .entry(Value::String(key.to_string()))
        .or_insert_with(|| Value::Mapping(Mapping::new())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::AgentCommands;
    use clap::Parser;
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    #[derive(clap::Parser)]
    struct TestCli {
        #[command(subcommand)]
        command: AgentCommands,
    }

    #[fixture]
    fn configured_home() -> TempDir {
        let home = TempDir::new().expect("config home should be created");
        let config_dir = home.path().join("armyknife");
        fs::create_dir_all(&config_dir).expect("config directory should be created");
        fs::write(
            config_dir.join("profiles.yaml"),
            indoc! {"
                agent:
                  default_engine: claude
                  launch_profiles:
                    sample_flow:
                      engine: claude
                      claude:
                        model: model-alpha
                        reasoning_effort: low
                      codex:
                        model: model-beta
                        reasoning_effort: high
                    second_flow:
                      engine: codex
                      codex:
                        model: model-gamma
                        reasoning_effort: max
            "},
        )
        .expect("profile config should be written");
        home
    }

    fn selection_path(home: &TempDir) -> PathBuf {
        home.path().join("armyknife").join(SELECTION_FILE)
    }

    fn execute_with_home(
        home: &TempDir,
        command: &ProfileCommands,
        env_engine: Option<&str>,
    ) -> Result<String> {
        let config_home = home
            .path()
            .to_str()
            .expect("config home path should be valid UTF-8");
        temp_env::with_vars(
            [
                ("XDG_CONFIG_HOME", Some(config_home)),
                (
                    "ARMYKNIFE_AGENT__LAUNCH_PROFILES__SAMPLE_FLOW__ENGINE",
                    env_engine,
                ),
            ],
            || execute(command),
        )
    }

    #[test]
    fn profile_commands_parse_kind_and_engine() {
        let parsed = vec![
            TestCli::try_parse_from(["a", "profile", "use", "sample_flow", "codex"])
                .expect("profile use should parse")
                .command,
            TestCli::try_parse_from(["a", "profile", "show", "sample_flow"])
                .expect("profile show should parse")
                .command,
        ];
        let expected = vec![
            AgentCommands::Profile(ProfileCommands::Use(UseProfileArgs {
                kind: "sample_flow".to_string(),
                engine: Engine::Codex,
            })),
            AgentCommands::Profile(ProfileCommands::Show(ShowProfileArgs {
                kind: "sample_flow".to_string(),
            })),
        ];

        assert!(parsed == expected);
    }

    #[rstest]
    fn use_creates_local_selection_file(configured_home: TempDir) {
        let command = ProfileCommands::Use(UseProfileArgs {
            kind: "sample_flow".to_string(),
            engine: Engine::Codex,
        });

        execute_with_home(&configured_home, &command, None)
            .expect("profile selection should be saved");
        let saved: Value = serde_yaml::from_str(
            &fs::read_to_string(selection_path(&configured_home))
                .expect("local selection file should be created"),
        )
        .expect("saved selection should be valid YAML");
        let expected: Value = serde_yaml::from_str(indoc! {"
            agent:
              launch_profiles:
                sample_flow:
                  engine: codex
        "})
        .expect("expected selection should be valid YAML");

        assert_eq!(saved, expected);
    }

    #[rstest]
    #[tokio::test(flavor = "multi_thread")]
    async fn profile_use_dispatch_persists_selection(configured_home: TempDir) {
        let cli = TestCli::try_parse_from(["a", "profile", "use", "sample_flow", "codex"])
            .expect("profile command should parse");
        let config_home = configured_home
            .path()
            .to_str()
            .expect("config home path should be valid UTF-8");

        let result = temp_env::with_vars(
            [
                ("XDG_CONFIG_HOME", Some(config_home)),
                (
                    "ARMYKNIFE_AGENT__LAUNCH_PROFILES__SAMPLE_FLOW__ENGINE",
                    None::<&str>,
                ),
            ],
            || {
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(cli.command.run())
                })
            },
        )
        .expect("profile command should succeed");

        let saved: Value = serde_yaml::from_str(
            &fs::read_to_string(selection_path(&configured_home))
                .expect("local selection file should be created"),
        )
        .expect("saved selection should be valid YAML");
        let expected: Value = serde_yaml::from_str(indoc! {"
            agent:
              launch_profiles:
                sample_flow:
                  engine: codex
        "})
        .expect("expected selection should be valid YAML");

        assert_eq!((result, saved), (None, expected));
    }

    #[rstest]
    fn use_saves_engine_and_preserves_other_local_selections(configured_home: TempDir) {
        let path = selection_path(&configured_home);
        fs::write(
            &path,
            indoc! {"
                agent:
                  launch_profiles:
                    sample_flow:
                      engine: claude
                    second_flow:
                      engine: claude
            "},
        )
        .expect("existing selections should be written");
        let command = ProfileCommands::Use(UseProfileArgs {
            kind: "sample_flow".to_string(),
            engine: Engine::Codex,
        });

        execute_with_home(&configured_home, &command, None)
            .expect("profile selection should be saved");
        let saved: Value = serde_yaml::from_str(
            &fs::read_to_string(path).expect("profile selections should be readable"),
        )
        .expect("saved selections should be valid YAML");
        let expected: Value = serde_yaml::from_str(indoc! {"
            agent:
              launch_profiles:
                sample_flow:
                  engine: codex
                second_flow:
                  engine: claude
        "})
        .expect("expected selections should be valid YAML");

        assert_eq!(saved, expected);
    }

    #[rstest]
    #[case::local_selection_is_effective(None, indoc! {"
        Next launch settings for `a agent new --kind sample_flow`:
          local engine selection: codex
          effective engine: codex
          effective model: model-beta
          effective reasoning effort: high
    "})]
    #[case::environment_override_is_reported(Some("claude"), indoc! {"
        Next launch settings for `a agent new --kind sample_flow`:
          local engine selection: codex
          effective engine: claude
          effective model: model-alpha
          effective reasoning effort: low
    "})]
    fn show_reports_next_launch_settings(
        configured_home: TempDir,
        #[case] env_engine: Option<&str>,
        #[case] expected: &str,
    ) {
        fs::write(
            selection_path(&configured_home),
            indoc! {"
                agent:
                  launch_profiles:
                    sample_flow:
                      engine: codex
            "},
        )
        .expect("profile selection should be written");
        let command = ProfileCommands::Show(ShowProfileArgs {
            kind: "sample_flow".to_string(),
        });

        let output = execute_with_home(&configured_home, &command, env_engine)
            .expect("profile settings should be shown");

        assert_eq!(output, expected);
    }

    #[rstest]
    fn show_reports_unset_local_selection(configured_home: TempDir) {
        let command = ProfileCommands::Show(ShowProfileArgs {
            kind: "sample_flow".to_string(),
        });

        let output = execute_with_home(&configured_home, &command, None)
            .expect("profile settings should be shown");

        assert_eq!(
            output,
            indoc! {"
                Next launch settings for `a agent new --kind sample_flow`:
                  local engine selection: unset
                  effective engine: claude
                  effective model: model-alpha
                  effective reasoning effort: low
            "},
        );
    }

    #[test]
    fn use_rejects_symlink_selection_file() {
        let home = TempDir::new().expect("config home should be created");
        let config_dir = home.path().join("armyknife");
        fs::create_dir_all(&config_dir).expect("config directory should be created");
        let target = home.path().join("linked-selection.yaml");
        fs::write(&target, "keep this target\n").expect("symlink target should be written");
        let path = selection_path(&home);
        symlink(&target, &path).expect("selection symlink should be created");
        let command = ProfileCommands::Use(UseProfileArgs {
            kind: "sample_flow".to_string(),
            engine: Engine::Codex,
        });
        let config_home = home
            .path()
            .to_str()
            .expect("config home path should be valid UTF-8");
        let result = temp_env::with_vars([("XDG_CONFIG_HOME", Some(config_home))], || {
            execute(&command)
        });

        assert_eq!(
            (
                result.err().map(|error| error.to_string()),
                fs::read_link(&path).expect("selection symlink should remain"),
                fs::read_to_string(&target).expect("symlink target should remain readable"),
            ),
            (
                Some(format!("refusing to update symlink: {}", path.display())),
                target.clone(),
                "keep this target\n".to_string(),
            ),
        );
    }

    #[rstest]
    fn use_rejects_unconfigured_kind(configured_home: TempDir) {
        let command = ProfileCommands::Use(UseProfileArgs {
            kind: "missing_flow".to_string(),
            engine: Engine::Codex,
        });

        let result = execute_with_home(&configured_home, &command, None);

        assert_eq!(
            (
                result.err().map(|error| error.to_string()),
                selection_path(&configured_home).exists(),
            ),
            (
                Some(
                    "unknown agent profile kind `missing_flow`; configured launch profile kinds: sample_flow, second_flow"
                        .to_string(),
                ),
                false,
            ),
        );
    }

    #[rstest]
    fn show_rejects_kind_defined_only_by_local_selection(configured_home: TempDir) {
        fs::write(
            selection_path(&configured_home),
            indoc! {"
                agent:
                  launch_profiles:
                    retired_flow:
                      engine: codex
            "},
        )
        .expect("stale local selection should be written");
        let command = ProfileCommands::Show(ShowProfileArgs {
            kind: "retired_flow".to_string(),
        });

        let result = execute_with_home(&configured_home, &command, None);

        assert_eq!(
            result.err().map(|error| error.to_string()),
            Some(
                "unknown agent profile kind `retired_flow`; configured launch profile kinds: sample_flow, second_flow"
                    .to_string(),
            ),
        );
    }
}
