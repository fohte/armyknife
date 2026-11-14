use std::path::Path;

use anyhow::{Context, Result, ensure};
use clap::{Args, Subcommand};

use super::store;
use super::types::{Session, SessionStatus, WebUiRegistration};
use crate::commands::agent::crit::url::add_url;
use crate::shared::hooks;

const WEB_UI_URL_ENV: &str = "ARMYKNIFE_WEB_UI_URL";
const WEB_UI_PORT_ENV: &str = "ARMYKNIFE_WEB_UI_PORT";
const WEB_UI_EXPOSED_URL_ENV: &str = "ARMYKNIFE_WEB_UI_EXPOSED_URL";

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
    record_url_in_with_hooks(
        &store::sessions_dir()?,
        session_id,
        name,
        url,
        record_legacy_crit_url,
        hooks::run_hook_capture_stdout,
    )
}

fn record_url_in_with_hooks(
    sessions_dir: &Path,
    session_id: &str,
    name: &str,
    url: &str,
    record_legacy_crit_url: bool,
    mut run_hook: impl FnMut(&str, &[(&str, &str)]) -> anyhow::Result<Option<String>>,
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
    let previous_urls = session.web_uis.clone();
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
    drop(lock);

    for previous in previous_urls.iter().filter(|previous| {
        !session
            .web_uis
            .iter()
            .any(|current| current.name == previous.name && current.url == previous.url)
    }) {
        run_unexpose_hook(previous, &mut run_hook);
    }

    let exposed_url = match web_ui_port(&registered_url) {
        Some(port) => {
            let port = port.to_string();
            match run_hook(
                "web-ui-expose",
                &[(WEB_UI_URL_ENV, &registered_url), (WEB_UI_PORT_ENV, &port)],
            ) {
                Ok(url) => exposed_web_ui_url(url),
                Err(error) => {
                    tracing::warn!(
                        event = "agent.web_ui.expose_failed",
                        url = %registered_url,
                        error = %error,
                        "web-ui-expose hook failed"
                    );
                    None
                }
            }
        }
        None => None,
    };

    if let Some(registered) = session
        .web_uis
        .iter_mut()
        .find(|registered| registered.name == name.trim() && registered.url == registered_url)
    {
        registered.exposed_url = exposed_url.clone();
    }

    let exposed_registration = WebUiRegistration {
        name: name.trim().to_owned(),
        url: registered_url.clone(),
        exposed_url,
    };
    let update_result =
        (|| -> Result<Option<Session>> {
            let lock = store::lock_session_for_update(sessions_dir, session_id)?;
            let Some(mut current) = lock.load()? else {
                return Ok(None);
            };
            if current.status == SessionStatus::Ended {
                return Ok(Some(current));
            }
            let Some(registered) = current.web_uis.iter_mut().find(|registered| {
                registered.name == name.trim() && registered.url == registered_url
            }) else {
                return Ok(Some(current));
            };
            registered.exposed_url = exposed_registration.exposed_url.clone();
            lock.save(&current)?;
            Ok(Some(current))
        })();

    match update_result {
        Ok(Some(current))
            if current.status != SessionStatus::Ended
                && current.web_uis.iter().any(|registered| {
                    registered.name == name.trim() && registered.url == registered_url
                }) =>
        {
            session = current;
        }
        Ok(Some(current)) => {
            run_unexpose_hook(&exposed_registration, &mut run_hook);
            return Ok(current);
        }
        Ok(None) => {
            run_unexpose_hook(&exposed_registration, &mut run_hook);
            return Ok(session);
        }
        Err(error) => {
            run_unexpose_hook(&exposed_registration, &mut run_hook);
            return Err(error);
        }
    }
    Ok(session)
}

pub(super) fn register(web_uis: &mut Vec<WebUiRegistration>, name: &str, url: &str) {
    let name = name.trim();
    web_uis.retain(|web_ui| web_ui.url != url && web_ui.name.trim() != name);
    web_uis.push(WebUiRegistration {
        name: name.to_owned(),
        url: url.to_owned(),
        exposed_url: None,
    });
}

