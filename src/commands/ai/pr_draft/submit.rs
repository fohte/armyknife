use clap::Args;
use std::io::Write;
use std::path::PathBuf;

use super::common::{DraftFile, PrDraftError, RepoInfo, contains_japanese, repo_allows_japanese};
use crate::infra::github::{
    CreatePrParams, GitHubClient, PrClient, PrState, RepoClient, UpdatePrParams,
};
use crate::shared::env_var::EnvVars;
use crate::shared::hooks;

/// Hook name fired right before a PR is created or updated on GitHub.
/// A non-zero exit aborts submission, so it doubles as a user-defined lint.
const PRE_PR_SUBMIT_HOOK: &str = "pre-pr-submit";

/// Function signature for executing a hook script. Production code uses
/// [`hooks::run_hook`]; tests inject a closure to verify invocation without
/// mutating the real `XDG_CONFIG_HOME`.
type HookRunner<'a> = &'a (dyn Fn(&str, &[(&str, &str)]) -> anyhow::Result<()> + Send + Sync);

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
    run_impl(args, client, &hooks::run_hook).await
}

async fn run_impl(
    args: &SubmitArgs,
    gh_client: &(impl PrClient + RepoClient),
    run_hook: HookRunner<'_>,
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

    // Verify approval
    draft.verify_approval()?;

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
    // The temp body file is dropped (and thus deleted) as soon as the hook
    // returns, so hook scripts must not rely on it persisting afterwards.
    drop(run_pre_submit_hook(
        &draft,
        &target,
        args,
        update_target,
        run_hook,
    )?);

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

    println!("{pr_url}");

    // Cleanup
    draft.cleanup()?;

    // Open PR in browser
    gh_client.open_in_browser(&pr_url);

    Ok(())
}

/// Materialize the PR body as a temp file and invoke `pre-pr-submit`.
///
/// The body is written to disk rather than passed via env so that hook scripts
/// can grep/match it without worrying about argv/env size limits and so that
/// embedded newlines round-trip exactly. The returned `NamedTempFile` keeps
/// the file alive for the hook's lifetime; dropping it cleans up the path.
fn run_pre_submit_hook(
    draft: &DraftFile,
    target: &PrTarget,
    args: &SubmitArgs,
    update_number: Option<u64>,
    run_hook: HookRunner<'_>,
) -> anyhow::Result<tempfile::NamedTempFile> {
    let mut body_file = tempfile::Builder::new()
        .prefix("armyknife-pr-body-")
        .suffix(".md")
        .tempfile()?;
    body_file.write_all(draft.body.as_bytes())?;
    body_file.flush()?;

    let body_path = body_file
        .path()
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("PR body temp file path is not valid UTF-8"))?
        .to_owned();
    let pr_number = update_number.map(|n| n.to_string()).unwrap_or_default();
    let is_update = if update_number.is_some() { "1" } else { "0" };
    let base = args.base.as_deref().unwrap_or("");

    run_hook(
        PRE_PR_SUBMIT_HOOK,
        &[
            (EnvVars::pr_title_name(), draft.frontmatter.title.as_str()),
            (EnvVars::pr_body_file_name(), body_path.as_str()),
            (EnvVars::pr_owner_name(), target.owner.as_str()),
            (EnvVars::pr_repo_name(), target.repo.as_str()),
            (EnvVars::pr_head_name(), target.branch.as_str()),
            (EnvVars::pr_base_name(), base),
            (EnvVars::pr_number_name(), pr_number.as_str()),
            (EnvVars::pr_is_update_name(), is_update),
        ],
    )?;

    Ok(body_file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::github::GitHubMockServer;
    use indoc::indoc;
    use std::fs;
    use std::sync::Mutex;

    /// Hook stub that always succeeds. Use when the test does not care about
    /// hook invocation.
    fn noop_hook(_name: &str, _env: &[(&str, &str)]) -> anyhow::Result<()> {
        Ok(())
    }

    /// Captures every hook invocation so tests can assert on hook name, env,
    /// and the contents of any temp file referenced by env. The body file is
    /// read **inside** the hook closure because `run_pre_submit_hook` deletes
    /// the temp file as soon as the hook returns.
    #[derive(Default)]
    struct HookSpy {
        calls: Mutex<Vec<HookCall>>,
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
                  submit: true
                ---
                {body}
            "#},
            title = title,
            body = body
        );
        fs::write(&draft_path, content).expect("write draft");
        DraftFile::from_path(draft_path.clone())
            .expect("draft file")
            .save_approval()
            .expect("save approval");
        draft_path
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
        };

        let client = env.mock.client();
        let result = run_impl(&args, &client, &noop_hook).await;

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
        };

        let client = env.mock.client();
        let result = run_impl(&args, &client, &noop_hook).await;

        assert!(
            result.is_ok(),
            "submit should pass the draft branch when --filepath is used: {:?}",
            result.err()
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
        };

        let client = env.mock.client();
        let result = run_impl(&args, &client, &noop_hook).await;

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
        };

        let client = env.mock.client();
        let result = run_impl(&args, &client, &noop_hook).await;

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
        };

        let client = env.mock.client();
        let result = run_impl(&args, &client, &noop_hook).await;

        assert!(
            result.is_ok(),
            "submit should update existing PR: {:?}",
            result.err()
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
        };

        let spy = HookSpy::default();
        let hook = |name: &str, env: &[(&str, &str)]| {
            spy.record(name, env);
            Ok(())
        };

        let client = env.mock.client();
        let result = run_impl(&args, &client, &hook).await;
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
        };

        let spy = HookSpy::default();
        let hook = |name: &str, env: &[(&str, &str)]| {
            spy.record(name, env);
            Ok(())
        };

        let client = env.mock.client();
        let result = run_impl(&args, &client, &hook).await;
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
        };

        let hook = |_name: &str, _env: &[(&str, &str)]| -> anyhow::Result<()> {
            anyhow::bail!("forbidden cross-org link")
        };

        let client = env.mock.client();
        let err = run_impl(&args, &client, &hook)
            .await
            .expect_err("hook failure must abort submission");
        assert_eq!(err.to_string(), "forbidden cross-org link");
    }
}
