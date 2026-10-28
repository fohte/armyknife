use clap::Args;
use std::path::{Path, PathBuf};

use super::common::{DraftFile, PrDraftError, RepoInfo, contains_japanese, repo_allows_japanese};
use super::hooks::{HookContext, HookRunner, PRE_PR_SUBMIT_HOOK, default_runner, run_pr_hook};
use crate::commands::agent::{ensure_repo_vibe_access, is_vibe_enabled};
use crate::commands::ai::pr_series_aftercare::{
    SubmitSyncFailure, SubmitSyncOutcome, SubmitSyncStage, link_and_sync_after_submit,
};
use crate::infra::git::cmd::{run_git, run_git_optional};
use crate::infra::github::{
    CreatePrParams, GitHubClient, PrClient, PrState, RepoClient, UpdatePrParams,
};
use crate::shared::config::{Config, load_config};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SubmitArgs {
    /// Path to the draft file (auto-detected if not specified)
    pub filepath: Option<PathBuf>,

    /// Base branch for the PR
    #[arg(long)]
    pub base: Option<String>,

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct TqBranchMetadata {
    task_id: String,
    checklist_item_id: String,
}

trait PostSubmitProcessor {
    fn metadata_for_branch(&self, branch: &str) -> Option<TqBranchMetadata>;

    async fn link_and_sync(
        &self,
        metadata: &TqBranchMetadata,
        target: &PrTarget,
        pr_url: &str,
    ) -> std::result::Result<SubmitSyncOutcome, SubmitSyncFailure>;
}

struct PrSeriesPostSubmitProcessor<'a> {
    github: &'a GitHubClient,
}

impl PostSubmitProcessor for PrSeriesPostSubmitProcessor<'_> {
    fn metadata_for_branch(&self, branch: &str) -> Option<TqBranchMetadata> {
        read_current_branch_tq_metadata(branch)
    }

    async fn link_and_sync(
        &self,
        metadata: &TqBranchMetadata,
        target: &PrTarget,
        pr_url: &str,
    ) -> std::result::Result<SubmitSyncOutcome, SubmitSyncFailure> {
        link_and_sync_after_submit(
            self.github,
            &metadata.task_id,
            &metadata.checklist_item_id,
            &target.owner,
            &target.repo,
            pr_url,
        )
        .await
    }
}

pub async fn run(args: &SubmitArgs) -> anyhow::Result<()> {
    let client = GitHubClient::get()?;
    let post_submit_processor = PrSeriesPostSubmitProcessor { github: client };
    run_impl_with_post_submit(
        args,
        client,
        default_runner(),
        write_pr_url_config,
        read_current_repo_identity,
        &post_submit_processor,
        |owner, repo| ensure_no_review_access(is_vibe_enabled, load_config, owner, repo),
    )
    .await
}

fn ensure_no_review_access(
    vibe_enabled: impl FnOnce() -> anyhow::Result<bool>,
    load_config: impl FnOnce() -> anyhow::Result<Config>,
    owner: &str,
    repo: &str,
) -> anyhow::Result<()> {
    if !vibe_enabled()? {
        anyhow::bail!("The --no-review option is only available when vibe mode is enabled.");
    }
    ensure_repo_vibe_access(&load_config()?, owner, repo)
}

#[cfg(test)]
async fn run_impl(
    args: &SubmitArgs,
    gh_client: &(impl PrClient + RepoClient),
    run_hook: HookRunner<'_>,
    write_config: impl Fn(&str, &str) -> anyhow::Result<()>,
    current_repo_identity: impl FnOnce() -> Option<(String, String)>,
    vibe_access_check: impl FnOnce(&str, &str) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    run_impl_with_post_submit(
        args,
        gh_client,
        run_hook,
        write_config,
        current_repo_identity,
        &NoopPostSubmitProcessor,
        vibe_access_check,
    )
    .await
}

