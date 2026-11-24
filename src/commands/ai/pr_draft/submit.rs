use clap::Args;
use std::path::{Path, PathBuf};

use super::common::{DraftFile, PrDraftError, RepoInfo, contains_japanese, repo_allows_japanese};
use super::hooks::{HookContext, HookRunner, PRE_PR_SUBMIT_HOOK, default_runner, run_pr_hook};
use super::stack::stack_created_pull_request;
use crate::commands::agent::{ensure_no_review_access, is_vibe_enabled};
use crate::commands::ai::pr_series::has_managed_section;
use crate::infra::git::cmd::run_git;
use crate::infra::github::{
    CreatePrParams, GitHubClient, PrBodyClient, PrClient, PrStackClient, PrState, RepoClient,
    UpdatePrParams,
};
use crate::shared::config::load_config;
use crate::shared::human_in_the_loop::LockGuard;

mod post_submit;

use post_submit::{
    PostSubmitProcessor, PrSeriesPostSubmitProcessor, SubmitEffects, body_for_existing_pr_update,
    format_post_submit_failure, record_created_pr_url,
};
#[cfg(test)]
use post_submit::{SavedPrSeriesPlan, TqBranchMetadata, record_created_pr_url_in};

const SERIES_MARKERS_REQUIRED_ERROR: &str = indoc::indoc! {"
    A PR series plan with at least two pull requests requires both markers in the PR body. Add them on their own lines after the bullet list in the changes section:
    <!-- pr-series:start -->
    <!-- pr-series:end -->
"};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SubmitArgs {
    /// Path to the draft file (auto-detected if not specified)
    pub filepath: Option<PathBuf>,

    /// Base branch for the PR
    #[arg(long)]
    pub base: Option<String>,

    /// Do not add the pull request to a GitHub stack
    #[arg(long)]
    pub no_stack: bool,

    /// Create as draft PR
    #[arg(long)]
    pub draft: bool,

    /// Skip draft approval verification when vibe mode and the target repository are allowed
    #[arg(long)]
    pub no_review: bool,
}

/// Holds the target repository and branch information for PR creation
struct PrTarget {
    owner: String,
    repo: String,
    branch: String,
    is_private: bool,
}

pub async fn run(args: &SubmitArgs) -> anyhow::Result<()> {
    let client = GitHubClient::get()?;
    let post_submit_processor = PrSeriesPostSubmitProcessor { github: client };
    let submit_effects = SubmitEffects {
        post_submit_processor: &post_submit_processor,
        record_created_pr_url,
    };
    run_impl_with_recording(
        args,
        client,
        default_runner(),
        write_pr_url_config,
        read_current_repo_identity,
        &submit_effects,
        |owner, repo| ensure_no_review_access(is_vibe_enabled, load_config, owner, repo),
    )
    .await
}

