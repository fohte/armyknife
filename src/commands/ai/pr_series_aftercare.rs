use anyhow::{Context, Result};

use super::pr_series::{
    PrSeriesPlan, Repository, load_plan_if_present, resolve_created_pull_requests,
    sync_after_submit, validate_checklist_leaf_ids,
};
use crate::infra::github::{PrBodyClient, RepoClient};
use crate::infra::tq::{TqChecklist, TqClient, TqTaskDetails};

pub(super) struct SubmittedPrContext<'a> {
    pub(super) task_id: &'a str,
    pub(super) checklist_item_id: &'a str,
    pub(super) owner: &'a str,
    pub(super) repo: &'a str,
    pub(super) url: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SubmitSyncStage {
    Prepare,
    Link,
    Sync,
}

#[derive(Debug)]
pub(super) struct SubmitSyncFailure {
    pub(super) stage: SubmitSyncStage,
    pub(super) error: anyhow::Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SubmitSyncOutcome {
    NoPlan,
    Synchronized,
}

impl SubmitSyncFailure {
    fn new(stage: SubmitSyncStage, error: impl Into<anyhow::Error>) -> Self {
        Self {
            stage,
            error: error.into(),
        }
    }
}

pub(super) async fn link_and_sync_after_submit(
    github: &(impl PrBodyClient + RepoClient),
    task_id: &str,
    checklist_item_id: &str,
    owner: &str,
    repo: &str,
    pr_url: &str,
) -> std::result::Result<SubmitSyncOutcome, SubmitSyncFailure> {
    let context = SubmittedPrContext {
        task_id,
        checklist_item_id,
        owner,
        repo,
        url: pr_url,
    };
    link_and_sync_after_submit_with(
        || TqClient::detect().context("tq CLI is required to synchronize a PR series"),
        load_plan_if_present,
        github,
        &context,
    )
    .await
}

pub(super) trait SubmitSyncTqClient {
    async fn task_details(&self, task_id: &str) -> Result<TqTaskDetails>;

    async fn list_checklists(&self, task_id: &str) -> Result<Vec<TqChecklist>>;

    async fn link_checklist_item(&self, item_id: &str, github_url: &str) -> Result<()>;
}

impl SubmitSyncTqClient for TqClient {
    async fn task_details(&self, task_id: &str) -> Result<TqTaskDetails> {
        TqClient::task_details(self, task_id)
            .await
            .map_err(Into::into)
    }

    async fn list_checklists(&self, task_id: &str) -> Result<Vec<TqChecklist>> {
        TqClient::list_checklists(self, task_id)
            .await
            .map_err(Into::into)
    }

    async fn link_checklist_item(&self, item_id: &str, github_url: &str) -> Result<()> {
        TqClient::link_checklist_item(self, item_id, github_url)
            .await
            .map_err(Into::into)
    }
}

impl<T: SubmitSyncTqClient + ?Sized> SubmitSyncTqClient for &T {
    async fn task_details(&self, task_id: &str) -> Result<TqTaskDetails> {
        (**self).task_details(task_id).await
    }

    async fn list_checklists(&self, task_id: &str) -> Result<Vec<TqChecklist>> {
        (**self).list_checklists(task_id).await
    }

