use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use toml_edit::DocumentMut;

pub(super) struct SourcePaths {
    pub(super) codex_home: PathBuf,
    pub(super) sqlite_home: PathBuf,
}

pub(super) fn resolve_source(cwd: &Path) -> Result<SourcePaths> {
    let user_home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is unavailable for Codex status inference")?;
    let codex_home = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| user_home.join(".codex"));
    let codex_home = resolve_path(&codex_home, cwd, &user_home)
        .context("resolve source Codex home")?
        .canonicalize()
        .context("resolve source Codex home")?;
    ensure!(codex_home.is_dir(), "source Codex home is not a directory");

    let sqlite_env = std::env::var("CODEX_SQLITE_HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    let configured_sqlite_home = read_configured_sqlite_home(&codex_home)?;
    let sqlite_home = resolve_sqlite_home(
        configured_sqlite_home,
        sqlite_env,
        &codex_home,
        cwd,
        &user_home,
    )?
    .canonicalize()
    .context("resolve source Codex SQLite home")?;
    ensure!(
        sqlite_home.is_dir(),
        "source Codex SQLite home is not a directory"
    );

    Ok(SourcePaths {
        codex_home,
        sqlite_home,
    })
}

fn read_configured_sqlite_home(codex_home: &Path) -> Result<Option<PathBuf>> {
    let path = codex_home.join("config.toml");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("inspect source Codex configuration"),
    };
    ensure!(
        metadata.file_type().is_file(),
        "source Codex configuration is not a regular file"
    );
    let contents = fs::read_to_string(path).context("read source Codex configuration")?;
    let document = contents
        .parse::<DocumentMut>()
        .context("parse source Codex configuration")?;
    let Some(item) = document.get("sqlite_home") else {
        return Ok(None);
    };
    let value = item
        .as_str()
        .context("source Codex sqlite_home is not a string")?;
    ensure!(
        !value.trim().is_empty(),
        "source Codex sqlite_home is empty"
    );
    Ok(Some(PathBuf::from(value)))
}

fn resolve_sqlite_home(
    configured: Option<PathBuf>,
    environment: Option<PathBuf>,
    codex_home: &Path,
    cwd: &Path,
    user_home: &Path,
) -> Result<PathBuf> {
    let (path, base) = match (configured, environment) {
        (Some(path), _) => (path, codex_home),
        (None, Some(path)) => (path, cwd),
        (None, None) => (codex_home.to_path_buf(), codex_home),
    };
    resolve_path(&path, base, user_home).context("resolve source Codex SQLite home")
}

fn resolve_path(path: &Path, base: &Path, user_home: &Path) -> Result<PathBuf> {
    let raw = path.to_str().context("Codex path is not valid UTF-8")?;
    if raw == "~" {
        return Ok(user_home.to_path_buf());
    }
    if let Some(suffix) = raw.strip_prefix("~/") {
        return Ok(user_home.join(suffix));
    }
    ensure!(
        !raw.starts_with('~'),
        "Codex path uses an unsupported home expansion"
    );
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    })
}

pub(super) fn copy_startup_files(source_home: &Path, private_home: &Path) -> Result<()> {
    for (name, required, json_object) in [
        ("config.toml", false, false),
        ("managed_config.toml", false, false),
        ("auth.json", true, true),
        (".credentials.json", false, true),
    ] {
        let source = source_home.join(name);
        let destination = private_home.join(name);
        let Some(contents) = copy_private_file(&source, &destination, required)? else {
            continue;
        };
        if json_object {
            let value: Value = serde_json::from_slice(&contents)
                .with_context(|| format!("parse source Codex {name} snapshot"))?;
            ensure!(
                value.is_object(),
                "source Codex credential snapshot is not a JSON object"
            );
        }
    }
    Ok(())
}

fn copy_private_file(source: &Path, destination: &Path, required: bool) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound && !required => return Ok(None),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            bail!("file-backed Codex authentication snapshot is unavailable")
        }
        Err(error) => return Err(error).context("inspect source Codex startup file"),
    };
    ensure!(
        metadata.file_type().is_file(),
        "source Codex startup file is not a regular file"
    );
    let contents = fs::read(source).context("read source Codex startup file")?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut destination_file = options
        .open(destination)
        .context("create private Codex startup file")?;
    io::copy(&mut contents.as_slice(), &mut destination_file)
        .context("copy private Codex startup file")?;
    Ok(Some(contents))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use rstest::rstest;

    use super::{resolve_path, resolve_sqlite_home};

    #[rstest]
    #[case::absolute_path(
        Path::new("/var/tmp/codex-state"),
        Path::new("/work"),
        Path::new("/users/example"),
        Path::new("/var/tmp/codex-state")
    )]
    #[case::relative_path(
        Path::new("codex-state"),
        Path::new("/work"),
        Path::new("/users/example"),
        Path::new("/work/codex-state")
    )]
    #[case::home_path(
        Path::new("~/.codex/state"),
        Path::new("/work"),
        Path::new("/users/example"),
        Path::new("/users/example/.codex/state")
    )]
    fn resolves_supported_paths(
        #[case] path: &Path,
        #[case] base: &Path,
        #[case] user_home: &Path,
        #[case] expected: &Path,
    ) {
        assert_eq!(
            resolve_path(path, base, user_home).expect("fixture path should resolve"),
            expected.to_path_buf(),
        );
    }

    #[rstest]
    #[case::configuration_overrides_environment(
        Some(PathBuf::from("configured-state")),
        Some(PathBuf::from("environment-state")),
        Path::new("/users/example/.codex/configured-state")
    )]
    #[case::environment_is_relative_to_working_directory(
        None,
        Some(PathBuf::from("environment-state")),
        Path::new("/work/environment-state")
    )]
    #[case::default_is_codex_home(None, None, Path::new("/users/example/.codex"))]
    fn selects_source_sqlite_home(
        #[case] configured: Option<PathBuf>,
        #[case] environment: Option<PathBuf>,
        #[case] expected: &Path,
    ) {
        let actual = resolve_sqlite_home(
            configured,
            environment,
            Path::new("/users/example/.codex"),
            Path::new("/work"),
            Path::new("/users/example"),
        )
        .expect("fixture SQLite home should resolve");

        assert_eq!(actual, expected.to_path_buf());
    }
}
