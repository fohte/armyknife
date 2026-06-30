use indoc::formatdoc;
use lazy_regex::regex_is_match;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::infra::git;
use crate::infra::github::{self, RepoClient};
use crate::shared::human_in_the_loop::{ApprovalManager, DocumentSchema, HumanInTheLoopError};
use crate::shared::yaml_frontmatter;

#[derive(Error, Debug)]
pub enum PrDraftError {
    #[error("Git error: {0}")]
    Git(#[from] git::GitError),

    #[error("GitHub error: {0}")]
    GitHub(#[from] github::GitHubError),

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

pub type Result<T> = anyhow::Result<T>;

#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub owner: String,
    pub repo: String,
    pub branch: String,
    pub is_private: bool,
}

impl RepoInfo {
    /// Get repo info with is_private check via GitHub API (async)
    pub async fn from_current_dir_async(gh_client: &impl RepoClient) -> Result<Self> {
        let repo = git::open_repo()?;
        Self::from_repo_async(&repo, Some(gh_client)).await
    }

    /// Get repo info from git only (no network call, is_private defaults to false)
    pub fn from_git_only() -> Result<Self> {
        let repo = git::open_repo()?;
        let branch = git::current_branch(&repo)?;
        let (owner, repo_name) = git::github_owner_and_repo(&repo)?;
        Ok(Self {
            owner,
            repo: repo_name,
            branch,
            is_private: false,
        })
    }

    /// Get repo info from a repository at a specific path.
    /// Used for testing with temporary repositories.
    #[cfg(test)]
    pub fn from_path(path: &Path) -> Result<Self> {
        let repo = git::open_repo_at(path)?;
        let branch = git::current_branch(&repo)?;
        let (owner, repo_name) = git::github_owner_and_repo(&repo)?;
        Ok(Self {
            owner,
            repo: repo_name,
            branch,
            is_private: false,
        })
    }

    async fn from_repo_async(
        repo: &git::GitRepo,
        gh_client: Option<&impl RepoClient>,
    ) -> Result<Self> {
        let branch = git::current_branch(repo)?;
        let (owner, repo_name) = git::github_owner_and_repo(repo)?;
        let is_private = match gh_client {
            Some(client) => client.is_repo_private(&owner, &repo_name).await?,
            None => false,
        };

        Ok(Self {
            owner,
            repo: repo_name,
            branch,
            is_private,
        })
    }
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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Steps {
    #[serde(default, rename = "ready-for-translation")]
    pub ready_for_translation: bool,
    #[serde(default)]
    pub submit: bool,
}

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
            return Err(PrDraftError::FileNotFound(path).into());
        }

        let content = fs::read_to_string(&path)?;
        let (frontmatter, body) = parse_frontmatter(&content)?;

        Ok(Self {
            path,
            frontmatter,
            body,
        })
    }

    fn approval_manager(&self) -> ApprovalManager {
        ApprovalManager::new(&self.path)
    }

    #[cfg(test)]
    pub fn save_approval(&self) -> Result<()> {
        self.approval_manager().save()?;
        Ok(())
    }

    pub fn remove_approval(&self) -> Result<()> {
        self.approval_manager().remove()?;
        Ok(())
    }

