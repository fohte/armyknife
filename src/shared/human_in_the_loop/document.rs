use lazy_regex::regex_captures;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fs;
use std::path::PathBuf;

use super::approval::ApprovalManager;
use super::error::{HumanInTheLoopError, Result};

/// Trait for frontmatter schemas.
///
/// Each use case (PR draft, issue comment, etc.) defines its own frontmatter
/// structure and implements this trait.
pub trait DocumentSchema: Serialize + DeserializeOwned + Clone + Default {
    /// Returns true if the document has been approved by the user.
    ///
    /// The meaning of "approved" depends on the use case:
    /// - PR draft: `steps.submit: true`
    /// - Issue comment: `action: submit`
    /// - etc.
    fn is_approved(&self) -> bool;
}

/// A document with frontmatter and body.
///
/// Generic over the frontmatter schema `S` to support different use cases.
#[derive(Debug, Clone)]
pub struct Document<S> {
    pub path: PathBuf,
    pub frontmatter: S,
}

impl<S: DocumentSchema> Document<S> {
    /// Load a document from a file path.
    pub fn from_path(path: PathBuf) -> Result<Self> {
        if !path.exists() {
            return Err(HumanInTheLoopError::FileNotFound(path));
        }

        let content = fs::read_to_string(&path)?;
        let (frontmatter, _body) = parse_frontmatter(&content)?;

        Ok(Self { path, frontmatter })
    }

    /// Get an ApprovalManager for this document.
    pub fn approval_manager(&self) -> ApprovalManager {
        ApprovalManager::new(&self.path)
    }

    /// Save the approval hash for this document.
    pub fn save_approval(&self) -> Result<()> {
        self.approval_manager().save()
    }

    /// Remove the approval file.
    pub fn remove_approval(&self) -> Result<()> {
        self.approval_manager().remove()
    }
}

/// Parse frontmatter from document content.
///
/// Returns the parsed frontmatter and the remaining body.
/// If no frontmatter is found, returns a default-constructed frontmatter and the entire content as body.
pub fn parse_frontmatter<S: DeserializeOwned + Default>(content: &str) -> Result<(S, String)> {
    if let Some((whole, yaml_str)) = regex_captures!(r"^---\n([\s\S]*?)\n---\n?", content) {
        let frontmatter: S = serde_yaml::from_str(yaml_str)?;
        let body = content[whole.len()..].to_string();
        Ok((frontmatter, body))
    } else {
        Ok((S::default(), content.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use serde::Deserialize;

    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
    struct TestSchema {
        #[serde(default)]
        title: String,
        #[serde(default)]
        approved: bool,
    }

    impl DocumentSchema for TestSchema {
        fn is_approved(&self) -> bool {
            self.approved
        }
    }

    #[test]
    fn test_parse_frontmatter_with_valid_yaml() {
        let content = indoc! {"
            ---
            title: Test
            approved: true
            ---
            Body content"};
        let (schema, body): (TestSchema, _) = parse_frontmatter(content).unwrap();
        assert_eq!(schema.title, "Test");
        assert!(schema.approved);
        assert_eq!(body, "Body content");
    }

    #[test]
    fn test_parse_frontmatter_without_frontmatter() {
        let content = "Just body content";
        let (schema, body): (TestSchema, _) = parse_frontmatter(content).unwrap();
        assert_eq!(schema, TestSchema::default());
        assert_eq!(body, "Just body content");
    }
}
