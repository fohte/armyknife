use indoc::formatdoc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::LazyLock;
use thiserror::Error;

use crate::human_in_the_loop::{Document, DocumentSchema};

static GITHUB_URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:github\.com[:/])([^/]+)/([^/]+?)(?:\.git)?$").unwrap());
static FRONTMATTER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^---\n([\s\S]*?)\n---\n?").unwrap());
static JAPANESE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\p{Hiragana}\p{Katakana}\p{Han}]").unwrap());

/// Trait for executing external commands (git, gh).
/// Enables dependency injection for testing without modifying global state.
pub trait CommandRunner {
    fn run_git(&self, args: &[&str]) -> io::Result<Output>;
    fn run_gh(&self, args: &[&str]) -> io::Result<Output>;

    /// Run gh with OsString arguments (for commands with file paths)
    fn run_gh_with_args(&self, args: &[std::ffi::OsString]) -> io::Result<Output>;

    /// Open PR in browser (fire-and-forget, ok to fail silently)
    fn open_in_browser(&self, repo: &str, pr_url: &str);
}

/// Production implementation that executes real commands.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealCommandRunner;

impl CommandRunner for RealCommandRunner {
    fn run_git(&self, args: &[&str]) -> io::Result<Output> {
        Command::new("git").args(args).output()
    }

    fn run_gh(&self, args: &[&str]) -> io::Result<Output> {
        Command::new("gh").args(args).output()
    }

    fn run_gh_with_args(&self, args: &[std::ffi::OsString]) -> io::Result<Output> {
        Command::new("gh").args(args).output()
    }

    fn open_in_browser(&self, repo: &str, pr_url: &str) {
        let _ = Command::new("gh")
            .args(["pr", "view", "--web", "--repo", repo, pr_url])
            .status();
    }
}

#[derive(Error, Debug)]
pub enum PrDraftError {
    #[error("Not in a git repository")]
    NotInGitRepo,

    #[error("Failed to get repository info: {0}")]
    RepoInfoError(String),

    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("Draft file already exists: {0}\nUse --force to overwrite")]
    FileAlreadyExists(PathBuf),

    #[error("PR was not approved. Please run 'review' and set 'steps.submit: true'")]
    NotApproved,

    #[error("File has been modified after approval. Please run 'review' again")]
    ModifiedAfterApproval,

    #[error("Please set a title in the frontmatter")]
    EmptyTitle,

    #[error("Title contains Japanese characters but this is a public repo")]
    JapaneseInTitle,

    #[error("Body contains Japanese characters but this is a public repo")]
    JapaneseInBody,

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("YAML parse error: {0}")]
    YamlParse(#[from] serde_yaml::Error),

    #[error("Command failed: {0}")]
    CommandFailed(String),
}

pub type Result<T> = std::result::Result<T, PrDraftError>;

#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub owner: String,
    pub repo: String,
    pub branch: String,
    pub is_private: bool,
}

impl RepoInfo {
    /// Get repo info using gh CLI (includes is_private check via network)
    pub fn from_current_dir(runner: &impl CommandRunner) -> Result<Self> {
        let branch = get_current_branch(runner)?;
        let (owner, repo) = get_repo_owner_and_name_from_git(runner)?;
        let is_private = check_is_private(runner, &owner, &repo)?;

        Ok(Self {
            owner,
            repo,
            branch,
            is_private,
        })
    }

    /// Get repo info from git only (no network call, is_private defaults to false)
    pub fn from_git_only(runner: &impl CommandRunner) -> Result<Self> {
        let branch = get_current_branch(runner)?;
        let (owner, repo) = get_repo_owner_and_name_from_git(runner)?;

        Ok(Self {
            owner,
            repo,
            branch,
            is_private: false,
        })
    }
}