async fn run_impl_with_post_submit(
    args: &SubmitArgs,
    gh_client: &(impl PrClient + RepoClient),
    run_hook: HookRunner<'_>,
    write_config: impl Fn(&str, &str) -> anyhow::Result<()>,
    current_repo_identity: impl FnOnce() -> Option<(String, String)>,
    post_submit_processor: &impl PostSubmitProcessor,
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

    // Check for lock (editor still open)
    if DraftFile::lock_path(&draft_path).exists() {
        anyhow::bail!("Please close the editor before submitting the PR.");
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

    // For public repos without Japanese language config, check for Japanese characters
    let allows_japanese = is_private || repo_allows_japanese(&target.owner, &target.repo);
    if !allows_japanese {
        if contains_japanese(&draft.frontmatter.title) {
            return Err(PrDraftError::JapaneseInTitle.into());
        }
        if contains_japanese(&draft.body) {
            return Err(PrDraftError::JapaneseInBody.into());
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
            gh_client
                .update_pull_request(UpdatePrParams {
                    owner: target.owner.clone(),
                    repo: target.repo.clone(),
                    number,
                    title: draft.frontmatter.title.clone(),
                    body: draft.body.clone(),
                })
                .await?
        }
        None => {
            // Create new PR
            gh_client
                .create_pull_request(CreatePrParams {
                    owner: target.owner.clone(),
                    repo: target.repo.clone(),
                    title: draft.frontmatter.title.clone(),
                    body: draft.body.clone(),
                    head: target.branch.clone(),
                    base: args.base.clone(),
                    draft: args.draft,
                })
                .await?
        }
    };

    let target_is_current_repo = if args.filepath.is_some() {
        current_repo_identity()
            .is_some_and(|(owner, repo)| owner == target.owner && repo == target.repo)
    } else {
        true
    };
    let tq_metadata = if target_is_current_repo {
        post_submit_processor.metadata_for_branch(&target.branch)
    } else {
        None
    };
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
        && let Err(failure) = post_submit_processor
            .link_and_sync(&metadata, &target, &pr_url)
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

#[cfg(test)]
struct NoopPostSubmitProcessor;

#[cfg(test)]
impl PostSubmitProcessor for NoopPostSubmitProcessor {
    fn metadata_for_branch(&self, _branch: &str) -> Option<TqBranchMetadata> {
        None
    }

    async fn link_and_sync(
        &self,
        _metadata: &TqBranchMetadata,
        _target: &PrTarget,
        _pr_url: &str,
    ) -> std::result::Result<SubmitSyncOutcome, SubmitSyncFailure> {
        Ok(SubmitSyncOutcome::NoPlan)
    }
}

fn read_current_branch_tq_metadata(branch: &str) -> Option<TqBranchMetadata> {
    tq_branch_metadata(
        read_branch_config_value(branch, "x-tq-task-id"),
        read_branch_config_value(branch, "x-tq-checklist-item-id"),
    )
}

fn tq_branch_metadata(
    task_id: Option<String>,
    checklist_item_id: Option<String>,
) -> Option<TqBranchMetadata> {
    let task_id = task_id.filter(|value| !value.trim().is_empty())?;
    let checklist_item_id = checklist_item_id.filter(|value| !value.trim().is_empty())?;
    Some(TqBranchMetadata {
        task_id,
        checklist_item_id,
    })
}

fn read_branch_config_value(branch: &str, name: &str) -> Option<String> {
    let key = format!("branch.{branch}.{name}");
    run_git_optional(Path::new("."), ["config", "--local", "--get", &key])
        .filter(|value| !value.trim().is_empty())
}

fn format_post_submit_failure(
    failure: &SubmitSyncFailure,
    metadata: &TqBranchMetadata,
    pr_url: &str,
) -> String {
    match failure.stage {
        SubmitSyncStage::Prepare => format!(
            "PR submit completed, but PR series setup failed: {}\nAfter resolving the error, run `tq checklist item update {} --github {}` and then `a ai pr-series sync {}`.",
            failure.error, metadata.checklist_item_id, pr_url, metadata.task_id
        ),
        SubmitSyncStage::Link => format!(
            "PR submit completed, but its tq checklist link failed: {}\nAfter resolving the error, run `tq checklist item update {} --github {}` and then `a ai pr-series sync {}`.",
            failure.error, metadata.checklist_item_id, pr_url, metadata.task_id
        ),
        SubmitSyncStage::Sync => format!(
            "PR submit completed and linked, but PR series synchronization failed: {}\nAfter resolving the listed errors, run `a ai pr-series sync {}`.",
            failure.error, metadata.task_id
        ),
    }
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
    use super::*;
    use crate::infra::github::GitHubMockServer;
    use crate::shared::config::Config;
    use crate::shared::env_var::EnvVars;
    use clap::Parser;
    use indoc::indoc;
    use rstest::rstest;
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

    fn no_current_repo() -> Option<(String, String)> {
        None
    }

    fn no_vibe_access(_owner: &str, _repo: &str) -> anyhow::Result<()> {
        anyhow::bail!("The --no-review option is only available when vibe mode is enabled.")
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
    }

    impl PostSubmitProcessor for PostSubmitSpy {
        fn metadata_for_branch(&self, _branch: &str) -> Option<TqBranchMetadata> {
            self.metadata.clone()
        }

        async fn link_and_sync(
            &self,
            metadata: &TqBranchMetadata,
            target: &PrTarget,
            pr_url: &str,
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

    async fn setup_test_env(owner: &str, repo: &str) -> TestEnv {
        crate::shared::testing::init_approval_dir();
        let mock = GitHubMockServer::start().await;
        let ctx = mock.repo(owner, repo);
        ctx.repo_info().private(true).get().await;
        ctx.list_pull_requests_empty().await;
        ctx.pull_request(1).create().await;
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

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
        };

        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            no_current_repo,
            no_vibe_access,
        )
        .await;

        assert!(
            result.is_ok(),
            "submit should work with GitHubMockServer: {:?}",
            result.err()
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
        };

        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            no_current_repo,
            no_vibe_access,
        )
        .await;

        assert!(
            result.is_ok(),
            "submit should pass the draft branch when --filepath is used: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn post_submit_sync_failure_does_not_undo_pr_creation() {
        let env = setup_test_env("acme", "notify-api").await;
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
        };
        let metadata = TqBranchMetadata {
            task_id: "sample-task-id".to_string(),
            checklist_item_id: "sample-leaf-id".to_string(),
        };
        let processor = PostSubmitSpy {
            failure_stage: Some(SubmitSyncStage::Link),
            metadata: Some(metadata.clone()),
            ..PostSubmitSpy::default()
        };
        let client = env.mock.client();

        let result = run_impl_with_post_submit(
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

        assert_eq!(
            (
                result,
                processor
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
                    "notify-api".to_string(),
                    "https://github.com/acme/notify-api/pull/1".to_string(),
                )],
            ),
        );
    }

    #[rstest]
    #[case::prepare(
        SubmitSyncStage::Prepare,
        "PR submit completed, but PR series setup failed: sample synchronization failure\nAfter resolving the error, run `tq checklist item update sample-leaf-id --github https://github.com/acme/notify-api/pull/17` and then `a ai pr-series sync sample-task-id`."
    )]
    #[case::link(
        SubmitSyncStage::Link,
        "PR submit completed, but its tq checklist link failed: sample synchronization failure\nAfter resolving the error, run `tq checklist item update sample-leaf-id --github https://github.com/acme/notify-api/pull/17` and then `a ai pr-series sync sample-task-id`."
    )]
    #[case::sync(
        SubmitSyncStage::Sync,
        "PR submit completed and linked, but PR series synchronization failed: sample synchronization failure\nAfter resolving the listed errors, run `a ai pr-series sync sample-task-id`."
    )]
    fn formats_post_submit_recovery_instructions(
        #[case] stage: SubmitSyncStage,
        #[case] expected: &str,
    ) {
        let failure = SubmitSyncFailure {
            stage,
            error: anyhow::anyhow!("sample synchronization failure"),
        };
        let metadata = TqBranchMetadata {
            task_id: "sample-task-id".to_string(),
            checklist_item_id: "sample-leaf-id".to_string(),
        };

        assert_eq!(
            format_post_submit_failure(
                &failure,
                &metadata,
                "https://github.com/acme/notify-api/pull/17",
            ),
            expected,
        );
    }

    #[rstest]
    #[case::both_ids(
        Some("sample-task-id".to_string()),
        Some("sample-leaf-id".to_string()),
        Some(TqBranchMetadata {
            task_id: "sample-task-id".to_string(),
            checklist_item_id: "sample-leaf-id".to_string(),
        })
    )]
    #[case::missing_task_id(None, Some("sample-leaf-id".to_string()), None)]
    #[case::missing_leaf_id(Some("sample-task-id".to_string()), None, None)]
    fn requires_both_branch_tq_ids(
        #[case] task_id: Option<String>,
        #[case] checklist_item_id: Option<String>,
        #[case] expected: Option<TqBranchMetadata>,
    ) {
        assert_eq!(tq_branch_metadata(task_id, checklist_item_id), expected);
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
        };

        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            no_current_repo,
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
        };

        let client = env.mock.client();
        let result = run_impl(
            &args,
            &client,
            &noop_hook,
            noop_config,
            no_current_repo,
            no_vibe_access,
        )
        .await;

        let err = result.expect_err("submit should fail with empty title");
        assert!(matches!(
            err.downcast_ref::<PrDraftError>(),
            Some(PrDraftError::EmptyTitle)
        ));
    }

    async fn setup_test_env_with_existing_pr(
        owner: &str,
        repo: &str,
        branch: &str,
        pr_number: u64,
    ) -> TestEnv {
        crate::shared::testing::init_approval_dir();
        let mock = GitHubMockServer::start().await;
        let ctx = mock.repo(owner, repo);
        ctx.repo_info().private(true).get().await;
        ctx.list_pull_requests_with(pr_number, "Old title", "Old body", branch)
            .await;
        ctx.update_pull_request(pr_number, "Updated title", "Updated body", branch)
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

    #[tokio::test]
    async fn submit_updates_existing_pr() {
        let env = setup_test_env_with_existing_pr(
            "owner",
            "repo_submit_update",
            "feature/existing-pr",
            42,
        )
        .await;

        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/existing-pr",
            "Updated title",
            "Updated body",
        );

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            no_review: false,
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
            no_vibe_access,
        )
        .await;

        assert_eq!(
            result
                .map(|()| cached_config.into_inner().expect("cache spy mutex"))
                .map_err(|error| error.to_string()),
            Ok(Some((
                "branch.feature/existing-pr.pr-url".to_string(),
                "https://github.com/owner/repo_submit_update/pull/42".to_string(),
            ))),
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
            no_vibe_access,
        )
        .await
        .expect_err("hook failure must abort submission");
        assert_eq!(err.to_string(), "forbidden cross-org link");
    }
}
