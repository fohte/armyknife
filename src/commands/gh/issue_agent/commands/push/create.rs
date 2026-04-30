//! Create new issue from local directory.

use std::path::{Path, PathBuf};

use super::PushArgs;
use super::changeset::{ChangeSet, DetectOptions, LocalState, RemoteState};
use crate::commands::gh::issue_agent::commands::common;
use crate::commands::gh::issue_agent::models::{
    EditableIssueFields, Issue, IssueFrontmatter, IssueMetadata, NewIssue,
};
use crate::commands::gh::issue_agent::storage::IssueStorage;
use crate::infra::git::parse_repo;
use crate::infra::github::GitHubClient;

/// Run create for new issue.
pub async fn run_create(args: &PushArgs, path: PathBuf) -> anyhow::Result<()> {
    let client = GitHubClient::get()?;
    run_create_with_client(args, path, client).await
}

/// Run create with an injected client (for testing).
pub async fn run_create_with_client(
    args: &PushArgs,
    path: PathBuf,
    client: impl std::ops::Deref<Target = GitHubClient>,
) -> anyhow::Result<()> {
    let client = &*client;
    // Get repo from path or -R option
    let repo = get_repo_from_path_or_arg(&path, &args.repo)?;
    let (owner, repo_name) = parse_repo(&repo)?;

    // Read and parse issue.md
    let issue_md_path = path.join("issue.md");
    if !issue_md_path.exists() {
        anyhow::bail!(
            "issue.md not found in {}. Create it with frontmatter and content.",
            path.display()
        );
    }

    let content = std::fs::read_to_string(&issue_md_path)?;
    let new_issue =
        NewIssue::parse(&content).map_err(|e| anyhow::anyhow!("Failed to parse issue.md: {e}"))?;

    // Display what will be created
    display_new_issue(&new_issue);

    if args.dry_run {
        println!();
        println!("[dry-run] Would create issue. Run without --dry-run to create.");
        return Ok(());
    }

    // Create issue on GitHub
    println!();
    println!("Creating issue on GitHub...");

    let created = client
        .create_issue(
            &owner,
            &repo_name,
            new_issue.title(),
            &new_issue.body,
            new_issue.labels(),
            new_issue.assignees(),
        )
        .await?;

    let issue_number = u64::try_from(created.number)
        .map_err(|_| anyhow::anyhow!("Invalid issue number returned: {}", created.number))?;

    // Reuse the edit path's ChangeSet::detect → apply pipeline to reconcile
    // any field the create API does not accept (currently parent/sub-issue
    // refs). Title / body / labels / assignees were sent via `create_issue`
    // above, so detect finds no diff for them and apply skips their update
    // calls. New fields added to `EditableIssueFields` automatically flow
    // through this same pipeline.
    let local_metadata = build_local_metadata(&new_issue, &created);
    let local_state = LocalState {
        metadata: &local_metadata,
        body: &new_issue.body,
        comments: &[],
    };
    let remote_state = RemoteState {
        issue: &created,
        comments: &[],
    };
    let detect_options = DetectOptions {
        // No comment work happens on create, so these flags are placeholders.
        current_user: "",
        edit_others: false,
        allow_delete: false,
    };
    let changeset = ChangeSet::detect(&local_state, &remote_state, &detect_options)?;

    // Rename directory from new/ to {issue_number}/.
    let new_dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine parent directory"))?
        .join(issue_number.to_string());

    if new_dir.exists() {
        anyhow::bail!(
            "Issue #{issue_number} created on GitHub, but directory '{}' already exists locally. \
             Remove or rename it, then run 'pull {issue_number}' to fetch the issue.",
            new_dir.display()
        );
    }

    if let Err(e) = std::fs::rename(&path, &new_dir) {
        anyhow::bail!(
            "Issue #{issue_number} created on GitHub, but failed to rename local directory: {e}. \
             Run 'pull {issue_number}' to fetch it locally."
        );
    }

    let storage = IssueStorage::from_dir(&new_dir);
    if changeset.has_changes() {
        changeset
            .apply(client, &owner, &repo_name, issue_number, &storage, &created)
            .await?;
    }

    // Re-fetch when apply established new parent/sub links so the saved
    // frontmatter mirrors the post-link remote state.
    let final_issue = if changeset.has_changes() {
        common::fetch_issue_with_sub_issues(client, &owner, &repo_name, issue_number).await?
    } else {
        created
    };

    let frontmatter = IssueFrontmatter::from_issue(&final_issue);
    if let Err(e) = storage.save_issue(&frontmatter, &new_issue.body) {
        // Directory was renamed successfully, so inform user about partial state
        anyhow::bail!(
            "Issue #{issue_number} created and directory renamed, but failed to save metadata: {e}. \
             Run 'pull --force {issue_number}' to refresh local state."
        );
    }

    // Success message
    println!();
    println!("Done! Created issue #{issue_number}");
    println!();
    println!("Local files moved to: {}/", new_dir.display());
    println!(
        "View on GitHub: https://github.com/{owner}/{repo_name}/issues/{issue_number}",
        owner = owner,
        repo_name = repo_name,
        issue_number = issue_number
    );

    Ok(())
}