pub fn get_current_branch(runner: &impl CommandRunner) -> Result<String> {
    // Use rev-parse --abbrev-ref HEAD to handle detached HEAD state
    // (returns "HEAD" when detached, unlike branch --show-current which returns empty)
    let output = runner
        .run_git(&["rev-parse", "--abbrev-ref", "HEAD"])
        .map_err(|e| PrDraftError::RepoInfoError(e.to_string()))?;

    if !output.status.success() {
        return Err(PrDraftError::NotInGitRepo);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Get owner and repo from git remote origin URL (no network call)
pub fn get_repo_owner_and_name_from_git(runner: &impl CommandRunner) -> Result<(String, String)> {
    let output = runner
        .run_git(&["remote", "get-url", "origin"])
        .map_err(|e| PrDraftError::RepoInfoError(e.to_string()))?;

    if !output.status.success() {
        // Include actual git error message for better debugging
        return Err(PrDraftError::RepoInfoError(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_github_url(&url)
}

fn parse_github_url(url: &str) -> Result<(String, String)> {
    // SSH format: git@github.com:owner/repo.git
    // HTTPS format: https://github.com/owner/repo.git
    if let Some(captures) = GITHUB_URL_RE.captures(url) {
        let owner = captures.get(1).unwrap().as_str().to_string();
        let repo = captures.get(2).unwrap().as_str().to_string();
        Ok((owner, repo))
    } else {
        Err(PrDraftError::RepoInfoError(format!(
            "Could not parse GitHub URL: {url}"
        )))
    }
}

pub fn check_is_private(runner: &impl CommandRunner, owner: &str, repo: &str) -> Result<bool> {
    let repo_spec = format!("{owner}/{repo}");
    let output = runner
        .run_gh(&[
            "repo",
            "view",
            &repo_spec,
            "--json",
            "isPrivate",
            "-q",
            ".isPrivate",
        ])
        .map_err(|e| PrDraftError::RepoInfoError(e.to_string()))?;

    if !output.status.success() {
        return Err(PrDraftError::RepoInfoError(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    let is_private_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(is_private_str == "true")
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Frontmatter {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub steps: Steps,
}

impl DocumentSchema for Frontmatter {
    fn is_approved(&self) -> bool {
        self.steps.submit
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Steps {
    #[serde(default, rename = "ready-for-translation")]
    pub ready_for_translation: bool,
    #[serde(default)]
    pub submit: bool,
}

/// Type alias for PR draft documents.
#[allow(dead_code)]
pub type PrDraftDocument = Document<Frontmatter>;

#[derive(Debug, Clone)]
pub struct DraftFile {
    pub path: PathBuf,
    pub frontmatter: Frontmatter,
    pub body: String,
}

impl DraftFile {
    pub fn draft_dir() -> PathBuf {
        std::env::temp_dir().join("pr-body-draft")
    }

    pub fn path_for(repo_info: &RepoInfo) -> PathBuf {
        Self::draft_dir()
            .join(&repo_info.owner)
            .join(&repo_info.repo)
            .join(format!("{}.md", &repo_info.branch))
    }

    pub fn lock_path(draft_path: &Path) -> PathBuf {
        draft_path.with_extension("md.lock")
    }

    pub fn approve_path(draft_path: &Path) -> PathBuf {
        draft_path.with_extension("md.approve")
    }

    /// Extract owner, repo, and branch from a draft file path.
    /// Path format: /tmp/pr-body-draft/<owner>/<repo>/<branch>.md
    /// Note: branch names can contain "/" (e.g., "feature/foo"), resulting in nested paths.
    pub fn parse_path(path: &Path) -> Option<(String, String, String)> {
        let draft_dir = Self::draft_dir();
        let relative = path.strip_prefix(&draft_dir).ok()?;
        let components: Vec<_> = relative.components().collect();

        // Need at least 3 components: owner, repo, and at least one branch segment
        if components.len() < 3 {
            return None;
        }

        let owner = components[0].as_os_str().to_str()?.to_string();
        let repo = components[1].as_os_str().to_str()?.to_string();

        // Join remaining components to reconstruct branch name with "/"
        let branch_parts: Vec<&str> = components[2..]
            .iter()
            .filter_map(|c| c.as_os_str().to_str())
            .collect();
        let branch_path = branch_parts.join("/");
        let branch = branch_path.strip_suffix(".md")?.to_string();

        Some((owner, repo, branch))
    }

    pub fn from_path(path: PathBuf) -> Result<Self> {
        if !path.exists() {
            return Err(PrDraftError::FileNotFound(path));
        }

        let content = fs::read_to_string(&path)?;
        let (frontmatter, body) = parse_frontmatter(&content)?;

        Ok(Self {
            path,
            frontmatter,
            body,
        })
    }

    pub fn compute_hash(&self) -> io::Result<String> {
        let content = fs::read_to_string(&self.path)?;
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        Ok(format!("{:x}", hasher.finalize()))
    }

    #[allow(dead_code)]
    pub fn save_approval(&self) -> Result<()> {
        let hash = self.compute_hash()?;
        let approve_path = Self::approve_path(&self.path);
        fs::write(&approve_path, hash)?;
        Ok(())
    }

    pub fn remove_approval(&self) -> Result<()> {
        let approve_path = Self::approve_path(&self.path);
        if approve_path.exists() {
            fs::remove_file(&approve_path)?;
        }
        Ok(())
    }

    pub fn verify_approval(&self) -> Result<()> {
        let approve_path = Self::approve_path(&self.path);

        if !approve_path.exists() {
            return Err(PrDraftError::NotApproved);
        }

        let saved_hash = fs::read_to_string(&approve_path)?.trim().to_string();
        let current_hash = self.compute_hash()?;

        if saved_hash != current_hash {
            return Err(PrDraftError::ModifiedAfterApproval);
        }

        Ok(())
    }

    pub fn remove_lock(&self) -> Result<()> {
        let lock_path = Self::lock_path(&self.path);
        if lock_path.exists() {
            fs::remove_file(&lock_path)?;
        }
        Ok(())
    }

    pub fn cleanup(&self) -> Result<()> {
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        self.remove_lock()?;
        self.remove_approval()?;
        Ok(())
    }
}

fn parse_frontmatter(content: &str) -> Result<(Frontmatter, String)> {
    if let Some(captures) = FRONTMATTER_RE.captures(content) {
        let yaml_str = captures.get(1).map_or("", |m| m.as_str());
        let frontmatter: Frontmatter = serde_yaml::from_str(yaml_str)?;
        let body = content[captures.get(0).unwrap().end()..].to_string();
        Ok((frontmatter, body))
    } else {
        Ok((
            Frontmatter {
                title: String::new(),
                steps: Steps::default(),
            },
            content.to_string(),
        ))
    }
}

pub fn generate_frontmatter(title: &str, is_private: bool) -> String {
    // Use serde_yaml to properly escape title (handles ", \n, and other special chars)
    let escaped_title = serde_yaml::to_string(&title).unwrap_or_else(|_| format!("\"{title}\""));
    let escaped_title = escaped_title.trim();

    if is_private {
        formatdoc! {"
            ---
            title: {escaped_title}
            steps:
              submit: false
            ---
        "}
    } else {
        formatdoc! {"
            ---
            title: {escaped_title}
            steps:
              ready-for-translation: false
              submit: false
            ---
        "}
    }
}

pub fn contains_japanese(text: &str) -> bool {
    JAPANESE_RE.is_match(text)
}

pub fn read_stdin_if_available() -> Option<String> {
    use std::io::IsTerminal;

    if io::stdin().is_terminal() {
        return None;
    }

    let mut buffer = String::new();
    match io::stdin().read_to_string(&mut buffer) {
        Ok(_) if !buffer.is_empty() => Some(buffer),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    #[rstest]
    #[case::basic(
        indoc! {r#"
            ---
            title: "Test PR"
            steps:
              submit: true
            ---
            This is the body
        "#},
        "Test PR",
        false,
        true,
        "This is the body\n"
    )]
    #[case::with_ready_for_translation(
        indoc! {r#"
            ---
            title: "Public PR"
            steps:
              ready-for-translation: true
              submit: false
            ---
            Body content
        "#},
        "Public PR",
        true,
        false,
        "Body content\n"
    )]
    fn test_parse_frontmatter(
        #[case] content: &str,
        #[case] expected_title: &str,
        #[case] expected_ready_for_translation: bool,
        #[case] expected_submit: bool,
        #[case] expected_body: &str,
    ) {
        let (frontmatter, body) = parse_frontmatter(content).unwrap();
        assert_eq!(frontmatter.title, expected_title);
        assert_eq!(
            frontmatter.steps.ready_for_translation,
            expected_ready_for_translation
        );
        assert_eq!(frontmatter.steps.submit, expected_submit);
        assert_eq!(body, expected_body);
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "Just a body without frontmatter";

        let (frontmatter, body) = parse_frontmatter(content).unwrap();
        assert_eq!(frontmatter.title, "");
        assert!(!frontmatter.steps.submit);
        assert_eq!(body, content);
    }

    #[rstest]
    #[case::hiragana("これはテスト", true)]
    #[case::mixed("Hello 世界", true)]
    #[case::katakana("カタカナ", true)]
    #[case::english("Hello World", false)]
    #[case::symbols("abc123!@#", false)]
    fn test_contains_japanese(#[case] text: &str, #[case] expected: bool) {
        assert_eq!(contains_japanese(text), expected);
    }

    #[rstest]
    #[case::private(true, false)]
    #[case::public(false, true)]
    fn test_generate_frontmatter(
        #[case] is_private: bool,
        #[case] should_contain_ready_for_translation: bool,
    ) {
        let result = generate_frontmatter("Test Title", is_private);
        // serde_yaml may quote the title differently, just check the title value is present
        assert!(result.contains("Test Title"), "result was: {result}");
        assert!(result.contains("submit: false"));
        assert_eq!(
            result.contains("ready-for-translation"),
            should_contain_ready_for_translation
        );
    }

    #[test]
    fn test_generate_frontmatter_escapes_special_chars() {
        // Test that special characters in title are properly escaped
        let result = generate_frontmatter("Title with \"quotes\" and\nnewline", false);
        // Should be parseable as valid YAML
        let parsed = parse_frontmatter(&result);
        assert!(parsed.is_ok(), "Failed to parse: {result}");
        let (frontmatter, _) = parsed.unwrap();
        assert_eq!(frontmatter.title, "Title with \"quotes\" and\nnewline");
    }
}

/// Test utilities for mocking command execution.
#[cfg(test)]
pub mod test_utils {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::process::ExitStatus;

    /// Mock implementation of CommandRunner for testing.
    /// Returns pre-configured responses without executing real commands.
    #[derive(Clone)]
    pub struct MockCommandRunner {
        pub branch: String,
        pub owner: String,
        pub repo: String,
        pub is_private: Option<bool>, // None = error (offline)
        pub gh_pr_create_result: Option<String>, // None = error
    }

    impl MockCommandRunner {
        pub fn new(owner: &str, repo: &str, branch: &str) -> Self {
            Self {
                branch: branch.to_string(),
                owner: owner.to_string(),
                repo: repo.to_string(),
                is_private: Some(true),
                gh_pr_create_result: Some("https://github.com/owner/repo/pull/1".to_string()),
            }
        }

        pub fn with_private(mut self, is_private: Option<bool>) -> Self {
            self.is_private = is_private;
            self
        }

        fn make_output(stdout: &str, success: bool) -> Output {
            Output {
                status: if success {
                    ExitStatus::from_raw(0)
                } else {
                    ExitStatus::from_raw(256) // exit code 1
                },
                stdout: stdout.as_bytes().to_vec(),
                stderr: if success {
                    Vec::new()
                } else {
                    stdout.as_bytes().to_vec()
                },
            }
        }
    }

    impl CommandRunner for MockCommandRunner {
        fn run_git(&self, args: &[&str]) -> io::Result<Output> {
            match args.first() {
                Some(&"rev-parse") => Ok(Self::make_output(&format!("{}\n", self.branch), true)),
                Some(&"remote") if args.get(1) == Some(&"get-url") => Ok(Self::make_output(
                    &format!("https://github.com/{}/{}.git\n", self.owner, self.repo),
                    true,
                )),
                _ => Ok(Self::make_output("unexpected git command", false)),
            }
        }

        fn run_gh(&self, args: &[&str]) -> io::Result<Output> {
            match args.first() {
                Some(&"repo") if args.get(1) == Some(&"view") => match self.is_private {
                    Some(true) => Ok(Self::make_output("true\n", true)),
                    Some(false) => Ok(Self::make_output("false\n", true)),
                    None => Ok(Self::make_output("offline", false)),
                },
                _ => Ok(Self::make_output("unexpected gh command", false)),
            }
        }

        fn run_gh_with_args(&self, args: &[std::ffi::OsString]) -> io::Result<Output> {
            let args_str: Vec<&str> = args.iter().filter_map(|s| s.to_str()).collect();
            match args_str.first() {
                Some(&"pr") if args_str.get(1) == Some(&"create") => {
                    match &self.gh_pr_create_result {
                        Some(url) => Ok(Self::make_output(&format!("{url}\n"), true)),
                        None => Ok(Self::make_output("gh pr create failed", false)),
                    }
                }
                _ => self.run_gh(&args_str),
            }
        }

        fn open_in_browser(&self, _repo: &str, _pr_url: &str) {
            // No-op in tests
        }
    }
}
