//! Init command for creating new issue/comment boilerplate files.

use clap::{Args, Subcommand};

use super::common::{get_repo_from_arg_or_git, parse_repo};
use crate::commands::gh::issue_agent::models::IssueTemplate;
use crate::commands::gh::issue_agent::storage::IssueStorage;
use crate::infra::github::{OctocrabClient, RepoClient};

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
    let client = OctocrabClient::get()?;
    match &args.command {
        InitCommands::Issue(issue_args) => run_init_issue(issue_args, client).await,
        InitCommands::Comment(comment_args) => run_init_comment(comment_args, client).await,
    }
}

/// Validate that a repository exists on GitHub.
async fn validate_repo_exists(
    client: &OctocrabClient,
    owner: &str,
    repo: &str,
) -> anyhow::Result<()> {
    if !client.repo_exists(owner, repo).await? {
        anyhow::bail!("Repository '{}/{}' not found on GitHub", owner, repo);
    }
    Ok(())
}

/// Initialize a new issue boilerplate file.
async fn run_init_issue(args: &InitIssueArgs, client: &OctocrabClient) -> anyhow::Result<()> {
    let repo = get_repo_from_arg_or_git(&args.repo)?;
    // Validate repo format to prevent path traversal
    let (owner, repo_name) = parse_repo(&repo)?;

    // Validate repository exists on GitHub
    validate_repo_exists(client, &owner, &repo_name).await?;

    // Handle --list-templates
    if args.list_templates {
        return list_templates(client, &owner, &repo_name).await;
    }

    // Determine which template to use
    let template = if args.no_template {
        None
    } else {
        fetch_and_select_template(client, &owner, &repo_name, args.template.as_deref()).await?
    };

    let storage = IssueStorage::new_for_new_issue(&repo);
    run_init_issue_with_storage(&storage, template.as_ref())
}

/// List available issue templates and exit.
async fn list_templates(client: &OctocrabClient, owner: &str, repo: &str) -> anyhow::Result<()> {
    let templates = fetch_templates_with_fallback(client, owner, repo).await;
    print_template_list(owner, repo, &templates);
    Ok(())
}

/// Print the list of available templates to stderr.
fn print_template_list(owner: &str, repo: &str, templates: &[IssueTemplate]) {
    if templates.is_empty() {
        eprintln!("No issue templates found for {}/{}", owner, repo);
    } else {
        eprintln!("Available issue templates for {}/{}:", owner, repo);
        for t in templates {
            let about = t.about.as_deref().unwrap_or("");
            if about.is_empty() {
                eprintln!("  - {}", t.name);
            } else {
                eprintln!("  - {} - {}", t.name, about);
            }
        }
    }
}

/// Fetch templates from GitHub API, with graceful fallback on error.
async fn fetch_templates_with_fallback(
    client: &OctocrabClient,
    owner: &str,
    repo: &str,
) -> Vec<IssueTemplate> {
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
    client: &OctocrabClient,
    owner: &str,
    repo: &str,
    requested_name: Option<&str>,
) -> anyhow::Result<Option<IssueTemplate>> {
    let templates = fetch_templates_with_fallback(client, owner, repo).await;
    select_template(templates, requested_name)
}

/// Result of template selection logic.
#[derive(Debug, PartialEq, Eq)]
enum TemplateSelectionResult {
    /// Use the selected template
    Selected(IssueTemplate),
    /// No template available or requested - use default
    UseDefault,
    /// Template not found
    NotFound {
        requested: String,
        available: Vec<String>,
    },
    /// Multiple templates available - user must choose
    MultipleAvailable(Vec<IssueTemplate>),
}

/// Pure template selection logic, separated from I/O for testability.
fn select_template_pure(
    templates: Vec<IssueTemplate>,
    requested_name: Option<&str>,
) -> TemplateSelectionResult {
    match (templates.len(), requested_name) {
        // No templates available, none requested - use default
        (0, None) => TemplateSelectionResult::UseDefault,

        // No templates available but specific one requested - not found
        (0, Some(name)) => TemplateSelectionResult::NotFound {
            requested: name.to_string(),
            available: vec![],
        },

        // Specific template requested
        (_, Some(name)) => {
            let available: Vec<String> = templates.iter().map(|t| t.name.clone()).collect();
            match templates.into_iter().find(|t| t.name == name) {
                Some(template) => TemplateSelectionResult::Selected(template),
                None => TemplateSelectionResult::NotFound {
                    requested: name.to_string(),
                    available,
                },
            }
        }

        // Single template - auto-select
        (1, None) => {
            // SAFETY: We know templates.len() == 1 from the match arm
            match templates.into_iter().next() {
                Some(template) => TemplateSelectionResult::Selected(template),
                None => TemplateSelectionResult::UseDefault,
            }
        }

        // Multiple templates - require explicit choice
        (_, None) => TemplateSelectionResult::MultipleAvailable(templates),
    }
}