#[cfg(not(test))]
pub(super) fn unexpose_session_web_uis(session: &Session) {
    for web_ui in &session.web_uis {
        unexpose_web_ui(web_ui);
    }
}

#[cfg(not(test))]
pub(super) fn unexpose_web_ui(web_ui: &WebUiRegistration) {
    let mut run_hook = hooks::run_hook_capture_stdout;
    run_unexpose_hook(web_ui, &mut run_hook);
}

fn run_unexpose_hook(
    web_ui: &WebUiRegistration,
    run_hook: &mut impl FnMut(&str, &[(&str, &str)]) -> anyhow::Result<Option<String>>,
) {
    let Some(port) = web_ui_port(&web_ui.url) else {
        return;
    };
    let port = port.to_string();
    let exposed_url = web_ui.exposed_url.as_deref().unwrap_or_default();
    if let Err(error) = run_hook(
        "web-ui-unexpose",
        &[
            (WEB_UI_URL_ENV, &web_ui.url),
            (WEB_UI_PORT_ENV, &port),
            (WEB_UI_EXPOSED_URL_ENV, exposed_url),
        ],
    ) {
        tracing::warn!(
            event = "agent.web_ui.unexpose_failed",
            url = %web_ui.url,
            error = %error,
            "web-ui-unexpose hook failed"
        );
    }
}

fn web_ui_port(url: &str) -> Option<u16> {
    let parsed = reqwest::Url::parse(url).ok()?;
    matches!(parsed.scheme(), "http" | "https")
        .then(|| parsed.port_or_known_default())
        .flatten()
}

