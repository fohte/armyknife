use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeSet;

use crate::infra::tmux;

use super::url::parse_port;

pub(crate) trait CritPaneRuntime {
    fn is_crit_pane(&self, pane_id: &str) -> bool;
    fn find_crit_panes_for_parent(&self, parent_pane_id: &str) -> Result<Vec<String>>;
    fn parent_pane_id_for_crit_pane(&self, pane_id: &str) -> Option<String>;
    fn port_for_crit_pane(&self, pane_id: &str) -> Option<u16>;
    fn find_crit_panes_for_port(&self, port: u16) -> Result<Vec<String>>;
    fn window_id_for_pane(&self, pane_id: &str) -> Option<String>;
    fn close_crit_pane(&self, pane_id: &str) -> Result<()>;
    fn focus_pane(&self, pane_id: &str) -> Result<()>;
    fn open_crit_pane(&self, spec: tmux::CritPaneSpec<'_>) -> Result<()>;
}

pub(crate) struct LiveCritPaneRuntime;

impl CritPaneRuntime for LiveCritPaneRuntime {
    fn is_crit_pane(&self, pane_id: &str) -> bool {
        tmux::is_crit_pane(pane_id)
    }

    fn find_crit_panes_for_parent(&self, parent_pane_id: &str) -> Result<Vec<String>> {
        Ok(tmux::find_crit_panes_for_parent(parent_pane_id)?)
    }

    fn parent_pane_id_for_crit_pane(&self, pane_id: &str) -> Option<String> {
        tmux::get_crit_pane_parent_id(pane_id)
    }

    fn port_for_crit_pane(&self, pane_id: &str) -> Option<u16> {
        tmux::get_crit_pane_port(pane_id)
    }

    fn find_crit_panes_for_port(&self, port: u16) -> Result<Vec<String>> {
        Ok(tmux::find_crit_panes_for_port(port)?)
    }

    fn window_id_for_pane(&self, pane_id: &str) -> Option<String> {
        tmux::get_window_id_for_pane(pane_id)
    }

    fn close_crit_pane(&self, pane_id: &str) -> Result<()> {
        super::close_crit_pane(pane_id)
    }

    fn focus_pane(&self, pane_id: &str) -> Result<()> {
        tmux::focus_pane(pane_id).map_err(Into::into)
    }

