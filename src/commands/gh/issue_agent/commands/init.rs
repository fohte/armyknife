//! Init command for creating new issue/comment boilerplate files.

use clap::{Args, Subcommand};

use super::common::{get_repo_from_arg_or_git, parse_repo};
use crate::commands::gh::issue_agent::models::IssueTemplate;
use crate::commands::gh::issue_agent::storage::IssueStorage;
use crate::infra::github::OctocrabClient;

/// Arguments for the init command.
#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct InitArgs {
    #[command(subcommand)]
    pub command: InitCommands,
}

/// Subcommands for init.
#[derive(Subcommand, Clone, PartialEq, Eq, Debug)]
pub enum InitCommands {
    /// Create a new issue boilerplate file
    Issue(InitIssueArgs),

    /// Create a new comment boilerplate file for an existing issue
    Comment(InitCommentArgs),
}

/// Arguments for `init issue`.
#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct InitIssueArgs {
    /// Target repository (owner/repo)
    #[arg(short = 'R', long)]
    pub repo: Option<String>,

    /// Use a specific issue template by name
    #[arg(long)]
    pub template: Option<String>,

    /// Do not use any issue template (use default boilerplate)
    #[arg(long, conflicts_with = "template")]
    pub no_template: bool,

    /// List available issue templates and exit
    #[arg(long)]
    pub list_templates: bool,
}

/// Arguments for `init comment`.
#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct InitCommentArgs {
    /// Issue number
    pub issue_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long)]
    pub repo: Option<String>,

    /// Name for the comment file (default: timestamp)
    #[arg(long)]
    pub name: Option<String>,
}

/// Run the init command.
pub async fn run(args: &InitArgs) -> anyhow::Result<()> {
    match &args.command {
        InitCommands::Issue(issue_args) => run_init_issue(issue_args).await,
        InitCommands::Comment(comment_args) => run_init_comment(comment_args),
    }
}

/// Initialize a new issue boilerplate file.
async fn run_init_issue(args: &InitIssueArgs) -> anyhow::Result<()> {
    let repo = get_repo_from_arg_or_git(&args.repo)?;
    // Validate repo format to prevent path traversal
    let (owner, repo_name) = parse_repo(&repo)?;

    // Handle --list-templates
    if args.list_templates {
        return list_templates(&owner, &repo_name).await;
    }

    // Determine which template to use
    let template = if args.no_template {
        None
    } else {
        fetch_and_select_template(&owner, &repo_name, args.template.as_deref()).await?
    };

    let storage = IssueStorage::new_for_new_issue(&repo);
    run_init_issue_with_storage(&storage, template.as_ref())
}

/// List available issue templates and exit.
async fn list_templates(owner: &str, repo: &str) -> anyhow::Result<()> {
    let templates = fetch_templates_with_fallback(owner, repo).await;

    if templates.is_empty() {
        eprintln!("No issue templates found for {}/{}", owner, repo);
    } else {
        eprintln!("Available issue templates for {}/{}:", owner, repo);
        for t in &templates {
            let about = t.about.as_deref().unwrap_or("");
            if about.is_empty() {
                eprintln!("  - {}", t.name);
            } else {
                eprintln!("  - {} - {}", t.name, about);
            }
        }
    }
    Ok(())
}

/// Fetch templates from GitHub API, with graceful fallback on error.
async fn fetch_templates_with_fallback(owner: &str, repo: &str) -> Vec<IssueTemplate> {
    let client = match OctocrabClient::get() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: Failed to initialize GitHub client: {}", e);
            return vec![];
        }
    };

    match client.get_issue_templates(owner, repo).await {
        Ok(templates) => templates,
        Err(e) => {
            eprintln!("Warning: Failed to fetch issue templates: {}", e);
            vec![]
        }
    }
}

/// Fetch and select a template based on the provided options.
///
/// Returns:
/// - `Ok(Some(template))` if a template should be used
/// - `Ok(None)` if no template should be used (fallback to default)
/// - `Err(...)` if the user needs to make a choice or template not found
async fn fetch_and_select_template(
    owner: &str,
    repo: &str,
    requested_name: Option<&str>,
) -> anyhow::Result<Option<IssueTemplate>> {
    let templates = fetch_templates_with_fallback(owner, repo).await;

    match (templates.len(), requested_name) {
        // No templates available - use default
        (0, _) => Ok(None),

        // Specific template requested
        (_, Some(name)) => {
            let available: Vec<String> = templates.iter().map(|t| t.name.clone()).collect();
            templates
                .into_iter()
                .find(|t| t.name == name)
                .map(Some)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "Template '{}' not found. Available templates: {}",
                        name,
                        available.join(", ")
                    )
                })
        }

        // Single template - auto-select
        (1, None) => {
            // SAFETY: We know templates.len() == 1 from the match arm
            if let Some(template) = templates.into_iter().next() {
                eprintln!("Using template: {}", template.name);
                Ok(Some(template))
            } else {
                // This branch is unreachable due to the match guard, but required for exhaustiveness
                Ok(None)
            }
        }

        // Multiple templates - require explicit choice
        (n, None) => {
            eprintln!("Multiple issue templates found ({}):", n);
            for t in &templates {
                let about = t.about.as_deref().unwrap_or("");
                if about.is_empty() {
                    eprintln!("  - {}", t.name);
                } else {
                    eprintln!("  - {} - {}", t.name, about);
                }
            }
            anyhow::bail!(
                "Use --template <NAME> to select a template, or --no-template to use the default boilerplate."
            )
        }
    }
}

