use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use super::error::Result;

/// Manages document approval state using SHA256 hashes.
///
/// When a document is approved, a hash of its content is saved to a separate file.
/// This allows detecting if the document was modified after approval.
pub struct ApprovalManager {
    document_path: PathBuf,
    approve_path: PathBuf,
}

impl ApprovalManager {
    /// Create a new approval manager for a document.
    ///
    /// The approve file is the document path with `.approve` appended
    /// (e.g., `issue.md` → `issue.md.approve`, `metadata.json` → `metadata.json.approve`).
    pub fn new(document_path: &Path) -> Self {
        let mut approve_name = document_path.as_os_str().to_os_string();
        approve_name.push(".approve");
        Self {
            document_path: document_path.to_path_buf(),
            approve_path: PathBuf::from(approve_name),
        }
    }

    /// Check if an approval file exists.
    pub fn exists(&self) -> bool {
        self.approve_path.exists()
    }

    /// Save the approval hash for a document.
    pub fn save(&self) -> Result<()> {
        let hash = self.compute_hash()?;
        fs::write(&self.approve_path, hash)?;
        Ok(())
    }

    /// Remove the approval file.
    pub fn remove(&self) -> Result<()> {
        if self.exists() {
            fs::remove_file(&self.approve_path)?;
        }
        Ok(())
    }

    /// Verify that the document has been approved and not modified since.
    pub fn verify(&self) -> Result<()> {
        if !self.exists() {
            return Err(super::error::HumanInTheLoopError::NotApproved);
        }

        let saved_hash = fs::read_to_string(&self.approve_path)?.trim().to_string();
        let current_hash = self.compute_hash()?;

        if saved_hash != current_hash {
            return Err(super::error::HumanInTheLoopError::ModifiedAfterApproval);
        }

        Ok(())
    }

    fn compute_hash(&self) -> std::io::Result<String> {
        let content = fs::read_to_string(&self.document_path)?;
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        Ok(format!("{:x}", hasher.finalize()))
    }
}
