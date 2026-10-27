use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, bail};
use clap::Args;

use super::model::{
    CreatedPullRequest, PrSeriesPlan, Repository, resolve_created_pull_requests,
    validate_checklist_leaf_ids,
};
use super::render::{render_section, replace_managed_section};
use super::storage;
use crate::infra::github::{GitHubClient, PrBodyClient, RepoClient, UpdatePrBodyParams};
use crate::infra::tq::TqClient;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SyncArgs {
    /// tq task number or UUID
    pub task_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct SyncReport {
    updated: Vec<String>,
    unchanged: Vec<String>,
    failed: Vec<(String, String)>,
}

struct PendingUpdate {
    pull_request: CreatedPullRequest,
    body: String,
}

pub async fn run(args: &SyncArgs) -> Result<()> {
    let tq_client = TqClient::detect().context("tq CLI is required to sync a PR series")?;
    let task = tq_client.task_details(&args.task_id).await?;
    let plan = storage::load_plan(&task.id)?;
    let checklists = tq_client.list_checklists(&task.id).await?;
    validate_checklist_leaf_ids(&plan, &checklists)?;
    let created_pull_requests =
        resolve_created_pull_requests(&plan, &checklists, &task.github_links)?;
    if created_pull_requests.is_empty() {
        println!(
            "No pull requests are linked to the plan for tq task {}.",
            task.number
        );
        return Ok(());
    }

    let github_client = GitHubClient::get()?;
    let report = sync_created_pull_requests(&plan, &created_pull_requests, github_client).await?;
    print_report(&report);
    if !report.failed.is_empty() {
        bail!(
            "{} pull request(s) could not be synchronized; run `a ai pr-series sync {}` again after resolving the listed errors",
            report.failed.len(),
            task.number
        );
    }
    Ok(())
}

async fn sync_created_pull_requests(
    plan: &PrSeriesPlan,
    created_pull_requests: &[CreatedPullRequest],
    github: &(impl PrBodyClient + RepoClient),
) -> Result<SyncReport> {
    plan.validate()?;
    let repositories = plan
        .pull_requests
        .iter()
        .map(|entry| {
            Repository::parse(&entry.repository).map(|repository| (repository.key(), repository))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut repository_privacy = HashMap::new();
    for (key, repository) in repositories {
        let is_private = github
            .is_repo_private(&repository.owner, &repository.name)
            .await
            .with_context(|| {
                format!(
                    "failed to determine visibility of {}",
                    repository.full_name()
                )
            })?;
        repository_privacy.insert(key, is_private);
    }

    let mut report = SyncReport::default();
    let mut pending_updates = Vec::new();
    for pull_request in created_pull_requests {
        let body = match github
            .get_pull_request_body(
                &pull_request.repository.owner,
                &pull_request.repository.name,
                pull_request.number,
            )
            .await
        {
            Ok(body) => body,
            Err(error) => {
                report
                    .failed
                    .push((pull_request.url.clone(), error.to_string()));
                continue;
            }
        };
        let section = match render_section(
            plan,
            &pull_request.entry_id,
            &body,
            &repository_privacy,
            created_pull_requests,
        ) {
            Ok(section) => section,
            Err(error) => {
                report
                    .failed
                    .push((pull_request.url.clone(), error.to_string()));
                continue;
            }
        };
        let updated_body = match replace_managed_section(&body, &section) {
            Ok(body) => body,
            Err(error) => {
                report
                    .failed
                    .push((pull_request.url.clone(), error.to_string()));
                continue;
            }
        };
        if updated_body == body {
            report.unchanged.push(pull_request.url.clone());
        } else {
            pending_updates.push(PendingUpdate {
                pull_request: pull_request.clone(),
                body: updated_body,
            });
        }
    }

    for pending in pending_updates {
        match github
            .update_pull_request_body(UpdatePrBodyParams {
                owner: pending.pull_request.repository.owner,
                repo: pending.pull_request.repository.name,
                number: pending.pull_request.number,
                body: pending.body,
            })
            .await
        {
            Ok(_) => report.updated.push(pending.pull_request.url),
            Err(error) => report
                .failed
                .push((pending.pull_request.url, error.to_string())),
        }
    }
    Ok(report)
}

fn print_report(report: &SyncReport) {
    for url in &report.updated {
        println!("Updated {url}");
    }
    for url in &report.unchanged {
        println!("Already synchronized {url}");
    }
    for (url, error) in &report.failed {
        eprintln!("Failed to synchronize {url}: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::Mutex;

    use anyhow::{Result, bail};

    use super::*;
    use crate::commands::ai::pr_series::model::PrSeriesEntry;
    use crate::infra::github::{CreatePrParams, PrBodyClient, PrClient, PrInfo, UpdatePrParams};

    #[derive(Default)]
    struct FakeGitHub {
        private_repositories: HashMap<String, bool>,
        visibility_errors: HashSet<String>,
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

        async fn is_repo_private(&self, owner: &str, repo: &str) -> Result<bool> {
            let key = format!("{owner}/{repo}").to_ascii_lowercase();
            if self.visibility_errors.contains(&key) {
                bail!("visibility lookup failed");
            }
            Ok(self
                .private_repositories
                .get(&key)
                .copied()
                .unwrap_or_default())
        }

        async fn get_default_branch(&self, _owner: &str, _repo: &str) -> Result<String> {
            Ok("main".to_string())
        }
    }

    impl PrClient for FakeGitHub {
        async fn create_pull_request(&self, _params: CreatePrParams) -> Result<String> {
            Ok(String::new())
        }

        async fn update_pull_request(&self, _params: UpdatePrParams) -> Result<String> {
            Ok(String::new())
        }

        async fn get_pr_for_branch(
            &self,
            _owner: &str,
            _repo: &str,
            _branch: &str,
        ) -> Result<Option<PrInfo>> {
            Ok(None)
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
                number: 31,
                url: "https://github.com/acme/notify-web/pull/31".to_string(),
            },
        ]
    }

    fn fake_github() -> FakeGitHub {
        let mut github = FakeGitHub::default();
        github
            .private_repositories
            .insert("acme/notify-api".to_string(), false);
        github
            .private_repositories
            .insert("acme/notify-web".to_string(), false);
        github.add_body("acme/notify-api", 12, "API details");
        github.add_body("acme/notify-web", 31, "Settings details");
        github
    }

    #[tokio::test]
    async fn retries_failed_updates_and_skips_bodies_already_in_sync() {
        let github = fake_github();
        github.fail_update_once("acme/notify-api", 12);
        let plan = sample_plan();
        let pull_requests = created_pull_requests();

        let first = sync_created_pull_requests(&plan, &pull_requests, &github)
            .await
            .unwrap();
        let second = sync_created_pull_requests(&plan, &pull_requests, &github)
            .await
            .unwrap();

        assert_eq!(
            (first, second),
            (
                SyncReport {
                    updated: vec!["https://github.com/acme/notify-web/pull/31".to_string()],
                    unchanged: Vec::new(),
                    failed: vec![(
                        "https://github.com/acme/notify-api/pull/12".to_string(),
                        "temporary API failure".to_string(),
                    )],
                },
                SyncReport {
                    updated: vec!["https://github.com/acme/notify-api/pull/12".to_string()],
                    unchanged: vec!["https://github.com/acme/notify-web/pull/31".to_string()],
                    failed: Vec::new(),
                },
            ),
        );
    }

    #[tokio::test]
    async fn stops_before_any_body_update_when_repository_visibility_is_unknown() {
        let mut github = fake_github();
        github
            .visibility_errors
            .insert("acme/notify-web".to_string());
        let plan = sample_plan();
        let pull_requests = created_pull_requests();
        let result = sync_created_pull_requests(&plan, &pull_requests, &github)
            .await
            .err()
            .map(|error| error.to_string());

        assert_eq!(
            (result, github.bodies()),
            (
                Some("failed to determine visibility of acme/notify-web".to_string()),
                HashMap::from([
                    (
                        ("acme/notify-api".to_string(), 12),
                        "API details".to_string()
                    ),
                    (
                        ("acme/notify-web".to_string(), 31),
                        "Settings details".to_string()
                    ),
                ]),
            ),
        );
    }
}