/// Build the local-side `IssueMetadata` used by `ChangeSet::detect`.
///
/// Editable fields come from the user's `NewIssue`, server-managed fields
/// (number, state, timestamps, author) come from the freshly created remote
/// issue. The result represents "what the user wants the issue to be" so
/// that diffing against the remote surfaces only the fields the create API
/// could not handle.
fn build_local_metadata(new_issue: &NewIssue, created: &Issue) -> IssueMetadata {
    let EditableIssueFields {
        title,
        labels,
        assignees,
        milestone,
        parent_issue,
        sub_issues,
    } = new_issue.frontmatter.fields.clone();
    IssueMetadata {
        number: created.number,
        title,
        state: created.state.clone(),
        labels,
        assignees,
        milestone,
        author: created
            .author
            .as_ref()
            .map(|a| a.login.clone())
            .unwrap_or_default(),
        created_at: created.created_at.to_rfc3339(),
        updated_at: created.updated_at.to_rfc3339(),
        last_edited_at: created.last_edited_at.map(|dt| dt.to_rfc3339()),
        parent_issue,
        sub_issues,
    }
}

/// Get repository from path structure or -R argument.
fn get_repo_from_path_or_arg(path: &Path, repo_arg: &Option<String>) -> anyhow::Result<String> {
    if let Some(repo) = repo_arg {
        return Ok(repo.clone());
    }

    // Try to extract owner/repo from path structure:
    // Expected: .../{owner}/{repo}/new/
    if path.file_name().is_some_and(|name| name == "new")
        && let Some(repo_dir) = path.parent()
        && let (Some(repo_name), Some(owner_name)) = (
            repo_dir.file_name(),
            repo_dir.parent().and_then(|p| p.file_name()),
        )
    {
        let repo = repo_name.to_string_lossy();
        let owner = owner_name.to_string_lossy();
        return Ok(format!("{}/{}", owner, repo));
    }

    anyhow::bail!(
        "Cannot determine repository from path '{}'. Use -R owner/repo to specify.",
        path.display()
    )
}