fn exposed_web_ui_url(url: Option<String>) -> Option<String> {
    let url = url?;
    let url = url.trim();
    if url.is_empty() {
        return None;
    }

    let parsed = reqwest::Url::parse(url).ok()?;
    matches!(parsed.scheme(), "http" | "https").then(|| url.to_owned())
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

    use super::{local_web_ui_port, record_url_in_with_hooks, register, validate_name};
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
                exposed_url: Some("https://public.example.test/old".to_owned()),
            },
            WebUiRegistration {
                name: "Preview".to_owned(),
                url: "http://localhost:48761/preview".to_owned(),
                exposed_url: None,
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

    #[rstest]
    #[case::unexpose_succeeds(false)]
    #[case::unexpose_fails(true)]
    fn recording_a_web_ui_replaces_the_same_trimmed_name_with_a_new_url(
        sessions_dir_with_web_uis: tempfile::TempDir,
        #[case] fail_unexpose: bool,
    ) {
        let mut calls = Vec::<(String, Vec<(String, String)>)>::new();
        let session = record_url_in_with_hooks(
            sessions_dir_with_web_uis.path(),
            "fixture-session",
            " Design editor ",
            "http://localhost:48761/new",
            false,
            |hook, env| {
                calls.push((
                    hook.to_owned(),
                    env.iter()
                        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                        .collect(),
                ));
                if hook == "web-ui-expose" {
                    Ok(Some("https://public.example.test/new".to_owned()))
                } else if fail_unexpose {
                    anyhow::bail!("unexpose failed")
                } else {
                    Ok(None)
                }
            },
        )
        .expect("registered web UI should be saved");
        let stored = store::load_session_from(sessions_dir_with_web_uis.path(), "fixture-session")
            .expect("session should load")
            .expect("session should exist");
        let expected = vec![
            WebUiRegistration {
                name: "Preview".to_owned(),
                url: "http://localhost:48761/preview".to_owned(),
                exposed_url: None,
            },
            WebUiRegistration {
                name: "Design editor".to_owned(),
                url: "http://localhost:48761/new".to_owned(),
                exposed_url: Some("https://public.example.test/new".to_owned()),
            },
        ];

        assert_eq!(
            (session.web_uis, stored.web_uis, calls),
            (
                expected.clone(),
                expected,
                vec![
                    (
                        "web-ui-unexpose".to_owned(),
                        vec![
                            (
                                "ARMYKNIFE_WEB_UI_URL".to_owned(),
                                "http://localhost:48761/old".to_owned()
                            ),
                            ("ARMYKNIFE_WEB_UI_PORT".to_owned(), "48761".to_owned()),
                            (
                                "ARMYKNIFE_WEB_UI_EXPOSED_URL".to_owned(),
                                "https://public.example.test/old".to_owned(),
                            ),
                        ],
                    ),
                    (
                        "web-ui-expose".to_owned(),
                        vec![
                            (
                                "ARMYKNIFE_WEB_UI_URL".to_owned(),
                                "http://localhost:48761/new".to_owned()
                            ),
                            ("ARMYKNIFE_WEB_UI_PORT".to_owned(), "48761".to_owned()),
                        ],
                    ),
                ],
            )
        );
    }

    #[rstest]
    #[case::empty_output(Some("  "))]
    #[case::invalid_url(Some("file:///fixture/external.html"))]
    #[case::hook_failure(None)]
    fn recording_succeeds_without_an_exposed_url(
        sessions_dir: tempfile::TempDir,
        #[case] hook_output: Option<&str>,
    ) {
        let session = record_url_in_with_hooks(
            sessions_dir.path(),
            "fixture-session",
            "Design editor",
            "http://localhost:48761/design/sample/",
            false,
            |hook, _| {
                if hook_output.is_none() && hook == "web-ui-expose" {
                    anyhow::bail!("hook failed")
                }
                Ok(hook_output.map(str::to_owned))
            },
        )
        .expect("registration should succeed when the expose hook is unavailable");

        assert_eq!(
            session.web_uis,
            vec![WebUiRegistration {
                name: "Design editor".to_owned(),
                url: "http://localhost:48761/design/sample/".to_owned(),
                exposed_url: None,
            }],
        );
    }

    #[rstest]
    fn recording_a_crit_url_passes_its_default_port_to_the_expose_hook(
        sessions_dir: tempfile::TempDir,
    ) {
        let mut calls = Vec::<(String, Vec<(String, String)>)>::new();
        let session = record_url_in_with_hooks(
            sessions_dir.path(),
            "fixture-session",
            "crit",
            "http://localhost/crit/review",
            true,
            |hook, env| {
                calls.push((
                    hook.to_owned(),
                    env.iter()
                        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                        .collect(),
                ));
                Ok((hook == "web-ui-expose")
                    .then(|| "https://review.example.test/crit/review".to_owned()))
            },
        )
        .expect("crit URL should be recorded without an explicit port");

        assert_eq!(
            (session.web_uis, calls),
            (
                vec![WebUiRegistration {
                    name: "crit".to_owned(),
                    url: "http://localhost/crit/review".to_owned(),
                    exposed_url: Some("https://review.example.test/crit/review".to_owned()),
                }],
                vec![(
                    "web-ui-expose".to_owned(),
                    vec![
                        (
                            "ARMYKNIFE_WEB_UI_URL".to_owned(),
                            "http://localhost/crit/review".to_owned(),
                        ),
                        ("ARMYKNIFE_WEB_UI_PORT".to_owned(), "80".to_owned()),
                    ],
                )],
            ),
        );
    }

    #[rstest]
    fn recording_a_crit_root_url_exposes_the_effective_pathful_url(
        sessions_dir: tempfile::TempDir,
    ) {
        let mut initial = store::load_session_from(sessions_dir.path(), "fixture-session")
            .expect("session should load")
            .expect("session should exist");
        initial.crit_urls = vec!["http://localhost:48761/crit/review".to_owned()];
        initial.web_uis = vec![WebUiRegistration {
            name: "crit".to_owned(),
            url: "http://localhost:48761/crit/review".to_owned(),
            exposed_url: None,
        }];
        store::save_session_to(sessions_dir.path(), &initial).expect("session should save");
        let mut calls = Vec::<(String, Vec<(String, String)>)>::new();

        let session = record_url_in_with_hooks(
            sessions_dir.path(),
            "fixture-session",
            "crit",
            "http://localhost:48761/",
            true,
            |hook, env| {
                calls.push((
                    hook.to_owned(),
                    env.iter()
                        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                        .collect(),
                ));
                Ok(Some("https://review.example.test/crit/review".to_owned()))
            },
        )
        .expect("crit URL should preserve the effective pathful URL");

        assert_eq!(
            (session.crit_urls, session.web_uis, calls),
            (
                vec!["http://localhost:48761/crit/review".to_owned()],
                vec![WebUiRegistration {
                    name: "crit".to_owned(),
                    url: "http://localhost:48761/crit/review".to_owned(),
                    exposed_url: Some("https://review.example.test/crit/review".to_owned()),
                }],
                vec![(
                    "web-ui-expose".to_owned(),
                    vec![
                        (
                            "ARMYKNIFE_WEB_UI_URL".to_owned(),
                            "http://localhost:48761/crit/review".to_owned(),
                        ),
                        ("ARMYKNIFE_WEB_UI_PORT".to_owned(), "48761".to_owned()),
                    ],
                )],
            ),
        );
    }

    #[rstest]
    fn recording_cleans_up_exposure_if_registration_changes_during_the_hook(
        sessions_dir: tempfile::TempDir,
    ) {
        let mut calls = Vec::<(String, Vec<(String, String)>)>::new();
        let session = record_url_in_with_hooks(
            sessions_dir.path(),
            "fixture-session",
            "Design editor",
            "http://localhost:48761/initial",
            false,
            |hook, env| {
                calls.push((
                    hook.to_owned(),
                    env.iter()
                        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                        .collect(),
                ));
                if hook == "web-ui-expose" {
                    let mut current =
                        store::load_session_from(sessions_dir.path(), "fixture-session")
                            .expect("concurrent session should load")
                            .expect("concurrent session should exist");
                    register(
                        &mut current.web_uis,
                        "Design editor",
                        "http://localhost:48761/replacement",
                    );
                    store::save_session_to(sessions_dir.path(), &current)
                        .expect("concurrent replacement should save");
                    Ok(Some("https://public.example.test/initial".to_owned()))
                } else {
                    Ok(None)
                }
            },
        )
        .expect("registration replacement during exposure should be recovered");
        let stored = store::load_session_from(sessions_dir.path(), "fixture-session")
            .expect("session should load")
            .expect("session should exist");
        let expected = vec![WebUiRegistration {
            name: "Design editor".to_owned(),
            url: "http://localhost:48761/replacement".to_owned(),
            exposed_url: None,
        }];

        assert_eq!(
            (session.web_uis, stored.web_uis, calls),
            (
                expected.clone(),
                expected,
                vec![
                    (
                        "web-ui-expose".to_owned(),
                        vec![
                            (
                                "ARMYKNIFE_WEB_UI_URL".to_owned(),
                                "http://localhost:48761/initial".to_owned()
                            ),
                            ("ARMYKNIFE_WEB_UI_PORT".to_owned(), "48761".to_owned()),
                        ],
                    ),
                    (
                        "web-ui-unexpose".to_owned(),
                        vec![
                            (
                                "ARMYKNIFE_WEB_UI_URL".to_owned(),
                                "http://localhost:48761/initial".to_owned()
                            ),
                            ("ARMYKNIFE_WEB_UI_PORT".to_owned(), "48761".to_owned()),
                            (
                                "ARMYKNIFE_WEB_UI_EXPOSED_URL".to_owned(),
                                "https://public.example.test/initial".to_owned(),
                            ),
                        ],
                    ),
                ],
            )
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
        let session = record_url_in_with_hooks(
            sessions_dir.path(),
            "fixture-session",
            name,
            url,
            record_legacy_crit_url,
            |_, _| Ok(None),
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
}