fn run_init_issue_with_storage(
    storage: &IssueStorage,
    template: Option<&IssueTemplate>,
) -> anyhow::Result<()> {
    let path = storage.init_new_issue(template)?;

    eprintln!("Created: {}", path.display());
    eprintln!();
    eprintln!(
        "Edit the file, then run: a gh issue-agent push {}",
        storage.dir().display()
    );

    Ok(())
}

/// Validate comment name to prevent path traversal.
fn validate_comment_name(name: &str) -> anyhow::Result<()> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        anyhow::bail!("Invalid comment name: must not contain '/', '\\', or '..'");
    }
    Ok(())
}

/// Initialize a new comment boilerplate file.
fn run_init_comment(args: &InitCommentArgs) -> anyhow::Result<()> {
    let repo = get_repo_from_arg_or_git(&args.repo)?;
    // Validate repo format to prevent path traversal
    let _ = parse_repo(&repo)?;

    // Validate comment name if provided
    if let Some(name) = &args.name {
        validate_comment_name(name)?;
    }

    let storage = IssueStorage::new(&repo, args.issue_number as i64);
    run_init_comment_with_storage(&storage, args.issue_number, args.name.as_deref())
}

fn run_init_comment_with_storage(
    storage: &IssueStorage,
    issue_number: u64,
    name: Option<&str>,
) -> anyhow::Result<()> {
    // Check if issue exists locally
    if !storage.dir().exists() {
        anyhow::bail!(
            "Issue #{} not found locally. Run 'a gh issue-agent pull {}' first.",
            issue_number,
            issue_number
        );
    }

    let path = storage.init_new_comment(name)?;

    eprintln!("Created: {}", path.display());
    eprintln!();
    eprintln!(
        "Edit the file and run: a gh issue-agent push {}",
        issue_number
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs;

    mod run_init_issue_with_storage_tests {
        use super::*;

        #[rstest]
        fn test_creates_issue_file_with_default_template() {
            let dir = tempfile::tempdir().unwrap();
            let storage = IssueStorage::from_dir(dir.path());

            let result = run_init_issue_with_storage(&storage, None);
            assert!(result.is_ok());

            let path = dir.path().join("issue.md");
            assert!(path.exists());

            let content = fs::read_to_string(&path).unwrap();
            assert_eq!(
                content,
                "---\ntitle: \"\"\nlabels: []\nassignees: []\n---\n\nBody\n"
            );
        }

        #[rstest]
        fn test_creates_issue_file_with_custom_template() {
            let dir = tempfile::tempdir().unwrap();
            let storage = IssueStorage::from_dir(dir.path());

            let template = IssueTemplate {
                name: "Bug Report".to_string(),
                title: Some("Bug: ".to_string()),
                body: Some("Describe the bug".to_string()),
                about: None,
                filename: None,
                labels: vec!["bug".to_string()],
                assignees: vec![],
            };

            let result = run_init_issue_with_storage(&storage, Some(&template));
            assert!(result.is_ok());

            let path = dir.path().join("issue.md");
            assert!(path.exists());

            let content = fs::read_to_string(&path).unwrap();
            assert!(content.contains("title: 'Bug: '"));
            assert!(content.contains("- bug"));
            assert!(content.contains("Describe the bug"));
        }

        #[rstest]
        fn test_returns_error_if_file_exists() {
            let dir = tempfile::tempdir().unwrap();
            let storage = IssueStorage::from_dir(dir.path());

            // Create file first
            run_init_issue_with_storage(&storage, None).unwrap();

            // Second call should fail
            let result = run_init_issue_with_storage(&storage, None);
            assert!(result.is_err());
            let err = result.unwrap_err();
            let expected = format!(
                "File already exists: {}",
                dir.path().join("issue.md").display()
            );
            assert_eq!(err.to_string(), expected);
        }
    }

    mod run_init_comment_with_storage_tests {
        use super::*;

        #[rstest]
        fn test_creates_comment_file_with_name() {
            let dir = tempfile::tempdir().unwrap();
            // Create issue directory to simulate pulled issue
            fs::create_dir_all(dir.path()).unwrap();
            fs::write(dir.path().join("issue.md"), "test").unwrap();

            let storage = IssueStorage::from_dir(dir.path());

            let result = run_init_comment_with_storage(&storage, 123, Some("test"));
            assert!(result.is_ok());

            let path = dir.path().join("comments/new_test.md");
            assert!(path.exists());

            let content = fs::read_to_string(&path).unwrap();
            assert_eq!(content, "Comment body\n");
        }

        #[rstest]
        fn test_creates_comment_file_with_timestamp() {
            let dir = tempfile::tempdir().unwrap();
            // Create issue directory to simulate pulled issue
            fs::create_dir_all(dir.path()).unwrap();
            fs::write(dir.path().join("issue.md"), "test").unwrap();

            let storage = IssueStorage::from_dir(dir.path());

            let result = run_init_comment_with_storage(&storage, 123, None);
            assert!(result.is_ok());

            // Check that a file was created in comments directory
            let comments_dir = dir.path().join("comments");
            assert!(comments_dir.exists());
            let entries: Vec<_> = fs::read_dir(&comments_dir).unwrap().collect();
            assert_eq!(entries.len(), 1);

            let filename = entries[0].as_ref().unwrap().file_name();
            let filename_str = filename.to_string_lossy();
            assert!(filename_str.starts_with("new_"));
            assert!(filename_str.ends_with(".md"));
        }

        #[rstest]
        fn test_returns_error_if_issue_not_pulled() {
            let dir = tempfile::tempdir().unwrap();
            // Don't create any files - issue not pulled
            let storage = IssueStorage::from_dir(dir.path().join("nonexistent"));

            let result = run_init_comment_with_storage(&storage, 123, Some("test"));
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "Issue #123 not found locally. Run 'a gh issue-agent pull 123' first."
            );
        }

        #[rstest]
        fn test_returns_error_if_file_exists() {
            let dir = tempfile::tempdir().unwrap();
            // Create issue directory to simulate pulled issue
            fs::create_dir_all(dir.path()).unwrap();
            fs::write(dir.path().join("issue.md"), "test").unwrap();

            let storage = IssueStorage::from_dir(dir.path());

            // Create file first
            run_init_comment_with_storage(&storage, 123, Some("duplicate")).unwrap();

            // Second call with same name should fail
            let result = run_init_comment_with_storage(&storage, 123, Some("duplicate"));
            assert!(result.is_err());
            let err = result.unwrap_err();
            let expected = format!(
                "File already exists: {}",
                dir.path().join("comments/new_duplicate.md").display()
            );
            assert_eq!(err.to_string(), expected);
        }
    }

    mod validate_comment_name_tests {
        use super::*;

        #[rstest]
        #[case::valid_simple("my_comment")]
        #[case::valid_with_dash("my-comment")]
        #[case::valid_with_numbers("comment123")]
        fn test_valid_names(#[case] name: &str) {
            assert!(validate_comment_name(name).is_ok());
        }

        #[rstest]
        #[case::forward_slash("../escape")]
        #[case::forward_slash_middle("foo/bar")]
        #[case::backslash("foo\\bar")]
        #[case::double_dot("..")]
        #[case::double_dot_prefix("..hidden")]
        fn test_invalid_names(#[case] name: &str) {
            let result = validate_comment_name(name);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "Invalid comment name: must not contain '/', '\\', or '..'"
            );
        }
    }

    mod run_init_issue_tests {
        use super::*;

        #[rstest]
        #[tokio::test]
        async fn test_rejects_invalid_repo_format() {
            let args = InitIssueArgs {
                repo: Some("invalid-repo-format".to_string()),
                template: None,
                no_template: false,
                list_templates: false,
            };

            let result = run_init_issue(&args).await;
            assert!(result.is_err());
            // parse_repo should reject repo without '/'
        }
    }

    mod run_init_comment_tests {
        use super::*;

        #[rstest]
        fn test_rejects_invalid_repo_format() {
            let args = InitCommentArgs {
                issue_number: 123,
                repo: Some("invalid-repo-format".to_string()),
                name: None,
            };

            let result = run_init_comment(&args);
            assert!(result.is_err());
        }

        #[rstest]
        fn test_rejects_path_traversal_in_name() {
            let args = InitCommentArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
                name: Some("../escape".to_string()),
            };

            let result = run_init_comment(&args);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "Invalid comment name: must not contain '/', '\\', or '..'"
            );
        }
    }
}