    async fn link_checklist_item(&self, item_id: &str, github_url: &str) -> Result<()> {
        (**self).link_checklist_item(item_id, github_url).await
    }
}

pub(super) async fn link_and_sync_after_submit_with<Tq, Detect, LoadPlan>(
    detect_tq_client: Detect,
    load_plan: LoadPlan,
    github: &(impl PrBodyClient + RepoClient),
    context: &SubmittedPrContext<'_>,
) -> std::result::Result<SubmitSyncOutcome, SubmitSyncFailure>
where
    Tq: SubmitSyncTqClient,
    Detect: FnOnce() -> Result<Tq>,
    LoadPlan: FnOnce(&str) -> Result<Option<PrSeriesPlan>>,
{
    let Some(plan) = load_plan(context.task_id)
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Prepare, error))?
    else {
        return Ok(SubmitSyncOutcome::NoPlan);
    };
    let tq_client = detect_tq_client()
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Prepare, error))?;
    let task = tq_client
        .task_details(context.task_id)
        .await
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Prepare, error))?;
    let checklists = tq_client
        .list_checklists(&task.id)
        .await
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Prepare, error))?;
    validate_checklist_leaf_ids(&plan, &checklists)
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Prepare, error))?;
    let Some(entry) = plan
        .pull_requests
        .iter()
        .find(|entry| entry.tq_leaf_id == context.checklist_item_id)
    else {
        return Err(SubmitSyncFailure::new(
            SubmitSyncStage::Prepare,
            anyhow::anyhow!(
                "checklist item `{}` is not part of the saved PR series plan",
                context.checklist_item_id
            ),
        ));
    };
    let repository = Repository::parse(&entry.repository)
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Prepare, error))?;
    if !repository.matches(context.owner, context.repo) {
        return Err(SubmitSyncFailure::new(
            SubmitSyncStage::Prepare,
            anyhow::anyhow!(
                "checklist item `{}` belongs to `{}`, not `{}/{}`",
                context.checklist_item_id,
                repository.full_name(),
                context.owner,
                context.repo,
            ),
        ));
    }
    let entry_id = entry.id.clone();

    tq_client
        .link_checklist_item(context.checklist_item_id, context.url)
        .await
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Link, error))?;

    let task = tq_client
        .task_details(&task.id)
        .await
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Sync, error))?;
    let checklists = tq_client
        .list_checklists(&task.id)
        .await
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Sync, error))?;
    let created_pull_requests =
        resolve_created_pull_requests(&plan, &checklists, &task.github_links)
            .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Sync, error))?;
    let current_pr_was_linked = created_pull_requests.iter().any(|pull_request| {
        pull_request.entry_id == entry_id && pull_request.url.eq_ignore_ascii_case(context.url)
    });

    if !current_pr_was_linked {
        let sync_error = sync_after_submit(&plan, &created_pull_requests, github)
            .await
            .err()
            .map(|error| format!("; body synchronization also failed: {error}"))
            .unwrap_or_default();
        return Err(SubmitSyncFailure::new(
            SubmitSyncStage::Link,
            anyhow::anyhow!("the checklist link is not visible in tq yet{sync_error}"),
        ));
    }
    let failed_updates = sync_after_submit(&plan, &created_pull_requests, github)
        .await
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Sync, error))?;
    if failed_updates > 0 {
        return Err(SubmitSyncFailure::new(
            SubmitSyncStage::Sync,
            anyhow::anyhow!("{failed_updates} pull request(s) could not be synchronized"),
        ));
    }

    Ok(SubmitSyncOutcome::Synchronized)
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::sync::Mutex;

    use anyhow::{Result, bail};
    use rstest::rstest;

    use super::*;
    use crate::commands::ai::pr_series::{
        CreatedPullRequest, PrSeriesEntry, synchronized_body_for_test,
    };
    use crate::infra::github::{PrBodyClient, UpdatePrBodyParams};
    use crate::infra::tq::{TqChecklist, TqChecklistItem, TqGithubLink};

    #[derive(Default)]
    struct FakeGitHub {
        bodies: Mutex<HashMap<(String, u64), String>>,
        failed_updates_once: Mutex<HashSet<(String, u64)>>,
    }

    impl FakeGitHub {
        fn add_body(&self, repository: &str, number: u64, body: &str) {
            self.bodies
                .lock()
                .unwrap()
                .insert((repository.to_string(), number), body.to_string());
        }

        fn fail_update_once(&self, repository: &str, number: u64) {
            self.failed_updates_once
                .lock()
                .unwrap()
                .insert((repository.to_string(), number));
        }

        fn bodies(&self) -> HashMap<(String, u64), String> {
            self.bodies.lock().unwrap().clone()
        }
    }

    impl RepoClient for FakeGitHub {
        async fn repo_exists(&self, _owner: &str, _repo: &str) -> Result<bool> {
            Ok(true)
        }

        async fn is_repo_private(&self, _owner: &str, _repo: &str) -> Result<bool> {
            Ok(false)
        }

        async fn get_default_branch(&self, _owner: &str, _repo: &str) -> Result<String> {
            Ok("main".to_string())
        }
    }

    impl PrBodyClient for FakeGitHub {
        async fn get_pull_request_body(
            &self,
            owner: &str,
            repo: &str,
            number: u64,
        ) -> Result<String> {
            self.bodies
                .lock()
                .unwrap()
                .get(&(format!("{owner}/{repo}"), number))
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("body was not found"))
        }

        async fn update_pull_request_body(&self, params: UpdatePrBodyParams) -> Result<String> {
            let key = (format!("{}/{}", params.owner, params.repo), params.number);
            if self.failed_updates_once.lock().unwrap().remove(&key) {
                bail!("temporary API failure");
            }
            self.bodies.lock().unwrap().insert(key, params.body);
            Ok(format!(
                "https://github.com/{}/{}/pull/{}",
                params.owner, params.repo, params.number
            ))
        }
    }

    struct FakeSubmitTq {
        task: Mutex<TqTaskDetails>,
        checklists: Mutex<Vec<TqChecklist>>,
        link_calls: Mutex<Vec<(String, String)>>,
        link_error: Option<String>,
        link_visible: bool,
    }

    impl SubmitSyncTqClient for FakeSubmitTq {
        async fn task_details(&self, _task_id: &str) -> Result<TqTaskDetails> {
            Ok(self.task.lock().unwrap().clone())
        }

        async fn list_checklists(&self, _task_id: &str) -> Result<Vec<TqChecklist>> {
            Ok(self.checklists.lock().unwrap().clone())
        }

        async fn link_checklist_item(&self, item_id: &str, github_url: &str) -> Result<()> {
            self.link_calls
                .lock()
                .unwrap()
                .push((item_id.to_string(), github_url.to_string()));
            if let Some(error) = &self.link_error {
                bail!("{error}");
            }
            if !self.link_visible {
                return Ok(());
            }

            let mut checklists = self.checklists.lock().unwrap();
            let found = checklists
                .iter_mut()
                .flat_map(|checklist| &mut checklist.items)
                .any(|item| set_leaf_link(item, item_id, "link-settings"));
            if !found {
                bail!("checklist item was not found");
            }
            self.task.lock().unwrap().github_links.push(TqGithubLink {
                id: "link-settings".to_string(),
                kind: Some("pull_request".to_string()),
                owner: Some("acme".to_string()),
                repo: Some("notify-web".to_string()),
                number: Some(17),
                title: Some("Add the settings page".to_string()),
                url: Some(github_url.to_string()),
                state: Some("open".to_string()),
            });
            Ok(())
        }
    }

    fn set_leaf_link(item: &mut TqChecklistItem, item_id: &str, link_id: &str) -> bool {
        if item.id == item_id && item.children.is_empty() {
            item.github_link_id = Some(link_id.to_string());
            return true;
        }
        item.children
            .iter_mut()
            .any(|child| set_leaf_link(child, item_id, link_id))
    }

    fn fake_submit_tq(link_error: Option<&str>, link_visible: bool) -> FakeSubmitTq {
        FakeSubmitTq {
            task: Mutex::new(TqTaskDetails {
                id: "sample-task-id".to_string(),
                number: 58,
                github_links: vec![TqGithubLink {
                    id: "link-schema".to_string(),
                    kind: Some("pull_request".to_string()),
                    owner: Some("acme".to_string()),
                    repo: Some("notify-api".to_string()),
                    number: Some(12),
                    title: Some("Add preference tables".to_string()),
                    url: Some("https://github.com/acme/notify-api/pull/12".to_string()),
                    state: Some("open".to_string()),
                }],
            }),
            checklists: Mutex::new(vec![TqChecklist {
                id: "checklist-1".to_string(),
                name: "Implementation".to_string(),
                items: vec![TqChecklistItem {
                    id: "batch-1".to_string(),
                    content: "Implementation".to_string(),
                    checked_at: None,
                    github_link_id: None,
                    children: vec![
                        TqChecklistItem {
                            id: "leaf-schema".to_string(),
                            content: "Add preference tables".to_string(),
                            checked_at: None,
                            github_link_id: Some("link-schema".to_string()),
                            children: Vec::new(),
                        },
                        TqChecklistItem {
                            id: "leaf-settings".to_string(),
                            content: "Add the settings page".to_string(),
                            checked_at: None,
                            github_link_id: None,
                            children: Vec::new(),
                        },
                    ],
                }],
            }]),
            link_calls: Mutex::new(Vec::new()),
            link_error: link_error.map(str::to_string),
            link_visible,
        }
    }

    fn sample_plan() -> PrSeriesPlan {
        PrSeriesPlan {
            name: "Notification preferences".to_string(),
            purpose: "Let users choose which notifications they receive.".to_string(),
            pull_requests: vec![
                PrSeriesEntry {
                    id: "schema".to_string(),
                    summary: "Add preference tables".to_string(),
                    depends_on: Vec::new(),
                    repository: "acme/notify-api".to_string(),
                    tq_leaf_id: "leaf-schema".to_string(),
                },
                PrSeriesEntry {
                    id: "settings".to_string(),
                    summary: "Add the settings page".to_string(),
                    depends_on: vec!["schema".to_string()],
                    repository: "acme/notify-web".to_string(),
                    tq_leaf_id: "leaf-settings".to_string(),
                },
            ],
        }
    }

    fn created_pull_requests() -> Vec<CreatedPullRequest> {
        vec![
            CreatedPullRequest {
                entry_id: "schema".to_string(),
                repository: Repository {
                    owner: "acme".to_string(),
                    name: "notify-api".to_string(),
                },
                number: 12,
                url: "https://github.com/acme/notify-api/pull/12".to_string(),
            },
            CreatedPullRequest {
                entry_id: "settings".to_string(),
                repository: Repository {
                    owner: "acme".to_string(),
                    name: "notify-web".to_string(),
                },
                number: 17,
                url: "https://github.com/acme/notify-web/pull/17".to_string(),
            },
        ]
    }

    fn expected_body(
        plan: &PrSeriesPlan,
        entry_id: &str,
        original_body: &str,
        created_pull_requests: &[CreatedPullRequest],
    ) -> String {
        synchronized_body_for_test(
            plan,
            entry_id,
            original_body,
            &HashMap::from([
                ("acme/notify-api".to_string(), false),
                ("acme/notify-web".to_string(), false),
            ]),
            created_pull_requests,
        )
        .unwrap()
    }

    fn current_pr_context() -> SubmittedPrContext<'static> {
        SubmittedPrContext {
            task_id: "58",
            checklist_item_id: "leaf-settings",
            owner: "acme",
            repo: "notify-web",
            url: "https://github.com/acme/notify-web/pull/17",
        }
    }

    #[tokio::test]
    async fn links_new_pr_then_syncs_all_created_bodies() {
        let tq = fake_submit_tq(None, true);
        let plan = sample_plan();
        let github = FakeGitHub::default();
        github.add_body("acme/notify-api", 12, "Schema PR body");
        github.add_body("acme/notify-web", 17, "Settings PR body");
        let pull_requests = created_pull_requests();
        let expected_bodies = HashMap::from([
            (
                ("acme/notify-api".to_string(), 12),
                expected_body(&plan, "schema", "Schema PR body", &pull_requests),
            ),
            (
                ("acme/notify-web".to_string(), 17),
                expected_body(&plan, "settings", "Settings PR body", &pull_requests),
            ),
        ]);

        let result = link_and_sync_after_submit_with(
            || Ok(&tq),
            |_| Ok(Some(plan)),
            &github,
            &current_pr_context(),
        )
        .await
        .map_err(|failure| (failure.stage, failure.error.to_string()));

        assert_eq!(
            (
                result,
                tq.link_calls.lock().unwrap().clone(),
                github.bodies(),
            ),
            (
                Ok(SubmitSyncOutcome::Synchronized),
                vec![(
                    "leaf-settings".to_string(),
                    "https://github.com/acme/notify-web/pull/17".to_string(),
                )],
                expected_bodies,
            ),
        );
    }

    #[tokio::test]
    async fn skips_linking_and_sync_when_no_plan_is_saved() {
        let tq = fake_submit_tq(None, true);
        let github = FakeGitHub::default();
        github.add_body("acme/notify-api", 12, "Schema PR body");
        let expected_bodies = github.bodies();
        let detect_called = std::cell::Cell::new(false);

        let result = link_and_sync_after_submit_with(
            || -> Result<&FakeSubmitTq> {
                detect_called.set(true);
                bail!("tq CLI is unavailable")
            },
            |_| Ok(None),
            &github,
            &current_pr_context(),
        )
        .await
        .map_err(|failure| (failure.stage, failure.error.to_string()));

        assert_eq!(
            (
                result,
                detect_called.get(),
                tq.link_calls.lock().unwrap().clone(),
                github.bodies(),
            ),
            (
                Ok(SubmitSyncOutcome::NoPlan),
                false,
                Vec::new(),
                expected_bodies,
            ),
        );
    }

    #[tokio::test]
    async fn reports_link_errors_without_syncing_bodies() {
        let tq = fake_submit_tq(Some("temporary tq failure"), true);
        let github = FakeGitHub::default();
        github.add_body("acme/notify-api", 12, "Schema PR body");
        github.add_body("acme/notify-web", 17, "Settings PR body");
        let expected_bodies = github.bodies();

        let result = link_and_sync_after_submit_with(
            || Ok(&tq),
            |_| Ok(Some(sample_plan())),
            &github,
            &current_pr_context(),
        )
        .await
        .map_err(|failure| (failure.stage, failure.error.to_string()));

        assert_eq!(
            (
                result,
                tq.link_calls.lock().unwrap().clone(),
                github.bodies(),
            ),
            (
                Err((SubmitSyncStage::Link, "temporary tq failure".to_string())),
                vec![(
                    "leaf-settings".to_string(),
                    "https://github.com/acme/notify-web/pull/17".to_string(),
                )],
                expected_bodies,
            ),
        );
    }

    #[rstest]
    #[case::unknown_item(
        "missing-leaf",
        "notify-web",
        "checklist item `missing-leaf` is not part of the saved PR series plan"
    )]
    #[case::repository_mismatch(
        "leaf-settings",
        "notify-api",
        "checklist item `leaf-settings` belongs to `acme/notify-web`, not `acme/notify-api`"
    )]
    #[tokio::test]
    async fn rejects_invalid_branch_mappings_before_linking(
        #[case] checklist_item_id: &str,
        #[case] repo: &str,
        #[case] expected_error: &str,
    ) {
        let tq = fake_submit_tq(None, true);
        let github = FakeGitHub::default();
        github.add_body("acme/notify-api", 12, "Schema PR body");
        let expected_bodies = github.bodies();
        let mut context = current_pr_context();
        context.checklist_item_id = checklist_item_id;
        context.repo = repo;

        let result = link_and_sync_after_submit_with(
            || Ok(&tq),
            |_| Ok(Some(sample_plan())),
            &github,
            &context,
        )
        .await
        .map_err(|failure| (failure.stage, failure.error.to_string()));

        assert_eq!(
            (
                result,
                tq.link_calls.lock().unwrap().clone(),
                github.bodies(),
            ),
            (
                Err((SubmitSyncStage::Prepare, expected_error.to_string())),
                Vec::new(),
                expected_bodies,
            ),
        );
    }

    #[tokio::test]
    async fn reports_when_tq_does_not_expose_the_new_link() {
        let tq = fake_submit_tq(None, false);
        let plan = sample_plan();
        let github = FakeGitHub::default();
        github.add_body("acme/notify-api", 12, "Schema PR body");
        github.add_body("acme/notify-web", 17, "Settings PR body");
        let schema_only = created_pull_requests()
            .into_iter()
            .take(1)
            .collect::<Vec<_>>();
        let expected_bodies = HashMap::from([
            (
                ("acme/notify-api".to_string(), 12),
                expected_body(&plan, "schema", "Schema PR body", &schema_only),
            ),
            (
                ("acme/notify-web".to_string(), 17),
                "Settings PR body".to_string(),
            ),
        ]);

        let result = link_and_sync_after_submit_with(
            || Ok(&tq),
            |_| Ok(Some(plan)),
            &github,
            &current_pr_context(),
        )
        .await
        .map_err(|failure| (failure.stage, failure.error.to_string()));

        assert_eq!(
            (
                result,
                tq.link_calls.lock().unwrap().clone(),
                github.bodies(),
            ),
            (
                Err((
                    SubmitSyncStage::Link,
                    "the checklist link is not visible in tq yet".to_string(),
                )),
                vec![(
                    "leaf-settings".to_string(),
                    "https://github.com/acme/notify-web/pull/17".to_string(),
                )],
                expected_bodies,
            ),
        );
    }

    #[tokio::test]
    async fn reports_body_sync_failures_after_linking() {
        let tq = fake_submit_tq(None, true);
        let plan = sample_plan();
        let github = FakeGitHub::default();
        github.add_body("acme/notify-api", 12, "Schema PR body");
        github.add_body("acme/notify-web", 17, "Settings PR body");
        github.fail_update_once("acme/notify-web", 17);
        let pull_requests = created_pull_requests();
        let expected_bodies = HashMap::from([
            (
                ("acme/notify-api".to_string(), 12),
                expected_body(&plan, "schema", "Schema PR body", &pull_requests),
            ),
            (
                ("acme/notify-web".to_string(), 17),
                "Settings PR body".to_string(),
            ),
        ]);

        let result = link_and_sync_after_submit_with(
            || Ok(&tq),
            |_| Ok(Some(plan)),
            &github,
            &current_pr_context(),
        )
        .await
        .map_err(|failure| (failure.stage, failure.error.to_string()));

        assert_eq!(
            (
                result,
                tq.link_calls.lock().unwrap().clone(),
                github.bodies(),
            ),
            (
                Err((
                    SubmitSyncStage::Sync,
                    "1 pull request(s) could not be synchronized".to_string(),
                )),
                vec![(
                    "leaf-settings".to_string(),
                    "https://github.com/acme/notify-web/pull/17".to_string(),
                )],
                expected_bodies,
            ),
        );
    }
}
