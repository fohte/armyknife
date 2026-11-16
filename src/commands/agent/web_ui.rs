use std::path::Path;

use anyhow::{Context, Result, ensure};
use clap::{Args, Subcommand};

use super::store;
use super::types::{Session, WebUiRegistration};
use crate::commands::agent::crit::url::add_url;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum WebUiCommands {
    /// Associate a named local web interface with the calling agent session.
    Add(AddArgs),
    /// Remove a named web interface from the calling agent session.
    Remove(RemoveArgs),
    /// List web interfaces registered by the calling agent session.
    List,
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct AddArgs {
    /// Local HTTP URL, including its port.
    pub url: String,

    /// Name shown in the watch-web status panel.
    #[arg(long)]
    pub name: String,
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct RemoveArgs {
    /// Name of the web interface to remove.
    pub name: String,
}

pub(crate) fn session_start_instruction() -> &'static str {
    "When you start a local web UI (an HTTP server with a port), register it with `a agent web-ui add <URL> --name <NAME>`."
}

pub fn run(command: &WebUiCommands) -> Result<()> {
    match command {
        WebUiCommands::Add(args) => {
            validate_name(&args.name)?;
            let session_id = crate::shared::env_var::EnvVars::load()
                .own_session_id()
                .context("A tracked agent session is required to register a web UI")?;
            record_url(&session_id, &args.name, &args.url, false)?;
            println!("Recorded web UI '{}'", args.name.trim());
            Ok(())
        }
        WebUiCommands::Remove(args) => {
            validate_name(&args.name)?;
            let session_id = crate::shared::env_var::EnvVars::load()
                .own_session_id()
                .context("A tracked agent session is required to remove a web UI")?;
            remove_web_ui(&session_id, &args.name)?;
            println!("Removed web UI '{}'", args.name.trim());
            Ok(())
        }
        WebUiCommands::List => {
            let session_id = crate::shared::env_var::EnvVars::load()
                .own_session_id()
                .context("A tracked agent session is required to list web UIs")?;
            let web_uis = list_web_uis(&session_id)?;
            println!("{}", format_web_ui_list(&web_uis));
            Ok(())
        }
    }
}

fn remove_web_ui(session_id: &str, name: &str) -> Result<Session> {
    remove_web_ui_in(&store::sessions_dir()?, session_id, name)
}

fn remove_web_ui_in(sessions_dir: &Path, session_id: &str, name: &str) -> Result<Session> {
    validate_name(name)?;
    let name = name.trim();
    let lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let Some(mut session) = lock.load()? else {
        anyhow::bail!("Agent session not found: {session_id}");
    };
    let original_len = session.web_uis.len();
    session.web_uis.retain(|web_ui| web_ui.name.trim() != name);
    anyhow::ensure!(
        session.web_uis.len() != original_len,
        "Web UI '{}' is not registered",
        name
    );
    lock.save(&session)?;
    Ok(session)
}

fn list_web_uis(session_id: &str) -> Result<Vec<WebUiRegistration>> {
    list_web_uis_in(&store::sessions_dir()?, session_id)
}

fn list_web_uis_in(sessions_dir: &Path, session_id: &str) -> Result<Vec<WebUiRegistration>> {
    let Some(session) = store::load_session_from(sessions_dir, session_id)? else {
        anyhow::bail!("Agent session not found: {session_id}");
    };
    Ok(session.web_uis)
}

fn format_web_ui_list(web_uis: &[WebUiRegistration]) -> String {
    if web_uis.is_empty() {
        return "No web UIs registered.".to_owned();
    }

    web_uis
        .iter()
        .map(|web_ui| {
            format!(
                "{}: {}",
                web_ui.name.escape_debug(),
                web_ui.url.escape_debug()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn record_url(
    session_id: &str,
    name: &str,
    url: &str,
    record_legacy_crit_url: bool,
) -> Result<Session> {
    record_url_in(
        &store::sessions_dir()?,
        session_id,
        name,
        url,
        record_legacy_crit_url,
    )
}

fn record_url_in(
    sessions_dir: &Path,
    session_id: &str,
    name: &str,
    url: &str,
    record_legacy_crit_url: bool,
) -> Result<Session> {
    if record_legacy_crit_url {
        ensure!(!name.trim().is_empty(), "Web UI name cannot be empty");
    } else {
        validate_name(name)?;
        local_web_ui_port(url)?;
    }

    let lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let Some(mut session) = lock.load()? else {
        anyhow::bail!("Agent session not found: {session_id}");
    };
    if record_legacy_crit_url {
        add_url(&mut session.crit_urls, url);
    }
    let registered_url = if record_legacy_crit_url {
        session.crit_urls.last().map(String::as_str).unwrap_or(url)
    } else {
        url
    }
    .to_owned();
    register(&mut session.web_uis, name, &registered_url);
    lock.save(&session)?;
    Ok(session)
}

pub(super) fn register(web_uis: &mut Vec<WebUiRegistration>, name: &str, url: &str) {
    let name = name.trim();
    let exposed_url = web_uis
        .iter()
        .find(|web_ui| web_ui.name.trim() == name && web_ui.url == url)
        .and_then(|web_ui| web_ui.exposed_url.clone());
    web_uis.retain(|web_ui| web_ui.url != url && web_ui.name.trim() != name);
    web_uis.push(WebUiRegistration {
        name: name.to_owned(),
        url: url.to_owned(),
        exposed_url,
    });
}

pub(super) fn validate_name(name: &str) -> Result<()> {
    ensure!(!name.trim().is_empty(), "Web UI name cannot be empty");
    ensure!(
        !name.trim().eq_ignore_ascii_case("crit"),
        "Web UI name 'crit' is reserved"
    );
    Ok(())
}

pub(super) fn local_web_ui_port(url: &str) -> Result<u16> {
    let parsed = reqwest::Url::parse(url).with_context(|| format!("Invalid web UI URL: {url}"))?;
    ensure!(
        parsed.scheme() == "http"
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && matches!(parsed.host_str(), Some("localhost" | "127.0.0.1")),
        "Web UI URL must use local HTTP (localhost or 127.0.0.1): {url}"
    );
    let port = parsed
        .port()
        .with_context(|| format!("Web UI URL has no explicit port: {url}"))?;
    ensure!(port != 0, "Web UI URL has an invalid port: {url}");
    Ok(port)
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use rstest::{fixture, rstest};
    use serde_json::json;

    use super::{
        format_web_ui_list, list_web_uis_in, local_web_ui_port, record_url_in, register,
        remove_web_ui_in, validate_name,
    };
    use crate::cli::{Cli, Commands};
    use crate::commands::agent::store;
    use crate::commands::agent::types::Session;
    use crate::commands::agent::types::WebUiRegistration;
    use crate::commands::agent::{AgentCommands, WebUiCommands};

    #[fixture]
    fn sessions_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("session directory should be created");
        let session: Session = serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture-worktree",
            "status": "running",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }))
        .expect("fixture session should deserialize");
        store::save_session_to(dir.path(), &session).expect("fixture session should be saved");
        dir
    }

    #[fixture]
    fn sessions_dir_with_web_uis() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("session directory should be created");
        let session: Session = serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture-worktree",
            "status": "running",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "web_uis": [
                {"name": "Sample editor", "url": "http://localhost:49152/sample/", "exposedUrl": null},
                {"name": "Secondary preview", "url": "http://localhost:49153/preview/", "exposedUrl": "https://preview.example.invalid/preview/"}
            ]
        }))
        .expect("fixture session should deserialize");
        store::save_session_to(dir.path(), &session).expect("fixture session should be saved");
        dir
    }

    #[rstest]
    #[case::loopback_ipv4("http://127.0.0.1:48761/design/sample/", Some(48761))]
    #[case::localhost("http://localhost:48761/design/sample/", Some(48761))]
    #[case::missing_port("http://localhost/design/sample/", None)]
    #[case::remote_host("http://example.test:48761/design/sample/", None)]
    #[case::secure_scheme("https://localhost:48761/design/sample/", None)]
    #[case::userinfo("http://user@localhost:48761/design/sample/", None)]
    fn validates_local_web_ui_urls(#[case] url: &str, #[case] expected: Option<u16>) {
        assert_eq!(local_web_ui_port(url).ok(), expected);
    }

    #[rstest]
    #[case::empty("")]
    #[case::whitespace(" \t ")]
    #[case::reserved("crit")]
    #[case::reserved_case_insensitive("Crit")]
    fn rejects_invalid_names(#[case] name: &str) {
        assert!(validate_name(name).is_err());
    }

    #[test]
    fn registering_an_existing_url_renames_and_promotes_it() {
        let mut web_uis = vec![
            WebUiRegistration {
                name: "Editor".to_owned(),
                url: "http://localhost:48761/first".to_owned(),
                exposed_url: None,
            },
            WebUiRegistration {
                name: "Preview".to_owned(),
                url: "http://localhost:48761/second".to_owned(),
                exposed_url: None,
            },
        ];

        register(&mut web_uis, "Design", "http://localhost:48761/first");

        assert_eq!(
            web_uis,
            vec![
                WebUiRegistration {
                    name: "Preview".to_owned(),
                    url: "http://localhost:48761/second".to_owned(),
                    exposed_url: None,
                },
                WebUiRegistration {
                    name: "Design".to_owned(),
                    url: "http://localhost:48761/first".to_owned(),
                    exposed_url: None,
                },
            ],
        );
    }

    #[test]
    fn registering_the_same_interface_preserves_its_exposed_url() {
        let mut web_uis = vec![WebUiRegistration {
            name: "Design editor".to_owned(),
            url: "http://localhost:48761/design/sample/".to_owned(),
            exposed_url: Some("https://public.example.invalid/design/sample/".to_owned()),
        }];

        register(
            &mut web_uis,
            "Design editor",
            "http://localhost:48761/design/sample/",
        );

        assert_eq!(
            web_uis,
            vec![WebUiRegistration {
                name: "Design editor".to_owned(),
                url: "http://localhost:48761/design/sample/".to_owned(),
                exposed_url: Some("https://public.example.invalid/design/sample/".to_owned()),
            }],
        );
    }

    #[rstest]
    #[case::web_ui_local(
        "Design editor",
        false,
        "http://127.0.0.1:4721/design/sample/",
        Vec::new()
    )]
    #[case::crit_local(
        "crit",
        true,
        "http://127.0.0.1:4721/design/sample/",
        vec!["http://127.0.0.1:4721/design/sample/"]
    )]
    #[case::crit_remote(
        "crit",
        true,
        "http://review.example.test:4721/review/sample",
        vec!["http://review.example.test:4721/review/sample"]
    )]
    fn records_named_web_uis_for_the_session(
        sessions_dir: tempfile::TempDir,
        #[case] name: &str,
        #[case] record_legacy_crit_url: bool,
        #[case] url: &str,
        #[case] expected_crit_urls: Vec<&str>,
    ) {
        let session = record_url_in(
            sessions_dir.path(),
            "fixture-session",
            name,
            url,
            record_legacy_crit_url,
        )
        .expect("registered web UI should be saved");

        let stored = store::load_session_from(sessions_dir.path(), "fixture-session")
            .expect("session should load")
            .expect("session should exist");
        let expected_crit_urls = expected_crit_urls
            .iter()
            .map(|url| (*url).to_owned())
            .collect::<Vec<_>>();
        let expected_web_uis = vec![WebUiRegistration {
            name: name.to_owned(),
            url: url.to_owned(),
            exposed_url: None,
        }];

        assert_eq!(
            (
                session.crit_urls,
                session.web_uis,
                stored.crit_urls,
                stored.web_uis,
            ),
            (
                expected_crit_urls.clone(),
                expected_web_uis.clone(),
                expected_crit_urls,
                expected_web_uis,
            ),
        );
    }

    #[test]
    fn parses_named_web_ui_registration_command() {
        let parsed = Cli::try_parse_from([
            "a",
            "agent",
            "web-ui",
            "add",
            "http://127.0.0.1:4721/design/sample/",
            "--name",
            "Design editor",
        ])
        .expect("web UI registration command should parse");
        let Commands::Agent(AgentCommands::WebUi(WebUiCommands::Add(args))) = parsed.command else {
            panic!("expected web UI registration command");
        };

        assert_eq!(
            (args.url.as_str(), args.name.as_str()),
            ("http://127.0.0.1:4721/design/sample/", "Design editor"),
        );
    }

    #[rstest]
    fn removes_a_named_web_ui_and_preserves_other_registrations(
        sessions_dir_with_web_uis: tempfile::TempDir,
    ) {
        let removed = remove_web_ui_in(
            sessions_dir_with_web_uis.path(),
            "fixture-session",
            " Sample editor ",
        )
        .expect("named web UI should be removed");
        let stored = store::load_session_from(sessions_dir_with_web_uis.path(), "fixture-session")
            .expect("session should load")
            .expect("session should exist");
        let expected = vec![WebUiRegistration {
            name: "Secondary preview".to_owned(),
            url: "http://localhost:49153/preview/".to_owned(),
            exposed_url: Some("https://preview.example.invalid/preview/".to_owned()),
        }];

        assert_eq!(
            (removed.web_uis, stored.web_uis),
            (expected.clone(), expected)
        );
    }

    #[rstest]
    fn removing_an_unregistered_web_ui_returns_an_error(
        sessions_dir_with_web_uis: tempfile::TempDir,
    ) {
        let result = remove_web_ui_in(
            sessions_dir_with_web_uis.path(),
            "fixture-session",
            "Missing",
        );

        assert_eq!(
            result
                .expect_err("unregistered web UI should fail")
                .to_string(),
            "Web UI 'Missing' is not registered",
        );
    }

    #[rstest]
    fn lists_the_calling_sessions_web_uis_with_names_and_urls(
        sessions_dir_with_web_uis: tempfile::TempDir,
    ) {
        let web_uis = list_web_uis_in(sessions_dir_with_web_uis.path(), "fixture-session")
            .expect("registered web UIs should be listed");

        assert_eq!(
            format_web_ui_list(&web_uis),
            "Sample editor: http://localhost:49152/sample/\nSecondary preview: http://localhost:49153/preview/",
        );
    }

    #[test]
    fn formats_an_empty_web_ui_list() {
        assert_eq!(format_web_ui_list(&[]), "No web UIs registered.");
    }

    #[test]
    fn escapes_terminal_control_characters_in_web_ui_names() {
        let web_uis = vec![WebUiRegistration {
            name: "Sample\u{1b}[2J interface\n".to_owned(),
            url: "http://localhost:49152/example/".to_owned(),
            exposed_url: None,
        }];

        assert_eq!(
            format_web_ui_list(&web_uis),
            "Sample\\u{1b}[2J interface\\n: http://localhost:49152/example/",
        );
    }

    #[test]
    fn parses_web_ui_remove_command() {
        let remove = Cli::try_parse_from(["a", "agent", "web-ui", "remove", "Sample editor"])
            .expect("web UI remove command should parse");
        let Commands::Agent(AgentCommands::WebUi(WebUiCommands::Remove(args))) = remove.command
        else {
            panic!("expected web UI remove command");
        };
        assert_eq!(args.name, "Sample editor");
    }

    #[test]
    fn parses_web_ui_list_command() {
        let list = Cli::try_parse_from(["a", "agent", "web-ui", "list"])
            .expect("web UI list command should parse");
        assert!(matches!(
            list.command,
            Commands::Agent(AgentCommands::WebUi(WebUiCommands::List))
        ));
    }
}
