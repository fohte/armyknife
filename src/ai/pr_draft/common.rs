use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PrDraftError {
    #[error("Not in a git repository")]
    NotInGitRepo,

    #[error("Failed to get repository info: {0}")]
    RepoInfoError(String),

    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[allow(dead_code)]
    #[error("Editor is already open for this file")]
    EditorAlreadyOpen,

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
    pub fn from_current_dir() -> Result<Self> {
        let branch = get_current_branch()?;
        let (owner, repo) = get_repo_owner_and_name()?;
        let is_private = check_is_private(&owner, &repo)?;

        Ok(Self {
            owner,
            repo,
            branch,
            is_private,
        })
    }
}

fn get_current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .output()
        .map_err(|e| PrDraftError::RepoInfoError(e.to_string()))?;

    if !output.status.success() {
        return Err(PrDraftError::NotInGitRepo);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn get_repo_owner_and_name() -> Result<(String, String)> {
    let output = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "owner,name",
            "-q",
            ".owner.login + \"/\" + .name",
        ])
        .output()
        .map_err(|e| PrDraftError::RepoInfoError(e.to_string()))?;

    if !output.status.success() {
        return Err(PrDraftError::RepoInfoError(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    let full_name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let parts: Vec<&str> = full_name.split('/').collect();

    if parts.len() != 2 {
        return Err(PrDraftError::RepoInfoError(format!(
            "Unexpected repo format: {}",
            full_name
        )));
    }

    Ok((parts[0].to_string(), parts[1].to_string()))
}

fn check_is_private(owner: &str, repo: &str) -> Result<bool> {
    let output = Command::new("gh")
        .args([
            "repo",
            "view",
            &format!("{}/{}", owner, repo),
            "--json",
            "isPrivate",
            "-q",
            ".isPrivate",
        ])
        .output()
        .map_err(|e| PrDraftError::RepoInfoError(e.to_string()))?;

    if !output.status.success() {
        return Err(PrDraftError::RepoInfoError(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    let is_private_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(is_private_str == "true")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub steps: Steps,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
        PathBuf::from("/tmp/pr-body-draft")
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

    pub fn compute_hash(&self) -> String {
        let content = fs::read_to_string(&self.path).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    #[allow(dead_code)]
    pub fn save_approval(&self) -> Result<()> {
        let hash = self.compute_hash();
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
        let current_hash = self.compute_hash();

        if saved_hash != current_hash {
            return Err(PrDraftError::ModifiedAfterApproval);
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub fn is_locked(&self) -> bool {
        Self::lock_path(&self.path).exists()
    }

    #[allow(dead_code)]
    pub fn create_lock(&self) -> Result<()> {
        fs::write(Self::lock_path(&self.path), "")?;
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
    let frontmatter_regex = Regex::new(r"^---\n([\s\S]*?)\n---\n?").unwrap();

    if let Some(captures) = frontmatter_regex.captures(content) {
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
    if is_private {
        format!(
            r#"---
title: "{}"
steps:
  submit: false
---
"#,
            title
        )
    } else {
        format!(
            r#"---
title: "{}"
steps:
  ready-for-translation: false
  submit: false
---
"#,
            title
        )
    }
}

pub fn contains_japanese(text: &str) -> bool {
    let japanese_regex = Regex::new(r"[\p{Hiragana}\p{Katakana}\p{Han}]").unwrap();
    japanese_regex.is_match(text)
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

    #[test]
    fn test_parse_frontmatter() {
        let content = r#"---
title: "Test PR"
steps:
  submit: true
---
This is the body"#;

        let (frontmatter, body) = parse_frontmatter(content).unwrap();
        assert_eq!(frontmatter.title, "Test PR");
        assert!(frontmatter.steps.submit);
        assert_eq!(body, "This is the body");
    }

    #[test]
    fn test_parse_frontmatter_with_ready_for_translation() {
        let content = r#"---
title: "Public PR"
steps:
  ready-for-translation: true
  submit: false
---
Body content"#;

        let (frontmatter, body) = parse_frontmatter(content).unwrap();
        assert_eq!(frontmatter.title, "Public PR");
        assert!(frontmatter.steps.ready_for_translation);
        assert!(!frontmatter.steps.submit);
        assert_eq!(body, "Body content");
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "Just a body without frontmatter";

        let (frontmatter, body) = parse_frontmatter(content).unwrap();
        assert_eq!(frontmatter.title, "");
        assert!(!frontmatter.steps.submit);
        assert_eq!(body, content);
    }

    #[test]
    fn test_contains_japanese() {
        assert!(contains_japanese("これはテスト"));
        assert!(contains_japanese("Hello 世界"));
        assert!(contains_japanese("カタカナ"));
        assert!(!contains_japanese("Hello World"));
        assert!(!contains_japanese("abc123!@#"));
    }

    #[test]
    fn test_generate_frontmatter_private() {
        let result = generate_frontmatter("Test Title", true);
        assert!(result.contains("title: \"Test Title\""));
        assert!(result.contains("submit: false"));
        assert!(!result.contains("ready-for-translation"));
    }

    #[test]
    fn test_generate_frontmatter_public() {
        let result = generate_frontmatter("Test Title", false);
        assert!(result.contains("title: \"Test Title\""));
        assert!(result.contains("submit: false"));
        assert!(result.contains("ready-for-translation: false"));
    }
}