async fn run_impl_with_recording<P: PostSubmitProcessor, R: Fn(&str)>(
    args: &SubmitArgs,
    gh_client: &(impl PrBodyClient + PrClient + PrStackClient + RepoClient),
    run_hook: HookRunner<'_>,
    write_config: impl Fn(&str, &str) -> anyhow::Result<()>,
    current_repo_identity: impl FnOnce() -> Option<(String, String)>,
    submit_effects: &SubmitEffects<'_, P, R>,
    vibe_access_check: impl FnOnce(&str, &str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    // Get draft path and target repo info
    let (draft_path, target) = match &args.filepath {
        Some(path) => {
            // When filepath is provided, parse it to get owner/repo/branch
            let (owner, repo, branch) = DraftFile::parse_path(path).ok_or_else(|| {
                PrDraftError::CommandFailed(format!("Invalid draft path: {}", path.display()))
            })?;
            let is_private = gh_client.is_repo_private(&owner, &repo).await?;
            (
                path.clone(),
                PrTarget {
                    owner,
                    repo,
                    branch,
                    is_private,
                },
            )
        }
        None => {
            // Auto-detect from current git repo
            let repo_info = RepoInfo::from_current_dir_async(gh_client).await?;
            let target = PrTarget {
                owner: repo_info.owner.clone(),
                repo: repo_info.repo.clone(),
                branch: repo_info.branch.clone(),
                is_private: repo_info.is_private,
            };
            (DraftFile::path_for(&repo_info), target)
        }
    };
    let is_private = target.is_private;

    // Check for a live review holder before submitting.
    let lock_path = LockGuard::lock_path(&draft_path);
    if LockGuard::is_locked(&draft_path) {
        anyhow::bail!(
            "Please close the editor before submitting the PR (lock: {}).",
            lock_path.display()
        );
    }

    let draft = DraftFile::from_path(draft_path.clone())?;

    if args.no_review {
        vibe_access_check(&target.owner, &target.repo)?;
    } else {
        draft.verify_approval()?;
    }

    // Validate title
    if draft.frontmatter.title.trim().is_empty() {
        return Err(PrDraftError::EmptyTitle.into());
    }

    // Reject Japanese content when the effective repository language is not Japanese.
    let allows_japanese = repo_allows_japanese(&target.owner, &target.repo, is_private).await?;
    if !allows_japanese {
        if contains_japanese(&draft.frontmatter.title) {
            return Err(PrDraftError::JapaneseInTitle.into());
        }
        if contains_japanese(&draft.body) {
            return Err(PrDraftError::JapaneseInBody.into());
        }
    }

    let target_is_current_repo = if args.filepath.is_some() {
        current_repo_identity()
            .is_some_and(|(owner, repo)| owner == target.owner && repo == target.repo)
    } else {
        true
    };
    let tq_metadata = if target_is_current_repo {
        submit_effects
            .post_submit_processor
            .metadata_for_branch(&target.branch)
    } else {
        None
    };
    let saved_plan = if let Some(metadata) = &tq_metadata {
        submit_effects
            .post_submit_processor
            .saved_plan(metadata)
            .await
    } else {
        None
    };
    if saved_plan
        .as_ref()
        .is_some_and(|saved_plan| saved_plan.plan.pull_requests.len() >= 2)
    {
        match has_managed_section(&draft.body) {
            Ok(true) => {}
            Ok(false) => anyhow::bail!("{SERIES_MARKERS_REQUIRED_ERROR}"),
            Err(error) => anyhow::bail!("{error}\n{SERIES_MARKERS_REQUIRED_ERROR}"),
        }
    }

    // Check if PR already exists for this branch
    let existing_pr = gh_client
        .get_pr_for_branch(&target.owner, &target.repo, &target.branch)
        .await?;

    let update_target = match &existing_pr {
        Some(pr_info) if pr_info.state == PrState::Open => Some(pr_info.number),
        _ => None,
    };
    // Fire pre-pr-submit hook before any network call. The hook script can
    // inspect the title/body and abort submission with a non-zero exit, which
    // lets users enforce custom rules (e.g., forbid cross-org issue links).
    let context = HookContext {
        owner: &target.owner,
        repo: &target.repo,
        head_branch: &target.branch,
        base_branch: args.base.as_deref().unwrap_or(""),
        update_pr_number: update_target,
    };
    run_pr_hook(
        PRE_PR_SUBMIT_HOOK,
        &draft.frontmatter.title,
        &draft.body,
        &context,
        run_hook,
    )?;

    let pr_url = match update_target {
        Some(number) => {
            // Update existing PR
            let body = body_for_existing_pr_update(
                gh_client,
                &target.owner,
                &target.repo,
                number,
                &draft.body,
                saved_plan.is_some(),
                tq_metadata.as_ref(),
            )
            .await;
            gh_client
                .update_pull_request(UpdatePrParams {
                    owner: target.owner.clone(),
                    repo: target.repo.clone(),
                    number,
                    title: draft.frontmatter.title.clone(),
                    body,
                })
                .await?
        }
        None => {
            // Create new PR
            let created_pr = gh_client
                .create_pull_request(CreatePrParams {
                    owner: target.owner.clone(),
                    repo: target.repo.clone(),
                    title: draft.frontmatter.title.clone(),
                    body: draft.body.clone(),
                    head: target.branch.clone(),
                    base: args.base.clone(),
                    draft: args.draft,
                })
                .await?;

            if !args.no_stack
                && let Some(base_branch) = args.base.as_deref()
                && let Some(warning) = stack_created_pull_request(
                    gh_client,
                    &target.owner,
                    &target.repo,
                    base_branch,
                    created_pr.number,
                )
                .await
            {
                eprintln!("{warning}");
            }

            created_pr.url
        }
    };

    (submit_effects.record_created_pr_url)(&pr_url);

    if target_is_current_repo {
        let config_key = format!("branch.{}.pr-url", target.branch);
        if let Err(error) = write_config(&config_key, &pr_url) {
            tracing::warn!(
                branch = %target.branch,
                error = %error,
                "failed to cache pull request URL"
            );
        }
    }

    println!("{pr_url}");

    if let Some(metadata) = tq_metadata
        && let Err(failure) = submit_effects
            .post_submit_processor
            .link_and_sync(&metadata, &target, &pr_url, saved_plan)
            .await
    {
        eprintln!(
            "{}",
            format_post_submit_failure(&failure, &metadata, &pr_url)
        );
    }

    // Cleanup
    draft.cleanup()?;

    Ok(())
}

fn write_pr_url_config(key: &str, url: &str) -> anyhow::Result<()> {
    write_pr_url_config_with(key, url, |path, args| run_git(path, args))
}

fn write_pr_url_config_with(
    key: &str,
    url: &str,
    run_git: impl FnOnce(&Path, [&str; 4]) -> anyhow::Result<String>,
) -> anyhow::Result<()> {
    run_git(Path::new("."), ["config", "--local", key, url])?;
    Ok(())
}

fn read_current_repo_identity() -> Option<(String, String)> {
    let repo_info = RepoInfo::from_git_only().ok()?;
    Some((repo_info.owner, repo_info.repo))
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::commands::agent::types::SessionPrContext;
    use crate::commands::ai::pr_series::PrSeriesPlan;
    use crate::commands::ai::pr_series_aftercare::{
        SubmitSyncFailure, SubmitSyncOutcome, SubmitSyncStage,
    };
    use crate::infra::github::error::Result as GitHubResult;
    use crate::infra::github::{
        CreatedPullRequest, GitHubMockServer, PullRequestStack, UpdatePrBodyParams,
    };
    use crate::shared::config::Config;
    use crate::shared::env_var::EnvVars;
    use clap::Parser;
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use std::fs;
    use std::sync::Mutex;

    /// Hook stub that always succeeds. Use when the test does not care about
    /// hook invocation.
    fn noop_hook(_name: &str, _env: &[(&str, &str)]) -> anyhow::Result<()> {
        Ok(())
    }

    fn noop_config(_key: &str, _value: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn series_plan(pull_request_count: usize) -> PrSeriesPlan {
        PrSeriesPlan {
            name: "sample series".to_string(),
            purpose: "exercise submit checks".to_string(),
            pull_requests: (0..pull_request_count)
                .map(|index| crate::commands::ai::pr_series::PrSeriesEntry {
                    id: format!("sample-{index}"),
                    summary: format!("Sample change {index}"),
                    depends_on: Vec::new(),
                    repository: "example-owner/example-repo".to_string(),
                    tq_leaf_id: format!("sample-leaf-{index}"),
                })
                .collect(),
            steps: None,
        }
    }

    struct NoopPostSubmitProcessor;

    impl PostSubmitProcessor for NoopPostSubmitProcessor {
        fn metadata_for_branch(&self, _branch: &str) -> Option<TqBranchMetadata> {
            None
        }

        async fn saved_plan(&self, _metadata: &TqBranchMetadata) -> Option<SavedPrSeriesPlan> {
            None
        }

        async fn link_and_sync(
            &self,
            _metadata: &TqBranchMetadata,
            _target: &PrTarget,
            _pr_url: &str,
            _saved_plan: Option<SavedPrSeriesPlan>,
        ) -> std::result::Result<SubmitSyncOutcome, SubmitSyncFailure> {
            Ok(SubmitSyncOutcome::NoPlan)
        }
    }

    fn no_current_repo() -> Option<(String, String)> {
        None
    }

    fn no_vibe_access(_owner: &str, _repo: &str) -> anyhow::Result<()> {
        anyhow::bail!("The --no-review option is only available when vibe mode is enabled.")
    }

    fn noop_record_created_pr_url(_pr_url: &str) {}

    #[fixture]
    fn sessions_dir_with_cwd_pr_context() -> tempfile::TempDir {
        let sessions_dir = tempfile::tempdir().expect("temporary session directory should exist");
        let session: crate::commands::agent::types::Session =
            serde_json::from_value(serde_json::json!({
                "session_id": "sample-session",
                "pr_context": {
                    "owner": "sample-owner",
                    "repo": "sample-repository",
                    "branch": "cwd-branch",
                },
                "cwd": "/workspace/sample",
                "transcript_path": null,
                "tmux_info": null,
                "status": "stopped",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "last_message": null,
            }))
            .expect("fixture session should deserialize");
        crate::commands::agent::store::save_session_to(sessions_dir.path(), &session)
            .expect("fixture session should save");
        sessions_dir
    }

    async fn run_impl(
        args: &SubmitArgs,
        gh_client: &(impl PrBodyClient + PrClient + PrStackClient + RepoClient),
        run_hook: HookRunner<'_>,
        write_config: impl Fn(&str, &str) -> anyhow::Result<()>,
        current_repo_identity: impl FnOnce() -> Option<(String, String)>,
        post_submit_processor: &impl PostSubmitProcessor,
        vibe_access_check: impl FnOnce(&str, &str) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let submit_effects = SubmitEffects {
            post_submit_processor,
            record_created_pr_url: noop_record_created_pr_url,
        };
        super::run_impl_with_recording(
            args,
            gh_client,
            run_hook,
            write_config,
            current_repo_identity,
            &submit_effects,
            vibe_access_check,
        )
        .await
    }

    /// Captures every hook invocation so tests can assert on hook name, env,
    /// and the contents of any temp file referenced by env. The body file is
    /// read **inside** the hook closure because `run_pr_hook` deletes the
    /// temp file as soon as the hook returns.
    #[derive(Default)]
    struct HookSpy {
        calls: Mutex<Vec<HookCall>>,
    }

    type PostSubmitCall = (String, String, String, String, String);

    #[derive(Default)]
    struct PostSubmitSpy {
        calls: Mutex<Vec<PostSubmitCall>>,
        failure_stage: Option<SubmitSyncStage>,
        metadata: Option<TqBranchMetadata>,
        plan_pull_request_count: Option<usize>,
    }

    impl PostSubmitProcessor for PostSubmitSpy {
        fn metadata_for_branch(&self, _branch: &str) -> Option<TqBranchMetadata> {
            self.metadata.clone()
        }

        async fn saved_plan(&self, metadata: &TqBranchMetadata) -> Option<SavedPrSeriesPlan> {
            self.plan_pull_request_count.map(|count| SavedPrSeriesPlan {
                task_id: metadata.task_id.clone(),
                plan: series_plan(count),
            })
        }

        async fn link_and_sync(
            &self,
            metadata: &TqBranchMetadata,
            target: &PrTarget,
            pr_url: &str,
            _saved_plan: Option<SavedPrSeriesPlan>,
        ) -> std::result::Result<SubmitSyncOutcome, SubmitSyncFailure> {
            self.calls.lock().expect("post-submit spy mutex").push((
                metadata.task_id.clone(),
                metadata.checklist_item_id.clone(),
                target.owner.clone(),
                target.repo.clone(),
                pr_url.to_string(),
            ));
            match self.failure_stage {
                Some(stage) => Err(SubmitSyncFailure {
                    stage,
                    error: anyhow::anyhow!("sample synchronization failure"),
                }),
                None => Ok(SubmitSyncOutcome::Synchronized),
            }
        }
    }

    #[fixture]
    fn post_submit_spy(#[default(true)] saved_plan: bool) -> PostSubmitSpy {
        PostSubmitSpy {
            metadata: Some(TqBranchMetadata {
                task_id: "sample-task-id".to_string(),
                checklist_item_id: "sample-leaf-id".to_string(),
            }),
            plan_pull_request_count: saved_plan.then_some(1),
            ..PostSubmitSpy::default()
        }
    }

    #[rstest]
    #[case::series_without_markers(
        "series-marker-missing",
        Some(2),
        true,
        false,
        "Body content",
        Err(SERIES_MARKERS_REQUIRED_ERROR.to_string()),
        false,
    )]
    #[case::series_with_only_one_marker(
        "series-marker-partial",
        Some(2),
        true,
        false,
        "<!-- pr-series:start -->",
        Err(format!(
            "PR series markers must occur exactly once\n{SERIES_MARKERS_REQUIRED_ERROR}"
        )),
        false,
    )]
    #[case::series_with_markers(
        "series-marker-present",
        Some(2),
        true,
        false,
        indoc! {"
            ## What changed

            - Add a sample change.

            <!-- pr-series:start -->
            <!-- pr-series:end -->
        "},
        Ok(()),
        true,
    )]
    #[case::single_pr_series_without_markers(
        "series-marker-single",
        Some(1),
        true,
        false,
        "Body content",
        Ok(()),
        true
    )]
    #[case::standalone_pr_without_markers(
        "series-marker-standalone",
        None,
        false,
        false,
        "Body content",
        Ok(()),
        true
    )]
    #[case::existing_series_without_markers(
        "series-marker-existing",
        Some(2),
        true,
        true,
        "Body content",
        Err(SERIES_MARKERS_REQUIRED_ERROR.to_string()),
        false
    )]
    #[tokio::test]
    async fn submit_requires_series_markers_only_for_plans_with_multiple_pull_requests(
        #[case] repo: &str,
        #[case] plan_pull_request_count: Option<usize>,
        #[case] has_tq_metadata: bool,
        #[case] existing_pr: bool,
        #[case] body: &str,
        #[case] expected_result: Result<(), String>,
        #[case] expected_pr_mutation: bool,
    ) {
        let env = if existing_pr {
            setup_test_env_with_existing_pr(
                "example-owner",
                repo,
                "feature/sample-change",
                42,
                ExistingPrBody::Text("Existing body"),
                body,
            )
            .await
        } else {
            setup_test_env("example-owner", repo).await
        };
        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/sample-change",
            "Add a sample change",
            body,
        );
        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };
        let processor = PostSubmitSpy {
            metadata: has_tq_metadata.then(|| TqBranchMetadata {
                task_id: "sample-task-id".to_string(),
                checklist_item_id: "sample-leaf-id".to_string(),
            }),
            plan_pull_request_count,
            ..PostSubmitSpy::default()
        };
        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            || Some((env.owner.clone(), env.repo.clone())),
            &processor,
            no_vibe_access,
        )
        .await
        .map_err(|error| error.to_string());
        let mutations = env
            .mock
            .received_requests()
            .await
            .iter()
            .filter(|request| {
                matches!(request.method.as_str(), "POST" | "PATCH")
                    && request.url.path().contains("/pulls")
            })
            .map(|request| (request.method.to_string(), request.url.path().to_string()))
            .collect::<Vec<_>>();
        let expected_mutations = if expected_pr_mutation {
            vec![(
                if existing_pr {
                    "PATCH".to_string()
                } else {
                    "POST".to_string()
                },
                if existing_pr {
                    format!("/repos/example-owner/{repo}/pulls/42")
                } else {
                    format!("/repos/example-owner/{repo}/pulls")
                },
            )]
        } else {
            Vec::new()
        };

        assert_eq!((result, mutations), (expected_result, expected_mutations));
    }

    #[derive(Clone)]
    struct HookCall {
        name: String,
        env: Vec<(String, String)>,
        body_file_contents: Option<String>,
    }

    impl HookSpy {
        fn record(&self, name: &str, env: &[(&str, &str)]) {
            let snapshot: Vec<(String, String)> = env
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect();
            let body_file_contents = snapshot
                .iter()
                .find(|(k, _)| k == EnvVars::pr_body_file_name())
                .and_then(|(_, path)| fs::read_to_string(path).ok());
            self.calls.lock().expect("hook spy mutex").push(HookCall {
                name: name.to_string(),
                env: snapshot,
                body_file_contents,
            });
        }

        fn calls(&self) -> Vec<HookCall> {
            self.calls.lock().expect("hook spy mutex").clone()
        }
    }

    fn lookup<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    struct TestEnv {
        mock: GitHubMockServer,
        owner: String,
        repo: String,
        draft_dir: std::path::PathBuf,
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.draft_dir);
        }
    }

    struct TestDraftDirectory {
        path: PathBuf,
    }

    impl TestDraftDirectory {
        fn new(owner: &str, repo: &str) -> Self {
            crate::shared::testing::init_approval_dir();
            let path = DraftFile::draft_dir().join(owner).join(repo);
            if path.exists() {
                let _ = fs::remove_dir_all(&path);
            }
            Self { path }
        }
    }

    impl Drop for TestDraftDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    type CreatePrCall = (String, String, String, String, Option<String>, bool);

    #[derive(Default)]
    struct SubmitClientStub {
        create_calls: Mutex<Vec<CreatePrCall>>,
        update_calls: Mutex<Vec<u64>>,
        existing_pr_number: Option<u64>,
    }

    impl RepoClient for SubmitClientStub {
        async fn repo_exists(&self, _owner: &str, _repo: &str) -> GitHubResult<bool> {
            Ok(true)
        }

        async fn is_repo_private(&self, _owner: &str, _repo: &str) -> GitHubResult<bool> {
            Ok(true)
        }

        async fn get_default_branch(&self, _owner: &str, _repo: &str) -> GitHubResult<String> {
            Ok("main".to_string())
        }
    }

    impl PrClient for SubmitClientStub {
        async fn create_pull_request(
            &self,
            params: CreatePrParams,
        ) -> GitHubResult<CreatedPullRequest> {
            self.create_calls
                .lock()
                .expect("submit client mutex")
                .push((
                    params.owner,
                    params.repo,
                    params.title,
                    params.head,
                    params.base,
                    params.draft,
                ));
            Ok(CreatedPullRequest {
                number: 1,
                url: "https://github.com/sample-owner/sample-repository/pull/1".to_string(),
            })
        }

        async fn update_pull_request(&self, params: UpdatePrParams) -> GitHubResult<String> {
            self.update_calls
                .lock()
                .expect("submit client mutex")
                .push(params.number);
            Ok(format!(
                "https://github.com/sample-owner/sample-repository/pull/{}",
                params.number
            ))
        }

        async fn get_pr_for_branch(
            &self,
            owner: &str,
            repo: &str,
            _branch: &str,
        ) -> GitHubResult<Option<crate::infra::github::PrInfo>> {
            Ok(self
                .existing_pr_number
                .map(|number| crate::infra::github::PrInfo {
                    number,
                    title: "Existing pull request".to_string(),
                    state: PrState::Open,
                    url: format!("https://github.com/{owner}/{repo}/pull/{number}"),
                }))
        }

        async fn get_open_pr_for_branch(
            &self,
            _owner: &str,
            _repo: &str,
            _branch: &str,
        ) -> GitHubResult<Option<crate::infra::github::PrInfo>> {
            Ok(None)
        }
    }

    impl PrBodyClient for SubmitClientStub {
        async fn get_pull_request_body(
            &self,
            _owner: &str,
            _repo: &str,
            _number: u64,
        ) -> GitHubResult<String> {
            Ok(String::new())
        }

        async fn update_pull_request_body(
            &self,
            _params: UpdatePrBodyParams,
        ) -> GitHubResult<String> {
            Ok("https://github.com/sample-owner/sample-repository/pull/1".to_string())
        }
    }

    impl PrStackClient for SubmitClientStub {
        async fn get_stack_for_pull_request(
            &self,
            _owner: &str,
            _repo: &str,
            _pull_request: u64,
        ) -> GitHubResult<Option<PullRequestStack>> {
            Ok(None)
        }

        async fn create_stack(
            &self,
            _owner: &str,
            _repo: &str,
            _pull_requests: &[u64],
        ) -> GitHubResult<()> {
            Ok(())
        }

        async fn add_pull_request_to_stack(
            &self,
            _owner: &str,
            _repo: &str,
            _stack_number: u64,
            _pull_request: u64,
        ) -> GitHubResult<()> {
            Ok(())
        }
    }

    async fn setup_test_env(owner: &str, repo: &str) -> TestEnv {
        let env = setup_test_env_without_pull_request_mocks(owner, repo).await;
        let ctx = env.mock.repo(owner, repo);
        ctx.list_pull_requests_empty().await;
        ctx.pull_request(1).create().await;
        env
    }

    async fn setup_test_env_without_pull_request_mocks(owner: &str, repo: &str) -> TestEnv {
        crate::shared::testing::init_approval_dir();
        let mock = GitHubMockServer::start().await;
        let ctx = mock.repo(owner, repo);
        ctx.repo_info().private(true).get().await;
        let draft_dir = DraftFile::draft_dir().join(owner).join(repo);
        if draft_dir.exists() {
            let _ = fs::remove_dir_all(&draft_dir);
        }
        TestEnv {
            mock,
            owner: owner.to_string(),
            repo: repo.to_string(),
            draft_dir,
        }
    }

    fn create_approved_draft(
        owner: &str,
        repo: &str,
        branch: &str,
        title: &str,
        body: &str,
    ) -> PathBuf {
        create_draft(owner, repo, branch, title, body, true)
    }

    fn create_unapproved_draft(
        owner: &str,
        repo: &str,
        branch: &str,
        title: &str,
        body: &str,
    ) -> PathBuf {
        create_draft(owner, repo, branch, title, body, false)
    }

    fn create_draft(
        owner: &str,
        repo: &str,
        branch: &str,
        title: &str,
        body: &str,
        approved: bool,
    ) -> PathBuf {
        let repo_info = RepoInfo {
            owner: owner.to_string(),
            repo: repo.to_string(),
            branch: branch.to_string(),
            is_private: true,
        };
        let draft_path = DraftFile::path_for(&repo_info);
        if let Some(parent) = draft_path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        let content = format!(
            indoc! {r#"
                ---
                title: "{title}"
                steps:
                  submit: {approved}
                ---
                {body}
            "#},
            title = title,
            body = body,
            approved = approved,
        );
        fs::write(&draft_path, content).expect("write draft");
        if approved {
            DraftFile::from_path(draft_path.clone())
                .expect("draft file")
                .save_approval()
                .expect("save approval");
        }
        draft_path
    }

    #[test]
    fn submit_parses_no_review_flag() {
        let parsed =
            crate::cli::Cli::try_parse_from(["a", "ai", "pr-draft", "submit", "--no-review"])
                .expect("--no-review should be a valid submit option");

        assert!(
            parsed.command
                == crate::cli::Commands::Ai(crate::commands::ai::AiCommands::PrDraft(
                    super::super::PrDraftCommands::Submit(SubmitArgs {
                        filepath: None,
                        base: None,
                        draft: false,
                        no_review: true,
                        no_stack: false,
                    }),
                )),
        );
    }

    #[test]
    fn submit_parses_no_stack_flag_with_base() {
        let parsed = crate::cli::Cli::try_parse_from([
            "a",
            "ai",
            "pr-draft",
            "submit",
            "--base",
            "renovate/example",
            "--no-stack",
        ])
        .expect("--no-stack should be a valid submit option");

        assert!(
            parsed.command
                == crate::cli::Commands::Ai(crate::commands::ai::AiCommands::PrDraft(
                    super::super::PrDraftCommands::Submit(SubmitArgs {
                        filepath: None,
                        base: Some("renovate/example".to_string()),
                        no_stack: true,
                        draft: false,
                        no_review: false,
                    }),
                )),
        );
    }

    #[rstest]
    #[case::vibe_enabled(
        "repo_submit_no_review_vibe",
        true,
        indoc! {"
            orgs:
              example-org:
                vibe: true
        "},
        Ok(()),
        true
    )]
    #[case::repo_disabled(
        "repo_submit_no_review_config_denied",
        true,
        "{}",
        Err("Vibe mode is disabled by configuration for repository example-org/repo_submit_no_review_config_denied.".to_string()),
        false
    )]
    #[case::vibe_disabled(
        "repo_submit_no_review_regular",
        false,
        indoc! {"
            orgs:
              example-org:
                vibe: true
        "},
        Err("The --no-review option is only available when vibe mode is enabled.".to_string()),
        false
    )]
    #[tokio::test]
    async fn submit_with_no_review_requires_vibe_and_repo_access(
        #[case] repo: &str,
        #[case] vibe_enabled: bool,
        #[case] config_yaml: &str,
        #[case] expected_result: Result<(), String>,
        #[case] expected_hook_invoked: bool,
    ) {
        let env = setup_test_env("example-org", repo).await;
        let config: Config = serde_yaml::from_str(config_yaml).unwrap();
        let draft_path = create_unapproved_draft(
            &env.owner,
            &env.repo,
            "feature/no-review",
            "Ready title",
            "Body content",
        );
        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: true,
            no_stack: false,
        };

        let target_repo = std::cell::RefCell::new(None);
        let spy = HookSpy::default();
        let hook = |name: &str, env: &[(&str, &str)]| {
            spy.record(name, env);
            Ok(())
        };
        let client = env.mock.client();
        let vibe_access_check = |owner: &str, repo: &str| {
            target_repo.replace(Some((owner.to_string(), repo.to_string())));
            ensure_no_review_access(|| Ok(vibe_enabled), || Ok(config), owner, repo)
        };
        let result = run_impl(
            &args,
            &client,
            &hook,
            noop_config,
            no_current_repo,
            &NoopPostSubmitProcessor,
            vibe_access_check,
        )
        .await
        .map_err(|error| error.to_string());
        let hook_names = spy
            .calls()
            .into_iter()
            .map(|call| call.name)
            .collect::<Vec<_>>();
        let expected_hook_names = if expected_hook_invoked {
            vec![PRE_PR_SUBMIT_HOOK.to_string()]
        } else {
            Vec::new()
        };

        assert_eq!(
            (result, hook_names, target_repo.into_inner()),
            (
                expected_result,
                expected_hook_names,
                Some((env.owner.clone(), env.repo.clone())),
            ),
        );
    }

    #[tokio::test]
    async fn submit_with_filepath_should_not_require_git_repo() {
        let env = setup_test_env("owner", "repo_submit_no_git").await;
        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/test",
            "Ready title",
            "Body content",
        );
        fs::write(
            LockGuard::lock_path(&draft_path),
            indoc! {"
                armyknife-review-lock-v1
                id=expired-session
                pid=4294967295
            "},
        )
        .unwrap();

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };

        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            no_current_repo,
            &NoopPostSubmitProcessor,
            no_vibe_access,
        )
        .await;

        assert_eq!(result.map_err(|error| error.to_string()), Ok(()));
    }

    #[tokio::test]
    async fn submit_fails_with_the_path_of_a_live_review_lock() {
        let env = setup_test_env("owner", "repo_submit_live_lock").await;
        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/test",
            "Ready title",
            "Body content",
        );
        let lock_guard = LockGuard::acquire(&draft_path).unwrap();
        let lock_path = LockGuard::lock_path(&draft_path);
        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };

        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            no_current_repo,
            &NoopPostSubmitProcessor,
            no_vibe_access,
        )
        .await
        .map_err(|error| error.to_string());
        drop(lock_guard);

        assert_eq!(
            result,
            Err(format!(
                "Please close the editor before submitting the PR (lock: {}).",
                lock_path.display()
            )),
        );
    }

    #[tokio::test]
    async fn submit_with_filepath_should_use_draft_branch() {
        let env = setup_test_env("owner", "repo_submit_branch").await;
        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/missing-head",
            "Ready title",
            "Body content",
        );

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };

        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            no_current_repo,
            &NoopPostSubmitProcessor,
            no_vibe_access,
        )
        .await;

        assert!(
            result.is_ok(),
            "submit should pass the draft branch when --filepath is used: {:?}",
            result.err()
        );
    }

    #[rstest]
    #[tokio::test]
    async fn post_submit_sync_failure_does_not_undo_pr_creation(
        mut post_submit_spy: PostSubmitSpy,
    ) {
        let env = setup_test_env("acme", "aftercare-failure-fixture").await;
        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/notify-setting",
            "Add notification setting",
            "Body content",
        );
        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };
        post_submit_spy.failure_stage = Some(SubmitSyncStage::Link);
        let client = env.mock.client();

        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            || Some((env.owner.clone(), env.repo.clone())),
            &post_submit_spy,
            no_vibe_access,
        )
        .await
        .map_err(|error| error.to_string());

        assert_eq!(
            (
                result,
                post_submit_spy
                    .calls
                    .lock()
                    .expect("post-submit spy mutex")
                    .clone()
            ),
            (
                Ok(()),
                vec![(
                    "sample-task-id".to_string(),
                    "sample-leaf-id".to_string(),
                    "acme".to_string(),
                    "aftercare-failure-fixture".to_string(),
                    "https://github.com/acme/aftercare-failure-fixture/pull/1".to_string(),
                )],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn skips_post_submit_sync_when_filepath_targets_another_repo(
        #[with(false)] post_submit_spy: PostSubmitSpy,
    ) {
        let env = setup_test_env("acme", "aftercare-target-fixture").await;
        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/notify-setting",
            "Add notification setting",
            "Body content",
        );
        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };
        let client = env.mock.client();

        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            || Some(("acme".to_string(), "notify-web".to_string())),
            &post_submit_spy,
            no_vibe_access,
        )
        .await
        .map_err(|error| error.to_string());

        assert_eq!(
            (
                result,
                post_submit_spy
                    .calls
                    .lock()
                    .expect("post-submit spy mutex")
                    .clone()
            ),
            (Ok(()), Vec::new()),
        );
    }

    #[tokio::test]
    async fn submit_fails_when_not_approved() {
        let env = setup_test_env("owner", "repo_submit_not_approved").await;
        let repo_info = RepoInfo {
            owner: env.owner.clone(),
            repo: env.repo.clone(),
            branch: "feature/unapproved".to_string(),
            is_private: true,
        };
        let draft_path = DraftFile::path_for(&repo_info);
        if let Some(parent) = draft_path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(
            &draft_path,
            indoc! {r#"
                ---
                title: "Not approved"
                steps:
                  submit: false
                ---
                Body
            "#},
        )
        .expect("write draft");

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };

        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            no_current_repo,
            &NoopPostSubmitProcessor,
            |_, _| Ok(()),
        )
        .await;

        let err = result.expect_err("submit should fail when not approved");
        assert!(matches!(
            err.downcast_ref::<PrDraftError>(),
            Some(PrDraftError::NotApproved)
        ));
    }

    #[tokio::test]
    async fn submit_fails_with_empty_title() {
        let env = setup_test_env("owner", "repo_submit_empty_title").await;
        let repo_info = RepoInfo {
            owner: env.owner.clone(),
            repo: env.repo.clone(),
            branch: "feature/empty-title".to_string(),
            is_private: true,
        };
        let draft_path = DraftFile::path_for(&repo_info);
        if let Some(parent) = draft_path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(
            &draft_path,
            indoc! {r#"
                ---
                title: ""
                steps:
                  submit: true
                ---
                Body
            "#},
        )
        .expect("write draft");
        DraftFile::from_path(draft_path.clone())
            .expect("draft file")
            .save_approval()
            .expect("save approval");

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };

        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            no_current_repo,
            &NoopPostSubmitProcessor,
            no_vibe_access,
        )
        .await;

        let err = result.expect_err("submit should fail with empty title");
        assert!(matches!(
            err.downcast_ref::<PrDraftError>(),
            Some(PrDraftError::EmptyTitle)
        ));
    }

    #[derive(Clone, Copy)]
    enum ExistingPrBody<'a> {
        Text(&'a str),
        MalformedMarkers,
        ReadFailure,
    }

    async fn setup_test_env_with_existing_pr(
        owner: &str,
        repo: &str,
        branch: &str,
        pr_number: u64,
        existing_body: ExistingPrBody<'_>,
        updated_body: &str,
    ) -> TestEnv {
        crate::shared::testing::init_approval_dir();
        let mock = GitHubMockServer::start().await;
        let ctx = mock.repo(owner, repo);
        ctx.repo_info().private(true).get().await;
        let existing_body = match existing_body {
            ExistingPrBody::Text(body) => Some(body),
            ExistingPrBody::MalformedMarkers => Some(
                indoc! {"
                    Existing body
                    <!-- pr-series:start -->
                "}
                .trim_end_matches('\n'),
            ),
            ExistingPrBody::ReadFailure => None,
        };
        ctx.list_pull_requests_with(pr_number, "Old title", existing_body.unwrap_or(""), branch)
            .await;
        match existing_body {
            Some(body) => ctx.pull_request(pr_number).get_body(Some(body)).await,
            None => ctx.pull_request(pr_number).get_body_error(503).await,
        }
        ctx.update_pull_request(pr_number, "Updated title", updated_body, branch)
            .await;
        let draft_dir = DraftFile::draft_dir().join(owner).join(repo);
        if draft_dir.exists() {
            let _ = fs::remove_dir_all(&draft_dir);
        }
        TestEnv {
            mock,
            owner: owner.to_string(),
            repo: repo.to_string(),
            draft_dir,
        }
    }

    async fn pull_request_payloads(
        env: &TestEnv,
        method: &str,
        path: &str,
    ) -> Vec<serde_json::Value> {
        env.mock
            .received_requests()
            .await
            .iter()
            .filter(|request| request.method.as_str() == method && request.url.path() == path)
            .map(|request| request.body_json::<serde_json::Value>())
            .collect::<serde_json::Result<Vec<_>>>()
            .expect("decode pull request payload")
    }

    async fn stack_requests(env: &TestEnv) -> Vec<(String, String, String, serde_json::Value)> {
        env.mock
            .received_requests()
            .await
            .iter()
            .filter(|request| {
                let path = request.url.path();
                path.contains("/stacks")
                    || (request.method.as_str() == "GET"
                        && path.contains("/pulls/")
                        && path
                            .rsplit('/')
                            .next()
                            .is_some_and(|number| number.parse::<u64>().is_ok()))
            })
            .map(|request| {
                let body = if request.method.as_str() == "POST" {
                    request
                        .body_json::<serde_json::Value>()
                        .expect("decode stack request body")
                } else {
                    serde_json::Value::Null
                };
                (
                    request.method.to_string(),
                    request.url.path().to_string(),
                    request.url.query().unwrap_or_default().to_string(),
                    body,
                )
            })
            .collect()
    }

    fn expected_stack_membership_get(
        repo: &str,
        pull_request: u64,
    ) -> (String, String, String, serde_json::Value) {
        (
            "GET".to_string(),
            format!("/repos/sample-owner/{repo}/pulls/{pull_request}"),
            String::new(),
            serde_json::Value::Null,
        )
    }

    fn expected_stack_post(
        path: String,
        body: serde_json::Value,
    ) -> (String, String, String, serde_json::Value) {
        ("POST".to_string(), path, String::new(), body)
    }

    async fn run_stack_submit(
        env: &TestEnv,
        branch: &str,
        base: Option<&str>,
    ) -> anyhow::Result<()> {
        run_stack_submit_with_options(env, branch, base, false).await
    }

    async fn run_stack_submit_with_options(
        env: &TestEnv,
        branch: &str,
        base: Option<&str>,
        no_stack: bool,
    ) -> anyhow::Result<()> {
        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            branch,
            "Add stack support",
            "Body content",
        );
        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: base.map(str::to_string),
            draft: false,
            no_review: false,
            no_stack,
        };
        let client = env.mock.client();

        run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            no_current_repo,
            &NoopPostSubmitProcessor,
            no_vibe_access,
        )
        .await
    }

    #[rstest]
    #[tokio::test]
    async fn submit_updates_existing_pr(#[with(true)] post_submit_spy: PostSubmitSpy) {
        let existing_body = indoc! {"
            Existing body

            <!-- pr-series:start -->
            ### Notification preferences

            Existing diagram
            <!-- pr-series:end -->
        "}
        .trim_end_matches('\n');
        let submitted_body = indoc! {"
            Updated body
            <!-- pr-series:start -->
            <!-- pr-series:end -->
            More updated body
        "}
        .trim_end_matches('\n');
        let updated_body = indoc! {"
            Updated body
            <!-- pr-series:start -->
            ### Notification preferences

            Existing diagram
            <!-- pr-series:end -->
            More updated body
        "};
        let env = setup_test_env_with_existing_pr(
            "owner",
            "repo_submit_update",
            "feature/existing-pr",
            42,
            ExistingPrBody::Text(existing_body),
            updated_body,
        )
        .await;

        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/existing-pr",
            "Updated title",
            submitted_body,
        );

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };

        let client = env.mock.client();
        let cached_config = Mutex::new(None);
        let write_config = |key: &str, value: &str| {
            *cached_config.lock().expect("cache spy mutex") =
                Some((key.to_string(), value.to_string()));
            Ok(())
        };
        let current_repo = || Some((env.owner.clone(), env.repo.clone()));
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            write_config,
            current_repo,
            &post_submit_spy,
            no_vibe_access,
        )
        .await;
        let update_payloads =
            pull_request_payloads(&env, "PATCH", "/repos/owner/repo_submit_update/pulls/42").await;

        assert_eq!(
            (
                result
                    .map(|()| cached_config.into_inner().expect("cache spy mutex"))
                    .map_err(|error| error.to_string()),
                update_payloads,
            ),
            (
                Ok(Some((
                    "branch.feature/existing-pr.pr-url".to_string(),
                    "https://github.com/owner/repo_submit_update/pull/42".to_string(),
                ))),
                vec![serde_json::json!({
                    "title": "Updated title",
                    "body": updated_body,
                })],
            ),
        );
    }

    #[rstest]
    #[case::malformed_markers(ExistingPrBody::MalformedMarkers, "repo_submit_preserve_malformed")]
    #[case::body_read_failure(ExistingPrBody::ReadFailure, "repo_submit_preserve_read_failure")]
    #[tokio::test]
    async fn submit_updates_with_draft_body_when_existing_section_cannot_be_preserved(
        #[case] existing_body: ExistingPrBody<'_>,
        #[case] repo: &str,
        #[with(true)] post_submit_spy: PostSubmitSpy,
    ) {
        let env = setup_test_env_with_existing_pr(
            "owner",
            repo,
            "feature/preserve-fallback",
            42,
            existing_body,
            "Updated body",
        )
        .await;
        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/preserve-fallback",
            "Updated title",
            "Updated body",
        );
        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };
        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            || Some((env.owner.clone(), env.repo.clone())),
            &post_submit_spy,
            no_vibe_access,
        )
        .await;
        let update_payloads =
            pull_request_payloads(&env, "PATCH", &format!("/repos/owner/{repo}/pulls/42")).await;

        assert_eq!(
            (result.map_err(|error| error.to_string()), update_payloads),
            (
                Ok(()),
                vec![serde_json::json!({
                    "title": "Updated title",
                    "body": "Updated body\n",
                })],
            ),
        );
    }

    #[rstest]
    #[case::success("stack-create", 201)]
    #[case::api_failure("stack-failure", 422)]
    #[tokio::test]
    async fn submit_handles_stack_create_results(#[case] repo: &str, #[case] status: u16) {
        let env = setup_test_env_without_pull_request_mocks("sample-owner", repo).await;
        let ctx = env.mock.repo(&env.owner, &env.repo);
        ctx.list_pull_requests_empty_for_branch("feature/top").await;
        ctx.list_open_pull_requests_with(17, "Base change", "Body", "main")
            .await;
        ctx.pull_request(18).create().await;
        ctx.stack(1).get_empty_for_pull_request(17).await;
        ctx.stack(1).create_with_status(&[17, 18], status).await;

        let result = run_stack_submit(&env, "feature/top", Some("main")).await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                stack_requests(&env).await,
            ),
            (
                Ok(()),
                vec![
                    expected_stack_membership_get(repo, 17),
                    expected_stack_post(
                        format!("/repos/sample-owner/{repo}/stacks"),
                        serde_json::json!({ "pull_requests": [17, 18] }),
                    ),
                ],
            ),
        );
    }

    #[rstest]
    #[case::success("stack-add", 201)]
    #[case::api_failure("stack-add-failure", 409)]
    #[tokio::test]
    async fn submit_handles_stack_add_results(#[case] repo: &str, #[case] status: u16) {
        let env = setup_test_env_without_pull_request_mocks("sample-owner", repo).await;
        let ctx = env.mock.repo(&env.owner, &env.repo);
        ctx.list_pull_requests_empty_for_branch("feature/top").await;
        ctx.list_open_pull_requests_with(17, "Base change", "Body", "main")
            .await;
        ctx.pull_request(18).create().await;
        ctx.stack(23).get_for_pull_request(17, 2, 2).await;
        ctx.stack(23).add_with_status(&[18], status).await;

        let result = run_stack_submit(&env, "feature/top", Some("main")).await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                stack_requests(&env).await,
            ),
            (
                Ok(()),
                vec![
                    expected_stack_membership_get(repo, 17),
                    expected_stack_post(
                        format!("/repos/sample-owner/{repo}/stacks/23/add"),
                        serde_json::json!({ "pull_requests": [18] }),
                    ),
                ],
            ),
        );
    }

    #[rstest]
    #[case::no_base("stack-skip-no-base", None)]
    #[case::no_open_base_pr("stack-skip-no-open-pr", Some("main"))]
    #[tokio::test]
    async fn submit_skips_stack_api_when_it_has_no_open_base_pr(
        #[case] repo: &str,
        #[case] base: Option<&str>,
    ) {
        let env = setup_test_env_without_pull_request_mocks("sample-owner", repo).await;
        let ctx = env.mock.repo(&env.owner, &env.repo);
        ctx.list_pull_requests_empty_for_branch("feature/top").await;
        if let Some(base) = base {
            ctx.list_open_pull_requests_empty_for_branch(base).await;
        }
        ctx.pull_request(18).create().await;

        let result = run_stack_submit(&env, "feature/top", base).await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                stack_requests(&env).await,
            ),
            (Ok(()), Vec::new()),
        );
    }

    #[tokio::test]
    async fn submit_does_not_stack_when_no_stack_is_enabled() {
        let env = setup_test_env_without_pull_request_mocks("sample-owner", "no-stack").await;
        let ctx = env.mock.repo(&env.owner, &env.repo);
        ctx.list_pull_requests_empty_for_branch("feature/top").await;
        ctx.list_open_pull_requests_with(17, "Base change", "Body", "renovate/example")
            .await;
        ctx.pull_request(18).create().await;

        let result =
            run_stack_submit_with_options(&env, "feature/top", Some("renovate/example"), true)
                .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                pull_request_payloads(&env, "POST", "/repos/sample-owner/no-stack/pulls").await,
                stack_requests(&env).await,
            ),
            (
                Ok(()),
                vec![serde_json::json!({
                    "title": "Add stack support",
                    "body": "Body content\n",
                    "head": "feature/top",
                    "base": "renovate/example",
                })],
                Vec::new(),
            ),
        );
    }

    #[tokio::test]
    async fn submit_does_not_stack_a_base_pr_from_another_repository() {
        let env = setup_test_env_without_pull_request_mocks("sample-owner", "stack-fork").await;
        let ctx = env.mock.repo(&env.owner, &env.repo);
        ctx.list_pull_requests_empty_for_branch("feature/top").await;
        ctx.list_open_pull_requests_with_head_repo(
            17,
            "Base change",
            "Body",
            "main",
            "sample-owner/another-repo",
        )
        .await;
        ctx.pull_request(18).create().await;

        let result = run_stack_submit(&env, "feature/top", Some("main")).await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                stack_requests(&env).await,
            ),
            (Ok(()), Vec::new()),
        );
    }

    #[tokio::test]
    async fn submit_leaves_new_pr_unstacked_when_base_pr_is_not_at_the_top() {
        let env = setup_test_env_without_pull_request_mocks("sample-owner", "stack-middle").await;
        let ctx = env.mock.repo(&env.owner, &env.repo);
        ctx.list_pull_requests_empty_for_branch("feature/top").await;
        ctx.list_open_pull_requests_with(17, "Base change", "Body", "main")
            .await;
        ctx.pull_request(18).create().await;
        ctx.stack(23).get_for_pull_request(17, 2, 3).await;

        let result = run_stack_submit(&env, "feature/top", Some("main")).await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                stack_requests(&env).await,
            ),
            (
                Ok(()),
                vec![expected_stack_membership_get("stack-middle", 17)],
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn submit_skips_existing_body_fetch_without_saved_plan(
        #[with(false)] post_submit_spy: PostSubmitSpy,
    ) {
        let env = setup_test_env_with_existing_pr(
            "owner",
            "repo_submit_without_plan",
            "feature/without-plan",
            42,
            ExistingPrBody::Text("Existing body"),
            "Updated body",
        )
        .await;
        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/without-plan",
            "Updated title",
            "Updated body",
        );
        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };
        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            || Some((env.owner.clone(), env.repo.clone())),
            &post_submit_spy,
            no_vibe_access,
        )
        .await;
        let requests = env
            .mock
            .received_requests()
            .await
            .iter()
            .map(|request| (request.method.to_string(), request.url.path().to_string()))
            .collect::<Vec<_>>();

        assert_eq!(
            (result.map_err(|error| error.to_string()), requests),
            (
                Ok(()),
                vec![
                    (
                        "GET".to_string(),
                        "/repos/owner/repo_submit_without_plan".to_string()
                    ),
                    (
                        "GET".to_string(),
                        "/repos/owner/repo_submit_without_plan/pulls".to_string(),
                    ),
                    (
                        "PATCH".to_string(),
                        "/repos/owner/repo_submit_without_plan/pulls/42".to_string(),
                    ),
                ],
            ),
        );
    }

    #[tokio::test]
    async fn submit_creates_pr_even_when_caching_pr_url_fails() {
        let env = setup_test_env("owner", "repo_submit_cache_url").await;
        let branch = "feature/cache.pr-url";
        let draft_path =
            create_approved_draft(&env.owner, &env.repo, branch, "Ready title", "Body content");
        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };

        let client = env.mock.client();
        let cached_config = Mutex::new(None);
        let write_config = |key: &str, value: &str| {
            *cached_config.lock().expect("cache spy mutex") =
                Some((key.to_string(), value.to_string()));
            anyhow::bail!("git config is unavailable")
        };
        let current_repo = || Some((env.owner.clone(), env.repo.clone()));
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            write_config,
            current_repo,
            &NoopPostSubmitProcessor,
            no_vibe_access,
        )
        .await;

        assert_eq!(
            result
                .map(|()| cached_config.into_inner().expect("cache spy mutex"))
                .map_err(|error| error.to_string()),
            Ok(Some((
                format!("branch.{branch}.pr-url"),
                format!("https://github.com/{}/{}/pull/1", env.owner, env.repo),
            ))),
        );
    }

    #[rstest]
    #[case::created_pr("sample-repository", None, 1, vec![(
        "sample-owner".to_string(),
        "sample-repository".to_string(),
        "Submit a change".to_string(),
        "feature/submitted-change".to_string(),
        None,
        false,
    )], Vec::new())]
    #[case::updated_pr("sample-repository-updated", Some(23), 23, Vec::new(), vec![23])]
    #[tokio::test]
    async fn submit_records_the_url_for_created_and_updated_prs(
        sessions_dir_with_cwd_pr_context: tempfile::TempDir,
        #[case] draft_repo: &str,
        #[case] existing_pr_number: Option<u64>,
        #[case] pull_request_number: u64,
        #[case] expected_create_calls: Vec<CreatePrCall>,
        #[case] expected_update_calls: Vec<u64>,
    ) {
        let _draft_directory = TestDraftDirectory::new("sample-owner", draft_repo);
        let branch = "feature/submitted-change";
        let draft_path = create_approved_draft(
            "sample-owner",
            draft_repo,
            branch,
            "Submit a change",
            "Body content",
        );
        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };
        let client = SubmitClientStub {
            existing_pr_number,
            ..SubmitClientStub::default()
        };
        let sessions_dir = sessions_dir_with_cwd_pr_context.path();
        let record_pr_url = |pr_url: &str| {
            record_created_pr_url_in(sessions_dir, "sample-session", pr_url)
                .expect("successful submit should record its pull request URL");
        };
        let submit_effects = SubmitEffects {
            post_submit_processor: &NoopPostSubmitProcessor,
            record_created_pr_url: record_pr_url,
        };
        let result = super::run_impl_with_recording(
            &args,
            &client,
            &noop_hook,
            noop_config,
            no_current_repo,
            &submit_effects,
            no_vibe_access,
        )
        .await;
        let session =
            crate::commands::agent::store::load_session_from(sessions_dir, "sample-session")
                .expect("session store should be readable")
                .expect("session should remain stored");
        let created_prs = client
            .create_calls
            .lock()
            .expect("submit client mutex")
            .clone();
        let updated_prs = client
            .update_calls
            .lock()
            .expect("submit client mutex")
            .clone();
        let pr_url =
            format!("https://github.com/sample-owner/sample-repository/pull/{pull_request_number}");

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                created_prs,
                updated_prs,
                (session.pr_context, session.created_pr_urls),
            ),
            (
                Ok(()),
                expected_create_calls,
                expected_update_calls,
                (
                    Some(SessionPrContext {
                        owner: "sample-owner".to_string(),
                        repo: "sample-repository".to_string(),
                        branch: "cwd-branch".to_string(),
                        checklist_item_id: None,
                    }),
                    vec![pr_url],
                ),
            ),
        );
    }

    #[tokio::test]
    async fn submit_with_filepath_skips_cache_for_an_unrelated_repo() {
        let env = setup_test_env("owner", "repo_submit_foreign_draft").await;
        let branch = "feature/foreign-draft";
        let draft_path =
            create_approved_draft(&env.owner, &env.repo, branch, "Ready title", "Body content");
        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };

        let client = env.mock.client();
        let cached_config = Mutex::new(None);
        let write_config = |key: &str, value: &str| {
            *cached_config.lock().expect("cache spy mutex") =
                Some((key.to_string(), value.to_string()));
            Ok(())
        };
        let unrelated_repo = || Some(("other-owner".to_string(), "other-repo".to_string()));
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            write_config,
            unrelated_repo,
            &NoopPostSubmitProcessor,
            no_vibe_access,
        )
        .await;

        assert_eq!(
            result
                .map(|()| cached_config.into_inner().expect("cache spy mutex"))
                .map_err(|error| error.to_string()),
            Ok(None),
        );
    }

    #[test]
    fn write_pr_url_config_uses_local_git_config() {
        let mut command = None;
        let result = write_pr_url_config_with(
            "branch.feature/example.pr-url",
            "https://github.com/example/project/pull/123",
            |path, args| {
                command = Some((path.to_path_buf(), args.map(|arg| arg.to_string()).to_vec()));
                Ok(String::new())
            },
        );

        assert_eq!(
            result.map(|()| command).map_err(|error| error.to_string()),
            Ok(Some((
                std::path::PathBuf::from("."),
                vec![
                    "config".to_string(),
                    "--local".to_string(),
                    "branch.feature/example.pr-url".to_string(),
                    "https://github.com/example/project/pull/123".to_string(),
                ],
            ))),
        );
    }

    #[tokio::test]
    async fn submit_invokes_pre_pr_submit_hook_with_env_for_new_pr() {
        let env = setup_test_env("owner", "repo_submit_hook_new").await;
        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/hook-new",
            "Hook title",
            "Body referencing #123",
        );

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: Some("master".to_string()),
            draft: false,
            no_review: false,
            no_stack: false,
        };

        let spy = HookSpy::default();
        let hook = |name: &str, env: &[(&str, &str)]| {
            spy.record(name, env);
            Ok(())
        };

        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &hook,
            noop_config,
            no_current_repo,
            &NoopPostSubmitProcessor,
            no_vibe_access,
        )
        .await;
        assert!(result.is_ok(), "submit failed: {:?}", result.err());

        let calls = spy.calls();
        assert_eq!(calls.len(), 1, "expected exactly one hook invocation");
        let call = &calls[0];
        assert_eq!(call.name, PRE_PR_SUBMIT_HOOK);

        assert_eq!(
            lookup(&call.env, EnvVars::pr_title_name()),
            Some("Hook title")
        );
        assert_eq!(lookup(&call.env, EnvVars::pr_owner_name()), Some("owner"));
        assert_eq!(
            lookup(&call.env, EnvVars::pr_repo_name()),
            Some("repo_submit_hook_new")
        );
        assert_eq!(
            lookup(&call.env, EnvVars::pr_head_name()),
            Some("feature/hook-new")
        );
        assert_eq!(lookup(&call.env, EnvVars::pr_base_name()), Some("master"));
        assert_eq!(lookup(&call.env, EnvVars::pr_is_update_name()), Some("0"));
        assert_eq!(lookup(&call.env, EnvVars::pr_number_name()), Some(""));

        let body_contents = call
            .body_file_contents
            .as_deref()
            .expect("body file readable inside hook");
        assert_eq!(body_contents, "Body referencing #123\n");
    }

    #[tokio::test]
    async fn submit_invokes_pre_pr_submit_hook_with_pr_number_for_update() {
        let env = setup_test_env_with_existing_pr(
            "owner",
            "repo_submit_hook_update",
            "feature/hook-update",
            42,
            ExistingPrBody::Text("Old body"),
            "Updated body",
        )
        .await;
        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/hook-update",
            "Updated title",
            "Updated body",
        );

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };

        let spy = HookSpy::default();
        let hook = |name: &str, env: &[(&str, &str)]| {
            spy.record(name, env);
            Ok(())
        };

        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &hook,
            noop_config,
            no_current_repo,
            &NoopPostSubmitProcessor,
            no_vibe_access,
        )
        .await;
        assert!(result.is_ok(), "submit failed: {:?}", result.err());

        let calls = spy.calls();
        assert_eq!(calls.len(), 1);
        let env = &calls[0].env;
        assert_eq!(lookup(env, EnvVars::pr_is_update_name()), Some("1"));
        assert_eq!(lookup(env, EnvVars::pr_number_name()), Some("42"));
        assert_eq!(lookup(env, EnvVars::pr_base_name()), Some(""));
    }

    #[tokio::test]
    async fn submit_aborts_when_pre_pr_submit_hook_fails() {
        let env = setup_test_env("owner", "repo_submit_hook_block").await;
        let draft_path =
            create_approved_draft(&env.owner, &env.repo, "feature/hook-block", "Title", "Body");

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
            no_stack: false,
        };

        let hook = |_name: &str, _env: &[(&str, &str)]| -> anyhow::Result<()> {
            anyhow::bail!("forbidden cross-org link")
        };

        let client = env.mock.client();
        let err = run_impl(
            &args,
            &client,
            &hook,
            noop_config,
            no_current_repo,
            &NoopPostSubmitProcessor,
            no_vibe_access,
        )
        .await
        .expect_err("hook failure must abort submission");
        assert_eq!(err.to_string(), "forbidden cross-org link");
    }
}