    fn open_crit_pane(&self, spec: tmux::CritPaneSpec<'_>) -> Result<()> {
        tmux::open_crit_pane(spec)
    }
}

pub(crate) fn close_if_crit_pane(pane_id: &str, runtime: &impl CritPaneRuntime) -> Result<bool> {
    if !runtime.is_crit_pane(pane_id) {
        return Ok(false);
    }

    runtime.close_crit_pane(pane_id)?;
    Ok(true)
}

pub(crate) fn toggle_review_pane(
    parent_pane_id: &str,
    latest_url: Option<&str>,
    title: &str,
    runtime: &impl CritPaneRuntime,
) -> Result<()> {
    let mut crit_pane_ids = runtime.find_crit_panes_for_parent(parent_pane_id)?;
    crit_pane_ids.sort();

    if crit_pane_ids.is_empty() {
        let url = latest_url.context("No crit review is associated with this agent session")?;
        let port = parse_port(url)?;
        runtime
            .window_id_for_pane(parent_pane_id)
            .with_context(|| {
                format!("Could not determine window for parent pane {parent_pane_id}")
            })?;
        ensure_no_port_conflict(port, parent_pane_id, &[], runtime)?;
        return open_review_pane(parent_pane_id, url, port, title, false, runtime);
    }

    let mut verified_crit_pane_ids = Vec::with_capacity(crit_pane_ids.len());
    for crit_pane_id in crit_pane_ids {
        let crit_parent_pane_id = runtime
            .parent_pane_id_for_crit_pane(&crit_pane_id)
            .with_context(|| format!("Crit pane {crit_pane_id} has no parent pane ID"))?;
        ensure!(
            crit_parent_pane_id == parent_pane_id,
            "Crit pane {crit_pane_id} belongs to parent pane {crit_parent_pane_id}, not {parent_pane_id}"
        );
        verified_crit_pane_ids.push(crit_pane_id);
    }

    let parent_window_id = runtime
        .window_id_for_pane(parent_pane_id)
        .with_context(|| format!("Could not determine window for parent pane {parent_pane_id}"))?;
    let mut crit_panes = Vec::with_capacity(verified_crit_pane_ids.len());
    for crit_pane_id in verified_crit_pane_ids {
        let crit_window_id = runtime
            .window_id_for_pane(&crit_pane_id)
            .with_context(|| format!("Could not determine window for Crit pane {crit_pane_id}"))?;
        crit_panes.push((crit_pane_id, crit_window_id));
    }

    if let Some((crit_pane_id, _)) = crit_panes
        .iter()
        .find(|(_, window_id)| window_id == &parent_window_id)
    {
        runtime.close_crit_pane(crit_pane_id)?;
        return Ok(());
    }

    let mut old_ports = BTreeSet::new();
    for (crit_pane_id, _) in &crit_panes {
        let old_port = runtime
            .port_for_crit_pane(crit_pane_id)
            .with_context(|| format!("Crit pane {crit_pane_id} has no valid port"))?;
        old_ports.insert(old_port);
    }

    let url = latest_url.context("No crit review is associated with this agent session")?;
    let port = parse_port(url)?;
    let force_shpool_attach = old_ports.contains(&port);
    let mut ports_to_check = old_ports;
    ports_to_check.insert(port);
    let closing_pane_ids = crit_panes
        .iter()
        .map(|(pane_id, _)| pane_id.clone())
        .collect::<Vec<_>>();
    for port_to_check in ports_to_check {
        ensure_no_port_conflict(port_to_check, parent_pane_id, &closing_pane_ids, runtime)?;
    }

    for crit_pane_id in closing_pane_ids {
        runtime.close_crit_pane(&crit_pane_id)?;
    }

    open_review_pane(
        parent_pane_id,
        url,
        port,
        title,
        force_shpool_attach,
        runtime,
    )
}

fn ensure_no_port_conflict(
    port: u16,
    parent_pane_id: &str,
    closing_pane_ids: &[String],
    runtime: &impl CritPaneRuntime,
) -> Result<()> {
    for pane_id in runtime.find_crit_panes_for_port(port)? {
        if closing_pane_ids.iter().any(|closing| closing == &pane_id) {
            continue;
        }

        let owner_pane_id = runtime
            .parent_pane_id_for_crit_pane(&pane_id)
            .with_context(|| format!("Crit pane {pane_id} has no parent pane ID"))?;
        if owner_pane_id != parent_pane_id {
            bail!(
                "Crit port {port} is already used by Crit pane {pane_id} for parent {owner_pane_id}"
            );
        }
    }

    Ok(())
}

fn open_review_pane(
    parent_pane_id: &str,
    url: &str,
    port: u16,
    title: &str,
    force_shpool_attach: bool,
    runtime: &impl CritPaneRuntime,
) -> Result<()> {
    runtime
        .focus_pane(parent_pane_id)
        .context("failed to focus the agent tmux pane")?;
    tracing::info!(
        event = "agent.crit.open.floating_pane_requested",
        port,
        force_shpool_attach
    );
    runtime.open_crit_pane(tmux::CritPaneSpec {
        parent_pane_id,
        url,
        port,
        title,
        force_shpool_attach,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use rstest::rstest;

    use super::*;

    const PARENT: &str = "%test-parent";
    const OLD_CRIT: &str = "%old-crit";
    const CURRENT_CRIT: &str = "%current-crit";
    const SECOND_OLD_CRIT: &str = "%second-old-crit";
    const OTHER_CRIT: &str = "%other-crit";
    const REVIEW_URL: &str = "http://127.0.0.1:58321/review";
    const SECOND_REVIEW_URL: &str = "http://127.0.0.1:59432/review";
    const THIRD_REVIEW_URL: &str = "http://127.0.0.1:62784/review";
    #[derive(Clone, Debug)]
    struct Pane {
        id: String,
        parent: Option<String>,
        port: Option<u16>,
    }

    #[derive(Debug, Default)]
    struct FakeRuntime {
        panes: Vec<Pane>,
        windows: HashMap<String, String>,
        operations: RefCell<Vec<String>>,
        unreadable_parent_panes: Vec<String>,
        find_error: bool,
        port_lookup_error: bool,
        close_error: bool,
    }

    impl FakeRuntime {
        fn new(panes: Vec<Pane>, windows: &[(&str, &str)]) -> Self {
            Self {
                panes,
                windows: windows
                    .iter()
                    .map(|(pane_id, window_id)| (pane_id.to_string(), window_id.to_string()))
                    .collect(),
                ..Self::default()
            }
        }

        fn operations(&self) -> Vec<String> {
            self.operations.borrow().clone()
        }

        fn record(&self, operation: String) {
            self.operations.borrow_mut().push(operation);
        }

        fn pane(&self, pane_id: &str) -> Option<&Pane> {
            self.panes.iter().find(|pane| pane.id == pane_id)
        }
    }

    impl CritPaneRuntime for FakeRuntime {
        fn is_crit_pane(&self, pane_id: &str) -> bool {
            self.record(format!("is_crit:{pane_id}"));
            self.pane(pane_id).is_some()
        }

        fn find_crit_panes_for_parent(&self, parent_pane_id: &str) -> Result<Vec<String>> {
            self.record(format!("find_parent:{parent_pane_id}"));
            if self.find_error {
                bail!("Crit pane lookup failed");
            }
            Ok(self
                .panes
                .iter()
                .filter(|pane| pane.parent.as_deref() == Some(parent_pane_id))
                .map(|pane| pane.id.clone())
                .collect())
        }

        fn parent_pane_id_for_crit_pane(&self, pane_id: &str) -> Option<String> {
            self.record(format!("get_parent:{pane_id}"));
            if self
                .unreadable_parent_panes
                .iter()
                .any(|unreadable| unreadable == pane_id)
            {
                return None;
            }
            self.pane(pane_id).and_then(|pane| pane.parent.clone())
        }

        fn port_for_crit_pane(&self, pane_id: &str) -> Option<u16> {
            self.record(format!("get_port:{pane_id}"));
            self.pane(pane_id).and_then(|pane| pane.port)
        }

        fn find_crit_panes_for_port(&self, port: u16) -> Result<Vec<String>> {
            self.record(format!("find_port:{port}"));
            if self.port_lookup_error {
                bail!("Crit port lookup failed");
            }
            Ok(self
                .panes
                .iter()
                .filter(|pane| pane.port == Some(port))
                .map(|pane| pane.id.clone())
                .collect())
        }

        fn window_id_for_pane(&self, pane_id: &str) -> Option<String> {
            self.record(format!("get_window:{pane_id}"));
            self.windows.get(pane_id).cloned()
        }

        fn close_crit_pane(&self, pane_id: &str) -> Result<()> {
            self.record(format!("close:{pane_id}"));
            if self.close_error {
                bail!("Crit pane close failed");
            }
            Ok(())
        }

        fn focus_pane(&self, pane_id: &str) -> Result<()> {
            self.record(format!("focus:{pane_id}"));
            Ok(())
        }

        fn open_crit_pane(&self, spec: tmux::CritPaneSpec<'_>) -> Result<()> {
            self.record(format!(
                "open:{}:{}:{}:force={}",
                spec.parent_pane_id, spec.url, spec.port, spec.force_shpool_attach
            ));
            Ok(())
        }
    }

    #[derive(Debug)]
    struct ToggleCase {
        runtime: FakeRuntime,
        target_pane_id: &'static str,
        latest_url: Option<&'static str>,
        expected_result: std::result::Result<(), String>,
        expected_operations: Vec<String>,
    }

    fn pane(id: &str, parent: Option<&str>, port: Option<u16>) -> Pane {
        Pane {
            id: id.to_string(),
            parent: parent.map(str::to_string),
            port,
        }
    }

    fn moved_case() -> ToggleCase {
        ToggleCase {
            runtime: FakeRuntime::new(
                vec![
                    pane(OLD_CRIT, Some(PARENT), Some(58321)),
                    pane(OTHER_CRIT, Some("%other-parent"), Some(59432)),
                ],
                &[(PARENT, "@new-window"), (OLD_CRIT, "@old-window")],
            ),
            target_pane_id: PARENT,
            latest_url: Some(REVIEW_URL),
            expected_result: Ok(()),
            expected_operations: vec![
                "find_parent:%test-parent",
                "get_parent:%old-crit",
                "get_window:%test-parent",
                "get_window:%old-crit",
                "get_port:%old-crit",
                "find_port:58321",
                "close:%old-crit",
                "focus:%test-parent",
                "open:%test-parent:http://127.0.0.1:58321/review:58321:force=true",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    fn moved_changed_port_case() -> ToggleCase {
        ToggleCase {
            runtime: FakeRuntime::new(
                vec![
                    pane(OLD_CRIT, Some(PARENT), Some(59432)),
                    pane(OTHER_CRIT, Some("%other-parent"), Some(62784)),
                ],
                &[(PARENT, "@new-window"), (OLD_CRIT, "@old-window")],
            ),
            target_pane_id: PARENT,
            latest_url: Some(REVIEW_URL),
            expected_result: Ok(()),
            expected_operations: vec![
                "find_parent:%test-parent",
                "get_parent:%old-crit",
                "get_window:%test-parent",
                "get_window:%old-crit",
                "get_port:%old-crit",
                "find_port:58321",
                "find_port:59432",
                "close:%old-crit",
                "focus:%test-parent",
                "open:%test-parent:http://127.0.0.1:58321/review:58321:force=false",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    fn moved_missing_url_case() -> ToggleCase {
        ToggleCase {
            runtime: FakeRuntime::new(
                vec![pane(OLD_CRIT, Some(PARENT), Some(58321))],
                &[(PARENT, "@new-window"), (OLD_CRIT, "@old-window")],
            ),
            target_pane_id: PARENT,
            latest_url: None,
            expected_result: Err("No crit review is associated with this agent session".to_string()),
            expected_operations: vec![
                "find_parent:%test-parent",
                "get_parent:%old-crit",
                "get_window:%test-parent",
                "get_window:%old-crit",
                "get_port:%old-crit",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    fn ordinary_open_case() -> ToggleCase {
        ToggleCase {
            runtime: FakeRuntime::new(Vec::new(), &[(PARENT, "@new-window")]),
            target_pane_id: PARENT,
            latest_url: Some(REVIEW_URL),
            expected_result: Ok(()),
            expected_operations: vec![
                "find_parent:%test-parent",
                "get_window:%test-parent",
                "find_port:58321",
                "focus:%test-parent",
                "open:%test-parent:http://127.0.0.1:58321/review:58321:force=false",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    fn same_window_case() -> ToggleCase {
        ToggleCase {
            runtime: FakeRuntime::new(
                vec![pane(OLD_CRIT, Some(PARENT), Some(58321))],
                &[(PARENT, "@same-window"), (OLD_CRIT, "@same-window")],
            ),
            target_pane_id: PARENT,
            latest_url: None,
            expected_result: Ok(()),
            expected_operations: vec![
                "find_parent:%test-parent",
                "get_parent:%old-crit",
                "get_window:%test-parent",
                "get_window:%old-crit",
                "close:%old-crit",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    fn same_port_conflict_case() -> ToggleCase {
        ToggleCase {
            runtime: FakeRuntime::new(
                vec![
                    pane(OLD_CRIT, Some(PARENT), Some(58321)),
                    pane(OTHER_CRIT, Some("%other-parent"), Some(58321)),
                ],
                &[(PARENT, "@new-window"), (OLD_CRIT, "@old-window")],
            ),
            target_pane_id: PARENT,
            latest_url: Some(REVIEW_URL),
            expected_result: Err(
                "Crit port 58321 is already used by Crit pane %other-crit for parent %other-parent"
                    .to_string(),
            ),
            expected_operations: vec![
                "find_parent:%test-parent",
                "get_parent:%old-crit",
                "get_window:%test-parent",
                "get_window:%old-crit",
                "get_port:%old-crit",
                "find_port:58321",
                "get_parent:%other-crit",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    fn desired_port_conflict_case() -> ToggleCase {
        let runtime = FakeRuntime::new(
            vec![
                pane(OLD_CRIT, Some(PARENT), Some(59432)),
                pane(OTHER_CRIT, Some("%other-parent"), Some(58321)),
            ],
            &[(PARENT, "@new-window"), (OLD_CRIT, "@old-window")],
        );
        ToggleCase {
            runtime,
            target_pane_id: PARENT,
            latest_url: Some(REVIEW_URL),
            expected_result: Err(
                "Crit port 58321 is already used by Crit pane %other-crit for parent %other-parent"
                    .to_string(),
            ),
            expected_operations: vec![
                "find_parent:%test-parent",
                "get_parent:%old-crit",
                "get_window:%test-parent",
                "get_window:%old-crit",
                "get_port:%old-crit",
                "find_port:58321",
                "get_parent:%other-crit",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    fn current_and_old_case(reverse: bool) -> ToggleCase {
        let current = pane(CURRENT_CRIT, Some(PARENT), Some(59432));
        let old = pane(OLD_CRIT, Some(PARENT), Some(58321));
        ToggleCase {
            runtime: FakeRuntime::new(
                if reverse {
                    vec![old, current]
                } else {
                    vec![current, old]
                },
                &[
                    (PARENT, "@new-window"),
                    (CURRENT_CRIT, "@new-window"),
                    (OLD_CRIT, "@old-window"),
                ],
            ),
            target_pane_id: PARENT,
            latest_url: Some(SECOND_REVIEW_URL),
            expected_result: Ok(()),
            expected_operations: vec![
                "find_parent:%test-parent",
                "get_parent:%current-crit",
                "get_parent:%old-crit",
                "get_window:%test-parent",
                "get_window:%current-crit",
                "get_window:%old-crit",
                "close:%current-crit",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    fn multiple_old_case(reverse: bool, latest_url: &'static str, force: bool) -> ToggleCase {
        let old = pane(OLD_CRIT, Some(PARENT), Some(58321));
        let second_old = pane(SECOND_OLD_CRIT, Some(PARENT), Some(59432));
        let mut expected_operations = vec![
            "find_parent:%test-parent",
            "get_parent:%old-crit",
            "get_parent:%second-old-crit",
            "get_window:%test-parent",
            "get_window:%old-crit",
            "get_window:%second-old-crit",
            "get_port:%old-crit",
            "get_port:%second-old-crit",
            "find_port:58321",
            "find_port:59432",
            "close:%old-crit",
            "close:%second-old-crit",
            "focus:%test-parent",
        ]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
        let (port, url) = if latest_url == SECOND_REVIEW_URL {
            (59432, SECOND_REVIEW_URL)
        } else {
            expected_operations.insert(10, "find_port:62784".to_string());
            (62784, THIRD_REVIEW_URL)
        };
        expected_operations.push(format!("open:%test-parent:{url}:{port}:force={force}"));

        ToggleCase {
            runtime: FakeRuntime::new(
                if reverse {
                    vec![second_old, old]
                } else {
                    vec![old, second_old]
                },
                &[
                    (PARENT, "@new-window"),
                    (OLD_CRIT, "@old-window"),
                    (SECOND_OLD_CRIT, "@older-window"),
                ],
            ),
            target_pane_id: PARENT,
            latest_url: Some(latest_url),
            expected_result: Ok(()),
            expected_operations,
        }
    }

    fn lookup_failure_case() -> ToggleCase {
        ToggleCase {
            runtime: FakeRuntime {
                find_error: true,
                ..FakeRuntime::default()
            },
            target_pane_id: PARENT,
            latest_url: Some(REVIEW_URL),
            expected_result: Err("Crit pane lookup failed".to_string()),
            expected_operations: vec!["find_parent:%test-parent".to_string()],
        }
    }

    fn close_failure_case() -> ToggleCase {
        let mut runtime = FakeRuntime::new(
            vec![pane(OLD_CRIT, Some(PARENT), Some(58321))],
            &[(PARENT, "@new-window"), (OLD_CRIT, "@old-window")],
        );
        runtime.close_error = true;
        ToggleCase {
            runtime,
            target_pane_id: PARENT,
            latest_url: Some(REVIEW_URL),
            expected_result: Err("Crit pane close failed".to_string()),
            expected_operations: vec![
                "find_parent:%test-parent",
                "get_parent:%old-crit",
                "get_window:%test-parent",
                "get_window:%old-crit",
                "get_port:%old-crit",
                "find_port:58321",
                "close:%old-crit",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    fn port_lookup_failure_case() -> ToggleCase {
        ToggleCase {
            runtime: FakeRuntime {
                panes: vec![pane(OLD_CRIT, Some(PARENT), Some(58321))],
                windows: [
                    (PARENT.to_string(), "@new-window".to_string()),
                    (OLD_CRIT.to_string(), "@old-window".to_string()),
                ]
                .into(),
                port_lookup_error: true,
                ..FakeRuntime::default()
            },
            target_pane_id: PARENT,
            latest_url: Some(REVIEW_URL),
            expected_result: Err("Crit port lookup failed".to_string()),
            expected_operations: vec![
                "find_parent:%test-parent",
                "get_parent:%old-crit",
                "get_window:%test-parent",
                "get_window:%old-crit",
                "get_port:%old-crit",
                "find_port:58321",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    fn missing_window_case() -> ToggleCase {
        ToggleCase {
            runtime: FakeRuntime::new(
                vec![pane(OLD_CRIT, Some(PARENT), Some(58321))],
                &[(OLD_CRIT, "@old-window")],
            ),
            target_pane_id: PARENT,
            latest_url: Some(REVIEW_URL),
            expected_result: Err(
                "Could not determine window for parent pane %test-parent".to_string()
            ),
            expected_operations: vec![
                "find_parent:%test-parent",
                "get_parent:%old-crit",
                "get_window:%test-parent",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    fn missing_port_case() -> ToggleCase {
        ToggleCase {
            runtime: FakeRuntime::new(
                vec![pane(OLD_CRIT, Some(PARENT), None)],
                &[(PARENT, "@new-window"), (OLD_CRIT, "@old-window")],
            ),
            target_pane_id: PARENT,
            latest_url: Some(REVIEW_URL),
            expected_result: Err("Crit pane %old-crit has no valid port".to_string()),
            expected_operations: vec![
                "find_parent:%test-parent",
                "get_parent:%old-crit",
                "get_window:%test-parent",
                "get_window:%old-crit",
                "get_port:%old-crit",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    fn missing_parent_case() -> ToggleCase {
        let mut runtime = FakeRuntime::new(
            vec![pane(OLD_CRIT, Some(PARENT), Some(58321))],
            &[(PARENT, "@new-window"), (OLD_CRIT, "@old-window")],
        );
        runtime.unreadable_parent_panes.push(OLD_CRIT.to_string());
        ToggleCase {
            runtime,
            target_pane_id: PARENT,
            latest_url: Some(REVIEW_URL),
            expected_result: Err("Crit pane %old-crit has no parent pane ID".to_string()),
            expected_operations: vec!["find_parent:%test-parent", "get_parent:%old-crit"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        }
    }

    fn ordinary_open_conflict_case() -> ToggleCase {
        ToggleCase {
            runtime: FakeRuntime::new(
                vec![pane(OTHER_CRIT, Some("%other-parent"), Some(58321))],
                &[(PARENT, "@new-window")],
            ),
            target_pane_id: PARENT,
            latest_url: Some(REVIEW_URL),
            expected_result: Err(
                "Crit port 58321 is already used by Crit pane %other-crit for parent %other-parent"
                    .to_string(),
            ),
            expected_operations: vec![
                "find_parent:%test-parent",
                "get_window:%test-parent",
                "find_port:58321",
                "get_parent:%other-crit",
            ]
            .into_iter()
            .map(str::to_string)
            .collect(),
        }
    }

    #[rstest]
    #[case::reopens_a_moved_review_pane(moved_case())]
    #[case::uses_regular_attach_when_the_moved_review_uses_a_different_port(
        moved_changed_port_case()
    )]
    #[case::keeps_the_old_pane_when_the_moved_review_url_is_missing(moved_missing_url_case())]
    #[case::opens_without_an_existing_review(ordinary_open_case())]
    #[case::closes_a_review_in_the_same_window(same_window_case())]
    #[case::prioritizes_the_current_window_pane_when_old_and_current_panes_are_listed_in_order(
        current_and_old_case(false)
    )]
    #[case::prioritizes_the_current_window_pane_when_old_and_current_panes_are_listed_in_reverse_order(
        current_and_old_case(true)
    )]
    #[case::closes_all_old_panes_before_reopening_when_listed_in_order(multiple_old_case(
        false,
        SECOND_REVIEW_URL,
        true
    ))]
    #[case::closes_all_old_panes_before_reopening_when_listed_in_reverse_order(multiple_old_case(
        true,
        SECOND_REVIEW_URL,
        true
    ))]
    #[case::uses_regular_attach_when_the_new_port_is_not_in_multiple_old_panes(multiple_old_case(
        false,
        THIRD_REVIEW_URL,
        false
    ))]
    #[case::preserves_another_parents_same_port_review(same_port_conflict_case())]
    #[case::checks_the_new_port_before_closing_the_old_pane(desired_port_conflict_case())]
    #[case::stops_when_discovery_fails(lookup_failure_case())]
    #[case::stops_when_port_discovery_fails(port_lookup_failure_case())]
    #[case::stops_when_close_fails(close_failure_case())]
    #[case::stops_when_a_window_id_is_unknown(missing_window_case())]
    #[case::stops_when_the_old_port_is_unknown(missing_port_case())]
    #[case::stops_when_the_old_parent_id_is_unknown(missing_parent_case())]
    #[case::rejects_another_parents_same_port_on_a_fresh_open(ordinary_open_conflict_case())]
    fn toggle_review_pane_runs_expected_operations(#[case] case: ToggleCase) {
        let result = toggle_review_pane(
            case.target_pane_id,
            case.latest_url,
            "Review title",
            &case.runtime,
        )
        .map_err(|error| format!("{error:#}"));

        assert_eq!(
            (result, case.runtime.operations()),
            (case.expected_result, case.expected_operations),
        );
    }

    #[rstest]
    #[case::leaves_non_crit_panes_open(false, false)]
    #[case::closes_crit_panes(true, true)]
    fn close_if_crit_pane_only_closes_crit_panes(
        #[case] is_crit: bool,
        #[case] expected_closed: bool,
    ) {
        let runtime = FakeRuntime::new(
            if is_crit {
                vec![pane("%direct-crit", Some(PARENT), Some(58321))]
            } else {
                Vec::new()
            },
            &[],
        );
        let result = close_if_crit_pane("%direct-crit", &runtime);
        let operations = runtime.operations();
        assert_eq!(
            (result.map_err(|error| error.to_string()), operations),
            (
                Ok(expected_closed),
                if expected_closed {
                    vec![
                        "is_crit:%direct-crit".to_string(),
                        "close:%direct-crit".to_string(),
                    ]
                } else {
                    vec!["is_crit:%direct-crit".to_string()]
                },
            ),
        );
    }
}
