use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use super::error::{HumanInTheLoopError, Result};

/// Manages document approval state using SHA256 hashes.
///
/// When a document is approved, a hash of its content is saved to a separate file.
/// This allows detecting if the document was modified after approval.
#[allow(dead_code)]
pub struct ApprovalManager {
    document_path: PathBuf,
    approve_path: PathBuf,
}

impl ApprovalManager {
    /// Create a new approval manager for a document.
    pub fn new(document_path: &Path) -> Self {
        Self {
            document_path: document_path.to_path_buf(),
            approve_path: document_path.with_extension("md.approve"),
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

    /// Verify that the document hasn't been modified since approval.
    pub fn verify(&self) -> Result<()> {
        if !self.exists() {
            return Err(HumanInTheLoopError::NotApproved);
        }

        let saved_hash = fs::read_to_string(&self.approve_path)?.trim().to_string();
        let current_hash = self.compute_hash()?;

        if saved_hash != current_hash {
            return Err(HumanInTheLoopError::ModifiedAfterApproval);
        }

        Ok(())
    }

    /// Remove the approval file.
    pub fn remove(&self) -> Result<()> {
        if self.exists() {
            fs::remove_file(&self.approve_path)?;
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