    pub fn verify_approval(&self) -> Result<()> {
        match self.approval_manager().verify() {
            Ok(()) => Ok(()),
            Err(HumanInTheLoopError::NotApproved) => Err(PrDraftError::NotApproved.into()),
            Err(HumanInTheLoopError::ModifiedAfterApproval) => {
                Err(PrDraftError::ModifiedAfterApproval.into())
            }
            Err(other) => Err(other.into()),
        }
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

    /// If the on-disk draft has any approval gesture flag (`steps.submit` or
    /// `steps.ready-for-translation`) set to `true`, rewrite it to `false`
    /// and refresh the in-memory state. Approval must come from the user
    /// flipping the flag inside the editor, so any pre-existing `true` (e.g.
    /// left over from a previous draft generation) would short-circuit that
    /// approval gesture.
    pub fn reset_approval_flags(&mut self) -> Result<bool> {
        if !self.frontmatter.steps.submit && !self.frontmatter.steps.ready_for_translation {
            return Ok(false);
        }
        let content = fs::read_to_string(&self.path)?;
        let new_content =
            yaml_frontmatter::reset_bool_fields(&content, &["submit", "ready-for-translation"]);
        if new_content == content {
            // Frontmatter says true but the regex did not match; bail rather
            // than silently leave the file inconsistent.
            return Err(PrDraftError::CommandFailed(format!(
                "Failed to reset steps.submit / steps.ready-for-translation in frontmatter at {}. \
                 Please open the file and set them to `false` manually.",
                self.path.display()
            ))
            .into());
        }
        fs::write(&self.path, &new_content)?;
        self.frontmatter.steps.submit = false;
        self.frontmatter.steps.ready_for_translation = false;
        Ok(true)
    }
}

fn parse_frontmatter(content: &str) -> Result<(Frontmatter, String)> {
    if let Some((_whole, yaml_str, body_offset)) = yaml_frontmatter::split_frontmatter(content) {
        let frontmatter: Frontmatter = serde_yaml::from_str(yaml_str)?;
        let body = content[body_offset..].to_string();
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

/// Check if the repo's config language allows Japanese content.
pub fn repo_allows_japanese(owner: &str, repo: &str) -> bool {
    let Ok(config) = crate::shared::config::load_config() else {
        return false;
    };
    let repo_id = format!("{owner}/{repo}");
    config
        .repos
        .get(&repo_id)
        .and_then(|r| r.language.as_deref())
        == Some("ja")
}

pub fn contains_japanese(text: &str) -> bool {
    regex_is_match!(r"[\p{Hiragana}\p{Katakana}\p{Han}]", text)
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
    #[case::private(
        true,
        indoc! {"
            ---
            title: Test Title
            steps:
              submit: false
            ---
        "}
    )]
    #[case::public(
        false,
        indoc! {"
            ---
            title: Test Title
            steps:
              ready-for-translation: false
              submit: false
            ---
        "}
    )]
    fn test_generate_frontmatter(#[case] is_private: bool, #[case] expected: &str) {
        let result = generate_frontmatter("Test Title", is_private);
        assert_eq!(result, expected);
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

    #[rstest]
    #[case::resets_submit_when_true(
        indoc! {"
            ---
            title: T
            steps:
              submit: true
            ---
            body
        "},
        true,
        indoc! {"
            ---
            title: T
            steps:
              submit: false
            ---
            body
        "}
    )]
    #[case::resets_ready_for_translation_when_true(
        indoc! {"
            ---
            title: T
            steps:
              ready-for-translation: true
              submit: false
            ---
            body
        "},
        true,
        indoc! {"
            ---
            title: T
            steps:
              ready-for-translation: false
              submit: false
            ---
            body
        "}
    )]
    #[case::noop_when_both_false(
        indoc! {"
            ---
            title: T
            steps:
              submit: false
            ---
            body
        "},
        false,
        indoc! {"
            ---
            title: T
            steps:
              submit: false
            ---
            body
        "}
    )]
    fn test_draft_file_reset_approval_flags(
        #[case] initial: &str,
        #[case] expected_changed: bool,
        #[case] expected_content: &str,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("draft.md");
        fs::write(&path, initial).expect("write initial");

        let mut draft = DraftFile::from_path(path.clone()).expect("parse draft");
        let changed = draft.reset_approval_flags().expect("reset");

        assert_eq!(
            (
                changed,
                draft.frontmatter.steps.submit,
                draft.frontmatter.steps.ready_for_translation,
                fs::read_to_string(&path).expect("read back"),
            ),
            (expected_changed, false, false, expected_content.to_string(),),
        );
    }
}
