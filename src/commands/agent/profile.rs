use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use clap::{Args, Subcommand};
use serde_yaml::{Mapping, Value};

use crate::infra::git::file_lock::{lock_exclusive, open_lock_file};
use crate::shared::config::{self, Config, format_sorted_names};

const SELECTION_FILE: &str = "zz-agent-profiles.local.yaml";
const SELECTION_LOCK_FILE: &str = "zz-agent-profiles.lock";

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum ProfileCommands {
    /// Select the profile used by `a agent new --type`
    Use(UseProfileArgs),

    /// Show the selected profile and its type settings
    Show,
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct UseProfileArgs {
    /// Configured profile name
    pub profile: String,
}

pub fn run(command: &ProfileCommands) -> Result<()> {
    print!("{}", execute(command)?);
    Ok(())
}

fn execute(command: &ProfileCommands) -> Result<String> {
    let config_dir = config_dir()?;
    let selection_path = config_dir.join(SELECTION_FILE);
    if let ProfileCommands::Use(args) = command {
        reject_symlink(&selection_path)?;
        let configured_profiles = config::load_config_without_file(SELECTION_FILE)?;
        ensure_profile_exists(&configured_profiles, &args.profile)?;
        save_profile_selection(&config_dir, &args.profile)?;
    }
    let config = config::load_config()?;
    format_profile(&config)
}

fn config_dir() -> Result<PathBuf> {
    crate::shared::dirs::config_dir()
        .map(|path| path.join("armyknife"))
        .context("configuration directory is unavailable")
}

fn ensure_profile_exists(config: &Config, profile_name: &str) -> Result<()> {
    ensure!(
        config.agent.profiles.contains_key(profile_name),
        "unknown agent profile `{profile_name}`; configured profiles: {}",
        format_sorted_names(config.agent.profiles.keys()),
    );
    Ok(())
}

fn format_profile(config: &Config) -> Result<String> {
    let Some(profile_name) = config.agent.active_profile.as_deref() else {
        return Ok("No agent profile is selected.\n".to_string());
    };
    let profile = config.agent.profiles.get(profile_name).with_context(|| {
        format!("agent.active_profile references undefined profile `{profile_name}`")
    })?;
    let mut types = profile.iter().collect::<Vec<_>>();
    types.sort_unstable_by(|left, right| left.0.cmp(right.0));

    let mut output = format!("Active profile: {profile_name}\n");
    if types.is_empty() {
        output.push_str("  types: (none)\n");
    }
    for (type_name, settings) in types {
        output.push_str(&format!(
            "  type: {type_name}\n    engine: {}\n    model: {}\n    reasoning effort: {}\n",
            settings.engine.process_name(),
            settings.model,
            settings.reasoning_effort.as_str(),
        ));
    }
    Ok(output)
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

fn save_profile_selection(config_dir: &Path, profile_name: &str) -> Result<()> {
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
    set_active_profile(&mut document, profile_name)?;

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

fn set_active_profile(document: &mut Value, profile_name: &str) -> Result<()> {
    let agent = mapping_child(document, "agent")?;
    let selection = mapping_child(agent, "active_profile")?;
    *selection = Value::String(profile_name.to_string());
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
                  profiles:
                    \"profile one\":
                      build:
                        engine: claude
                        model: model-raven
                        reasoning_effort: low
                      \"review type\":
                        engine: codex
                        model: model-ember
                        reasoning_effort: high
                    profile-two:
                      inspect:
                        engine: claude
                        model: model-slate
                        reasoning_effort: medium
            "},
        )
        .expect("profile config should be written");
        home
    }

    fn selection_path(home: &TempDir) -> PathBuf {
        home.path().join("armyknife").join(SELECTION_FILE)
    }

    fn execute_with_home(home: &TempDir, command: &ProfileCommands) -> Result<String> {
        let config_home = home
            .path()
            .to_str()
            .expect("config home path should be valid UTF-8");
        temp_env::with_vars(
            [
                ("XDG_CONFIG_HOME", Some(config_home)),
                ("ARMYKNIFE_AGENT__ACTIVE_PROFILE", None::<&str>),
            ],
            || execute(command),
        )
    }

    #[test]
    fn profile_commands_parse_arbitrary_names_and_show_without_arguments() {
        let parsed = vec![
            TestCli::try_parse_from(["a", "profile", "use", "profile one"])
                .expect("profile use should parse")
                .command,
            TestCli::try_parse_from(["a", "profile", "show"])
                .expect("profile show should parse")
                .command,
        ];
        let expected = vec![
            AgentCommands::Profile(ProfileCommands::Use(UseProfileArgs {
                profile: "profile one".to_string(),
            })),
            AgentCommands::Profile(ProfileCommands::Show),
        ];

        assert!(parsed == expected);
    }

    #[rstest]
    fn use_saves_only_active_profile_and_preserves_other_local_settings(configured_home: TempDir) {
        let path = selection_path(&configured_home);
        fs::write(
            &path,
            indoc! {"
                agent:
                  active_profile: profile-two
                  default_engine: codex
                  codex:
                    model: model-local
            "},
        )
        .expect("existing local settings should be written");
        let command = ProfileCommands::Use(UseProfileArgs {
            profile: "profile one".to_string(),
        });

        execute_with_home(&configured_home, &command).expect("profile selection should be saved");
        let saved: Value = serde_yaml::from_str(
            &fs::read_to_string(path).expect("local selection file should be readable"),
        )
        .expect("saved selection should be valid YAML");
        let expected: Value = serde_yaml::from_str(indoc! {"
            agent:
              active_profile: profile one
              default_engine: codex
              codex:
                model: model-local
        "})
        .expect("expected selection should be valid YAML");

        assert_eq!(saved, expected);
    }

    #[rstest]
    #[tokio::test(flavor = "multi_thread")]
    async fn profile_use_dispatch_persists_active_profile(configured_home: TempDir) {
        let cli = TestCli::try_parse_from(["a", "profile", "use", "profile one"])
            .expect("profile command should parse");
        let config_home = configured_home
            .path()
            .to_str()
            .expect("config home path should be valid UTF-8");

        let result = temp_env::with_vars(
            [
                ("XDG_CONFIG_HOME", Some(config_home)),
                ("ARMYKNIFE_AGENT__ACTIVE_PROFILE", None::<&str>),
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
              active_profile: profile one
        "})
        .expect("expected selection should be valid YAML");

        assert_eq!((result, saved), (None, expected));
    }

    #[rstest]
    fn show_reports_selected_profile_and_all_types(configured_home: TempDir) {
        fs::write(
            selection_path(&configured_home),
            indoc! {"
                agent:
                  active_profile: profile one
            "},
        )
        .expect("profile selection should be written");

        let output = execute_with_home(&configured_home, &ProfileCommands::Show)
            .expect("profile settings should be shown");

        assert_eq!(
            output,
            indoc! {"
                Active profile: profile one
                  type: build
                    engine: claude
                    model: model-raven
                    reasoning effort: low
                  type: review type
                    engine: codex
                    model: model-ember
                    reasoning effort: high
            "},
        );
    }

    #[rstest]
    fn show_reports_when_no_profile_is_selected(configured_home: TempDir) {
        let output = execute_with_home(&configured_home, &ProfileCommands::Show)
            .expect("profile status should be shown");

        assert_eq!(output, "No agent profile is selected.\n");
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
            profile: "profile one".to_string(),
        });
        let config_home = home
            .path()
            .to_str()
            .expect("config home path should be valid UTF-8");
        let result = temp_env::with_vars(
            [
                ("XDG_CONFIG_HOME", Some(config_home)),
                ("ARMYKNIFE_AGENT__ACTIVE_PROFILE", None::<&str>),
            ],
            || execute(&command),
        );

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
    fn use_rejects_unconfigured_profile(configured_home: TempDir) {
        let command = ProfileCommands::Use(UseProfileArgs {
            profile: "missing profile".to_string(),
        });

        let result = execute_with_home(&configured_home, &command);

        assert_eq!(
            (
                result.err().map(|error| error.to_string()),
                selection_path(&configured_home).exists(),
            ),
            (
                Some(
                    "unknown agent profile `missing profile`; configured profiles: profile one, profile-two"
                        .to_string(),
                ),
                false,
            ),
        );
    }

    #[rstest]
    fn show_rejects_undefined_active_profile(configured_home: TempDir) {
        fs::write(
            selection_path(&configured_home),
            indoc! {"
                agent:
                  active_profile: removed-profile
            "},
        )
        .expect("invalid profile selection should be written");

        let result = execute_with_home(&configured_home, &ProfileCommands::Show);

        assert_eq!(
            result.err().map(|error| error.to_string()),
            Some("agent.active_profile references undefined profile `removed-profile`".to_string()),
        );
    }
}
