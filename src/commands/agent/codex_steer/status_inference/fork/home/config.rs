use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use chrono::{Duration as ChronoDuration, Utc};
use serde_json::Value;
use toml_edit::DocumentMut;

use super::super::FORK_TIMEOUT;
use super::MAX_SNAPSHOT_DURATION;

const CHATGPT_TOKEN_REFRESH_WINDOW: ChronoDuration = ChronoDuration::minutes(5);
const TOKEN_SNAPSHOT_MARGIN: ChronoDuration = ChronoDuration::seconds(30);

#[derive(Debug, PartialEq, Eq)]
pub(super) struct SourcePaths {
    pub(super) codex_home: PathBuf,
    pub(super) sqlite_home: PathBuf,
}

#[derive(Debug, Clone)]
struct SourceEnvironment {
    user_home: Option<PathBuf>,
    codex_home: Option<PathBuf>,
    sqlite_home: Option<PathBuf>,
}

pub(super) fn resolve_source(cwd: &Path) -> Result<SourcePaths> {
    let environment = SourceEnvironment {
        user_home: std::env::var_os("HOME").map(PathBuf::from),
        codex_home: std::env::var_os("CODEX_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
        sqlite_home: std::env::var("CODEX_SQLITE_HOME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
    };
    resolve_source_with_environment(cwd, &environment)
}

fn resolve_source_with_environment(
    cwd: &Path,
    environment: &SourceEnvironment,
) -> Result<SourcePaths> {
    let user_home = environment
        .user_home
        .as_ref()
        .context("HOME is unavailable for Codex status inference")?;
    let codex_home = environment
        .codex_home
        .clone()
        .unwrap_or_else(|| user_home.join(".codex"));
    let codex_home = resolve_path(&codex_home, cwd, user_home)
        .context("resolve source Codex home")?
        .canonicalize()
        .context("resolve source Codex home")?;
    ensure!(codex_home.is_dir(), "source Codex home is not a directory");

    let sqlite_env = environment.sqlite_home.clone();
    let configured_sqlite_home = read_configured_sqlite_home(&codex_home)?;
    let sqlite_home = resolve_sqlite_home(
        configured_sqlite_home,
        sqlite_env,
        &codex_home,
        cwd,
        user_home,
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
        let Some(contents) = read_private_file(&source, required)? else {
            continue;
        };
        let snapshot = if name == "auth.json" {
            private_auth_snapshot(&contents)?
        } else {
            contents
        };
        if json_object {
            let value: Value = serde_json::from_slice(&snapshot)
                .with_context(|| format!("parse source Codex {name} snapshot"))?;
            ensure!(
                value.is_object(),
                "source Codex credential snapshot is not a JSON object"
            );
        }
        write_private_file(&destination, &snapshot)?;
    }
    Ok(())
}

fn read_private_file(source: &Path, required: bool) -> Result<Option<Vec<u8>>> {
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
    Ok(Some(contents))
}

fn write_private_file(destination: &Path, contents: &[u8]) -> Result<()> {
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
    let mut contents = contents;
    io::copy(&mut contents, &mut destination_file).context("copy private Codex startup file")?;
    Ok(())
}

fn private_auth_snapshot(contents: &[u8]) -> Result<Vec<u8>> {
    let mut auth: Value =
        serde_json::from_slice(contents).context("parse source Codex auth snapshot")?;
    let auth_object = auth
        .as_object_mut()
        .context("source Codex auth snapshot is not a JSON object")?;
    if let Some(tokens) = auth_object.get_mut("tokens")
        && !tokens.is_null()
    {
        let token_object = tokens
            .as_object_mut()
            .context("source Codex token snapshot is not a JSON object")?;
        let access_token = token_object
            .get("access_token")
            .and_then(Value::as_str)
            .context("source Codex access token is unavailable")?;
        let expiration = jwt_expiration(access_token)?;
        let minimum_validity = CHATGPT_TOKEN_REFRESH_WINDOW
            + ChronoDuration::from_std(MAX_SNAPSHOT_DURATION)
                .context("Codex snapshot duration exceeds the token expiry range")?
            + ChronoDuration::from_std(FORK_TIMEOUT)
                .context("Codex fork timeout exceeds the token expiry range")?
            + TOKEN_SNAPSHOT_MARGIN;
        let minimum_expiration = Utc::now() + minimum_validity;
        ensure!(
            expiration > minimum_expiration,
            "source Codex access token expires too soon for status inference"
        );
        ensure!(
            token_object
                .get("refresh_token")
                .and_then(Value::as_str)
                .is_some(),
            "source Codex refresh token is unavailable"
        );
        token_object.insert("refresh_token".to_string(), Value::String(String::new()));
    }
    serde_json::to_vec(&auth).context("serialize private Codex auth snapshot")
}

fn jwt_expiration(jwt: &str) -> Result<chrono::DateTime<Utc>> {
    let mut parts = jwt.split('.');
    let header = parts
        .next()
        .context("source Codex access token has no header")?;
    let payload = parts
        .next()
        .context("source Codex access token has no claims")?;
    let signature = parts
        .next()
        .context("source Codex access token has no signature")?;
    ensure!(
        !header.is_empty()
            && !payload.is_empty()
            && !signature.is_empty()
            && parts.next().is_none(),
        "source Codex access token has an unsupported format"
    );
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .context("decode source Codex access token claims")?;
    let claims: Value =
        serde_json::from_slice(&payload).context("parse source Codex access token claims")?;
    let expiration = claims
        .get("exp")
        .and_then(Value::as_i64)
        .context("source Codex access token has no expiration")?;
    chrono::DateTime::<Utc>::from_timestamp(expiration, 0)
        .context("source Codex access token expiration is invalid")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use base64::Engine;
    use chrono::Utc;
    use rstest::rstest;
    use serde_json::{Value, json};

    use std::fs;

    use tempfile::TempDir;

    use super::{
        SourceEnvironment, SourcePaths, copy_startup_files, private_auth_snapshot, resolve_path,
        resolve_source_with_environment, resolve_sqlite_home,
    };

    fn access_token(expires_in_seconds: i64) -> String {
        let expiration = Utc::now().timestamp() + expires_in_seconds;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(json!({ "exp": expiration }).to_string());
        format!("fixture-header.{payload}.fixture-signature")
    }

    fn auth_with_access_token(access_token: String) -> Value {
        json!({
            "auth_mode": "chatgpt",
            "tokens": {
                "id_token": "fixture-id-token",
                "access_token": access_token,
                "refresh_token": "fixture-refresh-token",
                "account_id": "fixture-account",
            },
            "last_refresh": "2026-01-01T00:00:00Z",
        })
    }

    #[test]
    fn private_auth_snapshot_removes_the_refresh_token_without_changing_source_auth()
    -> anyhow::Result<()> {
        let root = TempDir::new()?;
        let source_home = root.path().join("source");
        let private_home = root.path().join("private");
        fs::create_dir_all(&source_home)?;
        fs::create_dir_all(&private_home)?;
        let source_auth = auth_with_access_token(access_token(3600));
        let source_bytes = serde_json::to_vec(&source_auth)?;
        fs::write(source_home.join("auth.json"), &source_bytes)?;

        copy_startup_files(&source_home, &private_home)?;

        let private_auth: Value =
            serde_json::from_slice(&fs::read(private_home.join("auth.json"))?)?;
        let mut expected_auth = source_auth;
        expected_auth["tokens"]["refresh_token"] = Value::String(String::new());
        assert_eq!(
            (private_auth, fs::read(source_home.join("auth.json"))?),
            (expected_auth, source_bytes),
        );
        Ok(())
    }

    #[rstest]
    #[case::expired(0)]
    #[case::near_expiry(500)]
    fn private_auth_snapshot_rejects_tokens_that_may_refresh_during_the_fork(
        #[case] expires_in_seconds: i64,
    ) {
        let auth = auth_with_access_token(access_token(expires_in_seconds));
        let result =
            private_auth_snapshot(&serde_json::to_vec(&auth).expect("fixture auth serializes"))
                .map(|_| ())
                .map_err(|_| ());

        assert_eq!(result, Err(()));
    }

    #[test]
    fn private_auth_snapshot_rejects_access_tokens_without_an_expiration() {
        let auth = auth_with_access_token("fixture-header.e30.fixture-signature".to_string());
        let result =
            private_auth_snapshot(&serde_json::to_vec(&auth).expect("fixture auth serializes"))
                .map(|_| ())
                .map_err(|_| ());

        assert_eq!(result, Err(()));
    }

    #[test]
    fn resolves_custom_codex_home_and_configured_sqlite_home_from_injected_environment()
    -> anyhow::Result<()> {
        let root = TempDir::new()?;
        let root_path = root.path().canonicalize()?;
        let user_home = root_path.join("user-home");
        let codex_home = root_path.join("custom-codex-home");
        let configured_sqlite_home = codex_home.join("configured-state");
        let environment_sqlite_home = root_path.join("environment-state");
        fs::create_dir_all(&user_home)?;
        fs::create_dir_all(&configured_sqlite_home)?;
        fs::create_dir_all(&environment_sqlite_home)?;
        fs::write(
            codex_home.join("config.toml"),
            "sqlite_home = \"configured-state\"\n",
        )?;

        let environment = SourceEnvironment {
            user_home: Some(user_home),
            codex_home: Some(codex_home.clone()),
            sqlite_home: Some(environment_sqlite_home),
        };
        let actual = resolve_source_with_environment(&root_path, &environment)?;

        assert_eq!(
            actual,
            SourcePaths {
                codex_home: codex_home.canonicalize()?,
                sqlite_home: configured_sqlite_home.canonicalize()?,
            },
        );
        Ok(())
    }

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