/// Select a template from the available templates.
///
/// Handles I/O (stderr messages) and converts selection result to anyhow::Result.
fn select_template(
    templates: Vec<IssueTemplate>,
    requested_name: Option<&str>,
) -> anyhow::Result<Option<IssueTemplate>> {
    match select_template_pure(templates, requested_name) {
        TemplateSelectionResult::Selected(template) => {
            if requested_name.is_none() {
                eprintln!("Using template: {}", template.name);
            }
            Ok(Some(template))
        }
        TemplateSelectionResult::UseDefault => Ok(None),
        TemplateSelectionResult::NotFound {
            requested,
            available,
        } => {
            anyhow::bail!(
                "Template '{}' not found. Available templates: {}",
                requested,
                available.join(", ")
            )
        }
        TemplateSelectionResult::MultipleAvailable(templates) => {
            eprintln!("Multiple issue templates found ({}):", templates.len());
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
async fn run_init_comment(args: &InitCommentArgs, client: &OctocrabClient) -> anyhow::Result<()> {
    let repo = get_repo_from_arg_or_git(&args.repo)?;
    // Validate repo format to prevent path traversal
    let (owner, repo_name) = parse_repo(&repo)?;

    // Validate comment name if provided (local validation first, before network call)
    if let Some(name) = &args.name {
        validate_comment_name(name)?;
    }

    // Validate repository exists on GitHub
    validate_repo_exists(client, &owner, &repo_name).await?;

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
            assert_eq!(
                content,
                "---\ntitle: 'Bug: '\nlabels:\n- bug\nassignees: []\n---\n\nDescribe the bug\n"
            );
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

    mod select_template_pure_tests {
        use super::*;

        fn template(name: &str) -> IssueTemplate {
            IssueTemplate {
                name: name.to_string(),
                title: None,
                body: None,
                about: None,
                filename: None,
                labels: vec![],
                assignees: vec![],
            }
        }

        #[rstest]
        #[case::no_templates_no_request(vec![], None, TemplateSelectionResult::UseDefault)]
        #[case::no_templates_with_request(
            vec![],
            Some("Bug"),
            TemplateSelectionResult::NotFound {
                requested: "Bug".to_string(),
                available: vec![],
            }
        )]
        #[case::single_template_auto_selects(
            vec![template("Bug")],
            None,
            TemplateSelectionResult::Selected(template("Bug"))
        )]
        #[case::single_template_matching_name(
            vec![template("Bug")],
            Some("Bug"),
            TemplateSelectionResult::Selected(template("Bug"))
        )]
        #[case::single_template_non_matching(
            vec![template("Bug")],
            Some("Feature"),
            TemplateSelectionResult::NotFound {
                requested: "Feature".to_string(),
                available: vec!["Bug".to_string()],
            }
        )]
        #[case::multiple_templates_no_request(
            vec![template("Bug"), template("Feature")],
            None,
            TemplateSelectionResult::MultipleAvailable(vec![template("Bug"), template("Feature")])
        )]
        #[case::multiple_templates_matching_name(
            vec![template("Bug"), template("Feature")],
            Some("Bug"),
            TemplateSelectionResult::Selected(template("Bug"))
        )]
        #[case::multiple_templates_non_matching(
            vec![template("Bug"), template("Feature")],
            Some("Docs"),
            TemplateSelectionResult::NotFound {
                requested: "Docs".to_string(),
                available: vec!["Bug".to_string(), "Feature".to_string()],
            }
        )]
        fn test_select_template_pure(
            #[case] templates: Vec<IssueTemplate>,
            #[case] requested_name: Option<&str>,
            #[case] expected: TemplateSelectionResult,
        ) {
            let result = select_template_pure(templates, requested_name);
            assert_eq!(result, expected);
        }
    }

    mod select_template_tests {
        use super::*;

        fn template(name: &str) -> IssueTemplate {
            IssueTemplate {
                name: name.to_string(),
                title: None,
                body: None,
                about: None,
                filename: None,
                labels: vec![],
                assignees: vec![],
            }
        }

        #[rstest]
        #[case::no_templates(vec![], None, Ok(None))]
        #[case::single_auto_selects(vec![template("Bug")], None, Ok(Some(template("Bug"))))]
        #[case::selects_by_name(
            vec![template("Bug"), template("Feature")],
            Some("Bug"),
            Ok(Some(template("Bug")))
        )]
        fn test_select_template_ok(
            #[case] templates: Vec<IssueTemplate>,
            #[case] requested_name: Option<&str>,
            #[case] expected: anyhow::Result<Option<IssueTemplate>>,
        ) {
            let result = select_template(templates, requested_name);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), expected.unwrap());
        }

        #[rstest]
        #[case::not_found(
            vec![template("Bug")],
            Some("NonExistent"),
            "Template 'NonExistent' not found. Available templates: Bug"
        )]
        #[case::no_templates_with_request(
            vec![],
            Some("Bug"),
            "Template 'Bug' not found. Available templates: "
        )]
        #[case::multiple_without_selection(
            vec![template("Bug"), template("Feature")],
            None,
            "Use --template <NAME> to select a template, or --no-template to use the default boilerplate."
        )]
        fn test_select_template_err(
            #[case] templates: Vec<IssueTemplate>,
            #[case] requested_name: Option<&str>,
            #[case] expected_msg: &str,
        ) {
            let result = select_template(templates, requested_name);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().to_string(), expected_msg);
        }
    }

    mod print_template_list_tests {
        use super::*;

        fn template_with_about(name: &str, about: Option<&str>) -> IssueTemplate {
            IssueTemplate {
                name: name.to_string(),
                title: None,
                body: None,
                about: about.map(|s| s.to_string()),
                filename: None,
                labels: vec![],
                assignees: vec![],
            }
        }

        #[rstest]
        #[case::empty(&[])]
        #[case::single_with_about(&[template_with_about("Bug", Some("Report a bug"))])]
        #[case::single_without_about(&[template_with_about("Feature", None)])]
        #[case::multiple(&[
            template_with_about("Bug", Some("Report a bug")),
            template_with_about("Feature", None),
        ])]
        fn test_print_template_list_does_not_panic(#[case] templates: &[IssueTemplate]) {
            print_template_list("owner", "repo", templates);
        }
    }

    mod fetch_templates_with_fallback_tests {
        use super::*;
        use crate::commands::gh::issue_agent::commands::test_helpers::GitHubMockServer;

        #[rstest]
        #[tokio::test]
        async fn test_returns_templates_from_api() {
            let mock = GitHubMockServer::start().await;
            let template = IssueTemplate {
                name: "Bug Report".to_string(),
                title: Some("[Bug]: ".to_string()),
                body: Some("Describe the bug".to_string()),
                about: Some("Report a bug".to_string()),
                filename: None,
                labels: vec!["bug".to_string()],
                assignees: vec![],
            };
            mock.repo("owner", "repo")
                .graphql_issue_templates(std::slice::from_ref(&template))
                .await;

            let client = mock.client();
            let templates = fetch_templates_with_fallback(&client, "owner", "repo").await;

            assert_eq!(templates.len(), 1);
            assert_eq!(templates[0].name, "Bug Report");
        }

        #[rstest]
        #[tokio::test]
        async fn test_returns_empty_on_api_error() {
            let mock = GitHubMockServer::start().await;
            // No mock set up, so API call will fail

            let client = mock.client();
            let templates = fetch_templates_with_fallback(&client, "owner", "repo").await;

            assert!(templates.is_empty());
        }
    }

    mod validate_repo_exists_tests {
        use super::*;
        use crate::commands::gh::issue_agent::commands::test_helpers::GitHubMockServer;

        #[rstest]
        #[tokio::test]
        async fn test_returns_ok_when_repo_exists() {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo").repo_info().get().await;

            let client = mock.client();
            let result = validate_repo_exists(&client, "owner", "repo").await;

            assert!(result.is_ok());
        }
    }
}
