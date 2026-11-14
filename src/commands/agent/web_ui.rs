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
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct AddArgs {
    /// Local HTTP URL, including its port.
    pub url: String,

    /// Name shown in the watch-web status panel.
    #[arg(long)]
    pub name: String,
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
    }
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
    register(&mut session.web_uis, name, url);
    if record_legacy_crit_url {
        add_url(&mut session.crit_urls, url);
    }
    lock.save(&session)?;
    drop(lock);
    Ok(session)
}

pub(super) fn register(web_uis: &mut Vec<WebUiRegistration>, name: &str, url: &str) {
    let name = name.trim();
    web_uis.retain(|web_ui| web_ui.url != url && web_ui.name.trim() != name);
    web_uis.push(WebUiRegistration {
        name: name.to_owned(),
        url: url.to_owned(),
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

    use super::{local_web_ui_port, record_url_in, register, validate_name};
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
    fn sessions_dir_with_web_uis(sessions_dir: tempfile::TempDir) -> tempfile::TempDir {
        let mut session = store::load_session_from(sessions_dir.path(), "fixture-session")
            .expect("session should load")
            .expect("session should exist");
        session.web_uis = vec![
            WebUiRegistration {
                name: "Design editor".to_owned(),
                url: "http://localhost:48761/old".to_owned(),
            },
            WebUiRegistration {
                name: "Preview".to_owned(),
                url: "http://localhost:48761/preview".to_owned(),
            },
        ];
        store::save_session_to(sessions_dir.path(), &session)
            .expect("session with web UIs should be saved");
        sessions_dir
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
            },
            WebUiRegistration {
                name: "Preview".to_owned(),
                url: "http://localhost:48761/second".to_owned(),
            },
        ];

        register(&mut web_uis, "Design", "http://localhost:48761/first");

        assert_eq!(
            web_uis,
            vec![
                WebUiRegistration {
                    name: "Preview".to_owned(),
                    url: "http://localhost:48761/second".to_owned(),
                },
                WebUiRegistration {
                    name: "Design".to_owned(),
                    url: "http://localhost:48761/first".to_owned(),
                },
            ],
        );
    }

    #[rstest]
    fn recording_a_web_ui_replaces_the_same_trimmed_name_with_a_new_url(
        sessions_dir_with_web_uis: tempfile::TempDir,
    ) {
        let session = record_url_in(
            sessions_dir_with_web_uis.path(),
            "fixture-session",
            " Design editor ",
            "http://localhost:48761/new",
            false,
        )
        .expect("registered web UI should be saved");
        let stored = store::load_session_from(sessions_dir_with_web_uis.path(), "fixture-session")
            .expect("session should load")
            .expect("session should exist");
        let expected = vec![
            WebUiRegistration {
                name: "Preview".to_owned(),
                url: "http://localhost:48761/preview".to_owned(),
            },
            WebUiRegistration {
                name: "Design editor".to_owned(),
                url: "http://localhost:48761/new".to_owned(),
            },
        ];

        assert_eq!(
            (session.web_uis, stored.web_uis),
            (expected.clone(), expected)
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
}