/// Display the new issue that will be created.
fn display_new_issue(issue: &NewIssue) {
    println!("=== New Issue ===");
    println!();
    println!("Title: {}", issue.title());
    println!();

    let has_metadata = !issue.labels().is_empty()
        || !issue.assignees().is_empty()
        || issue.parent_issue().is_some()
        || !issue.sub_issues().is_empty();

    if !issue.labels().is_empty() {
        println!("Labels: {}", issue.labels().join(", "));
    }
    if !issue.assignees().is_empty() {
        println!("Assignees: {}", issue.assignees().join(", "));
    }
    if let Some(parent) = issue.parent_issue() {
        println!("Parent issue: {}", parent);
    }
    if !issue.sub_issues().is_empty() {
        println!("Sub-issues: {}", issue.sub_issues().join(", "));
    }

    if has_metadata {
        println!();
    }

    println!("Body:");
    println!("---");
    if issue.body.is_empty() {
        println!("(empty)");
    } else {
        println!("{}", issue.body);
    }
    println!("---");
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;
    use tempfile::TempDir;

    mod run_create_tests {
        use super::*;
        use crate::infra::github::GitHubMockServer;

        fn make_args(repo: Option<String>, dry_run: bool) -> PushArgs {
            PushArgs {
                target: String::new(), // not used in run_create_with_client
                repo,
                dry_run,
                force: false,
                edit_others: false,
                allow_delete: false,
            }
        }

        fn setup_new_issue_dir(base: &Path) -> PathBuf {
            let new_dir = base.join("owner").join("repo").join("new");
            std::fs::create_dir_all(&new_dir).unwrap();
            new_dir
        }

        fn write_issue_md(dir: &Path, content: &str) {
            std::fs::write(dir.join("issue.md"), content).unwrap();
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_success() {
            let temp = TempDir::new().unwrap();
            let new_dir = setup_new_issue_dir(temp.path());
            write_issue_md(
                &new_dir,
                indoc! {"
                    ---
                    title: Test Issue
                    labels: [bug]
                    assignees: [testuser]
                    ---

                    This is the body.
                "},
            );

            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo")
                .issue(42)
                .title("Test Issue")
                .body("This is the body.\n")
                .labels(vec!["bug"])
                .create()
                .await;

            let client = mock.client();
            let args = make_args(None, false);
            let result = run_create_with_client(&args, new_dir.clone(), &client).await;

            assert!(result.is_ok());

            // Verify directory was renamed
            let renamed_dir = temp.path().join("owner").join("repo").join("42");
            assert!(renamed_dir.exists());
            assert!(!new_dir.exists());

            // Verify issue.md with frontmatter was saved
            let issue_md = std::fs::read_to_string(renamed_dir.join("issue.md")).unwrap();
            assert_eq!(
                issue_md,
                indoc! {"
                    ---
                    title: Test Issue
                    labels:
                    - bug
                    assignees: []
                    milestone: null
                    submit: false
                    readonly:
                      number: 42
                      state: OPEN
                      author: testuser
                      createdAt: 2024-01-01T00:00:00+00:00
                      updatedAt: 2024-01-02T00:00:00+00:00
                    ---

                    This is the body.
                "}
            );
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_dry_run() {
            let temp = TempDir::new().unwrap();
            let new_dir = setup_new_issue_dir(temp.path());
            write_issue_md(
                &new_dir,
                indoc! {"
                    ---
                    title: Dry Run Test
                    ---

                    Body content.
                "},
            );

            let mock = GitHubMockServer::start().await;
            // No mock setup needed - API should not be called in dry run

            let client = mock.client();
            let args = make_args(None, true);
            let result = run_create_with_client(&args, new_dir.clone(), &client).await;

            assert!(result.is_ok());

            // Verify directory was NOT renamed
            assert!(new_dir.exists());
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_missing_issue_md() {
            let temp = TempDir::new().unwrap();
            let new_dir = setup_new_issue_dir(temp.path());
            // Don't create issue.md

            let mock = GitHubMockServer::start().await;
            let client = mock.client();
            let args = make_args(None, false);
            let result = run_create_with_client(&args, new_dir, &client).await;

            let err = result.unwrap_err();
            let err_msg = err.to_string();
            assert!(
                err_msg.starts_with("issue.md not found in "),
                "expected error to start with 'issue.md not found in ', got: {}",
                err_msg
            );
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_invalid_format() {
            let temp = TempDir::new().unwrap();
            let new_dir = setup_new_issue_dir(temp.path());
            write_issue_md(&new_dir, "No title here, just body text.");

            let mock = GitHubMockServer::start().await;
            let client = mock.client();
            let args = make_args(None, false);
            let result = run_create_with_client(&args, new_dir, &client).await;

            let err = result.unwrap_err();
            let err_msg = err.to_string();
            assert!(
                err_msg.starts_with("Failed to parse issue.md: "),
                "expected error to start with 'Failed to parse issue.md: ', got: {}",
                err_msg
            );
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_with_repo_arg() {
            let temp = TempDir::new().unwrap();
            // Create dir without owner/repo structure
            let new_dir = temp.path().join("new");
            std::fs::create_dir_all(&new_dir).unwrap();
            write_issue_md(
                &new_dir,
                indoc! {"
                    ---
                    title: With Repo Arg
                    ---

                    Body.
                "},
            );

            let mock = GitHubMockServer::start().await;
            mock.repo("custom", "repo")
                .issue(99)
                .title("With Repo Arg")
                .body("Body.\n")
                .labels(vec![])
                .create()
                .await;

            let client = mock.client();
            let args = make_args(Some("custom/repo".to_string()), false);
            let result = run_create_with_client(&args, new_dir.clone(), &client).await;

            assert!(result.is_ok());

            // Verify directory was renamed
            let renamed_dir = temp.path().join("99");
            assert!(renamed_dir.exists());
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_with_sub_issues_links_via_api() {
            let temp = TempDir::new().unwrap();
            let new_dir = setup_new_issue_dir(temp.path());
            write_issue_md(
                &new_dir,
                indoc! {"
                    ---
                    title: Parent Issue
                    subIssues:
                      - owner/repo#10
                    ---

                    Body.
                "},
            );

            let mock = GitHubMockServer::start().await;
            // Mock create-issue (POST /repos/{owner}/{repo}/issues).
            mock.repo("owner", "repo")
                .issue(50)
                .title("Parent Issue")
                .body("Body.\n")
                .labels(vec![])
                .create()
                .await;
            // Mock get_issue_id for the child reference (issue #10) so the
            // create path can resolve it to an internal ID before linking.
            mock.repo("owner", "repo").get_issue_id(10, 12345).await;
            // Mock POST .../{50}/sub_issues for adding the sub-issue link.
            mock.repo("owner", "repo").add_sub_issue(50).await;
            // Re-fetch to refresh local frontmatter post-link.
            mock.repo("owner", "repo")
                .issue(50)
                .title("Parent Issue")
                .body("Body.")
                .get()
                .await;
            mock.repo("owner", "repo").sub_issues_empty(50).await;

            let client = mock.client();
            let args = make_args(None, false);
            let result = run_create_with_client(&args, new_dir.clone(), &client).await;

            assert!(result.is_ok(), "expected ok, got: {:?}", result.err());
            // Verify the directory was renamed to the assigned issue number.
            let renamed_dir = temp.path().join("owner").join("repo").join("50");
            assert!(renamed_dir.exists());
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_with_parent_issue_links_via_api() {
            let temp = TempDir::new().unwrap();
            let new_dir = setup_new_issue_dir(temp.path());
            write_issue_md(
                &new_dir,
                indoc! {"
                    ---
                    title: Child Issue
                    parentIssue: owner/repo#1
                    ---

                    Body.
                "},
            );

            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo")
                .issue(60)
                .title("Child Issue")
                .body("Body.\n")
                .labels(vec![])
                .create()
                .await;
            // get_issue_id for "this" issue (60), then add_sub_issue on parent (#1).
            mock.repo("owner", "repo").get_issue_id(60, 99999).await;
            mock.repo("owner", "repo").add_sub_issue(1).await;
            // Re-fetch after linking.
            mock.repo("owner", "repo")
                .issue(60)
                .title("Child Issue")
                .body("Body.")
                .get()
                .await;
            mock.repo("owner", "repo").sub_issues_empty(60).await;

            let client = mock.client();
            let args = make_args(None, false);
            let result = run_create_with_client(&args, new_dir.clone(), &client).await;

            assert!(result.is_ok(), "expected ok, got: {:?}", result.err());
            let renamed_dir = temp.path().join("owner").join("repo").join("60");
            assert!(renamed_dir.exists());
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_unknown_frontmatter_key_errors() {
            let temp = TempDir::new().unwrap();
            let new_dir = setup_new_issue_dir(temp.path());
            // Typo: "parentIssues" (plural) instead of "parentIssue"
            write_issue_md(
                &new_dir,
                indoc! {"
                    ---
                    title: Typo
                    parentIssues: owner/repo#1
                    ---

                    Body.
                "},
            );

            let mock = GitHubMockServer::start().await;
            // No mocks needed; parsing should fail before any API call.
            let client = mock.client();
            let args = make_args(None, false);
            let result = run_create_with_client(&args, new_dir.clone(), &client).await;

            let err = result.unwrap_err().to_string();
            assert_eq!(
                err,
                "Failed to parse issue.md: Unknown frontmatter key(s): parentIssues. \
                 Allowed keys: title, labels, assignees, parentIssue, subIssues"
            );
            // Directory must remain unchanged on parse failure.
            assert!(new_dir.exists());
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_cannot_determine_repo() {
            let temp = TempDir::new().unwrap();
            // Create dir without owner/repo/new structure and no -R arg
            let some_dir = temp.path().join("random");
            std::fs::create_dir_all(&some_dir).unwrap();
            write_issue_md(
                &some_dir,
                indoc! {"
                    ---
                    title: Test
                    ---

                    Body.
                "},
            );

            let mock = GitHubMockServer::start().await;
            let client = mock.client();
            let args = make_args(None, false);
            let result = run_create_with_client(&args, some_dir, &client).await;

            let err = result.unwrap_err();
            let err_msg = err.to_string();
            assert!(
                err_msg.starts_with("Cannot determine repository from path '"),
                "expected error to start with 'Cannot determine repository from path ', got: {}",
                err_msg
            );
        }
    }

    mod get_repo_from_path_tests {
        use super::*;

        #[rstest]
        fn test_with_repo_arg() {
            let path = PathBuf::from("/some/random/path");
            let repo_arg = Some("owner/repo".to_string());

            let result = get_repo_from_path_or_arg(&path, &repo_arg).unwrap();
            assert_eq!(result, "owner/repo");
        }

        #[rstest]
        fn test_extract_from_new_path() {
            let path = PathBuf::from("/home/user/.cache/gh-issue-agent/fohte/armyknife/new");
            let result = get_repo_from_path_or_arg(&path, &None).unwrap();
            assert_eq!(result, "fohte/armyknife");
        }

        #[rstest]
        fn test_extract_from_relative_new_path() {
            let path = PathBuf::from("owner/repo/new");
            let result = get_repo_from_path_or_arg(&path, &None).unwrap();
            assert_eq!(result, "owner/repo");
        }

        #[rstest]
        fn test_fail_without_new_suffix() {
            let path = PathBuf::from("/home/user/.cache/gh-issue-agent/fohte/armyknife/123");
            let result = get_repo_from_path_or_arg(&path, &None);
            assert!(result.is_err());
        }

        #[rstest]
        fn test_fail_too_short_path() {
            let path = PathBuf::from("new");
            let result = get_repo_from_path_or_arg(&path, &None);
            assert!(result.is_err());
        }
    }

    mod display_new_issue_tests {
        use super::*;
        use crate::commands::gh::issue_agent::models::{EditableIssueFields, NewIssueFrontmatter};

        fn make_issue(fields: EditableIssueFields, body: &str) -> NewIssue {
            NewIssue {
                frontmatter: NewIssueFrontmatter { fields },
                body: body.to_string(),
            }
        }

        #[rstest]
        fn test_display_with_labels_and_assignees() {
            let issue = make_issue(
                EditableIssueFields {
                    title: "Test Title".to_string(),
                    labels: vec!["bug".to_string(), "urgent".to_string()],
                    assignees: vec!["user1".to_string()],
                    ..Default::default()
                },
                "Test body content.",
            );

            // Just verify it doesn't panic
            display_new_issue(&issue);
        }

        #[rstest]
        fn test_display_with_empty_body() {
            let issue = make_issue(
                EditableIssueFields {
                    title: "Title Only".to_string(),
                    ..Default::default()
                },
                "",
            );

            // Just verify it doesn't panic
            display_new_issue(&issue);
        }

        #[rstest]
        fn test_display_minimal() {
            let issue = make_issue(
                EditableIssueFields {
                    title: "Minimal".to_string(),
                    ..Default::default()
                },
                "Body",
            );

            // Just verify it doesn't panic
            display_new_issue(&issue);
        }

        #[rstest]
        fn test_display_with_parent_and_sub_issues() {
            let issue = make_issue(
                EditableIssueFields {
                    title: "Linked Issue".to_string(),
                    parent_issue: Some("owner/repo#1".to_string()),
                    sub_issues: vec!["owner/repo#10".to_string(), "owner/repo#20".to_string()],
                    ..Default::default()
                },
                "Body",
            );

            // Just verify it doesn't panic
            display_new_issue(&issue);
        }
    }
}
