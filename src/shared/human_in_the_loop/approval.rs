use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

use super::error::{HumanInTheLoopError, Result};

/// HMAC key authenticating approval records.
/// Changing this value invalidates every approval already on disk.
const APPROVAL_HMAC_KEY: [u8; 32] = [
    0xf3, 0x27, 0x88, 0xff, 0xa4, 0xe6, 0x22, 0xb2, 0x91, 0xb2, 0xdb, 0xb5, 0x67, 0x62, 0xb8, 0x1d,
    0xd4, 0xf1, 0x00, 0xbd, 0x54, 0x33, 0x05, 0x72, 0x92, 0x20, 0xa9, 0xbe, 0x9a, 0xfd, 0x7c, 0xab,
];

/// Domain separator mixed into the filename derivation so that the same
/// key reused for content authentication cannot collide with filename
/// derivation.
const APPROVAL_ID_DOMAIN: &[u8] = b"approval-id-v1\n";

/// Domain separator mixed into the body MAC so that filename and body
/// derivations live in disjoint input spaces.
const APPROVAL_MAC_DOMAIN: &[u8] = b"approval-mac-v1\n";

/// Environment variable that overrides the approval directory. Read
/// only; this code never calls `setenv`, so concurrent readers are safe.
const APPROVAL_DIR_ENV: &str = "ARMYKNIFE_APPROVAL_DIR";

/// Process-wide test override. Initialised once by
/// `shared::testing::init_approval_dir` before any test interacts with
/// `ApprovalManager`; never written from production code.
#[doc(hidden)]
pub static TEST_APPROVAL_DIR_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Manages document approval state.
///
/// The approval record lives under the user's state directory keyed by
/// an opaque, key-derived id, not next to the document. Callers must
/// pass the same `document_path` representation to `save` and `verify`;
/// the id is computed from path bytes verbatim.
pub struct ApprovalManager {
    approve_path: PathBuf,
    document_path: PathBuf,
}

impl ApprovalManager {
    pub fn new(document_path: &Path) -> Self {
        let id = derive_approval_id(document_path);
        let approve_path = approvals_dir().join(id);
        Self {
            approve_path,
            document_path: document_path.to_path_buf(),
        }
    }

    #[cfg(test)]
    pub fn approve_path(&self) -> &Path {
        &self.approve_path
    }

    pub fn exists(&self) -> bool {
        self.approve_path.exists()
    }

    pub fn save(&self) -> Result<()> {
        let mac = self.compute_mac()?;
        let dir = self
            .approve_path
            .parent()
            .ok_or_else(|| std::io::Error::other("approval path has no parent"))?;
        create_dir_secure(dir)?;
        write_file_secure(&self.approve_path, mac.as_bytes())?;
        Ok(())
    }

    pub fn remove(&self) -> Result<()> {
        if self.exists() {
            fs::remove_file(&self.approve_path)?;
        }
        Ok(())
    }

    pub fn verify(&self) -> Result<()> {
        if !self.exists() {
            return Err(HumanInTheLoopError::NotApproved);
        }
        let saved = fs::read_to_string(&self.approve_path)?.trim().to_string();
        let current = self.compute_mac()?;
        if !constant_time_eq(saved.as_bytes(), current.as_bytes()) {
            return Err(HumanInTheLoopError::ModifiedAfterApproval);
        }
        Ok(())
    }

    fn compute_mac(&self) -> std::io::Result<String> {
        let content = fs::read(&self.document_path)?;
        let mut input = Vec::with_capacity(APPROVAL_MAC_DOMAIN.len() + content.len());
        input.extend_from_slice(APPROVAL_MAC_DOMAIN);
        input.extend_from_slice(&content);
        Ok(crate::shared::hex::encode(&hmac_sha256(
            &APPROVAL_HMAC_KEY,
            &input,
        )))
    }
}

/// Compute the opaque on-disk identifier for a document path.
fn derive_approval_id(document_path: &Path) -> String {
    let path_bytes = path_bytes(document_path);
    let mut input = Vec::with_capacity(APPROVAL_ID_DOMAIN.len() + path_bytes.len());
    input.extend_from_slice(APPROVAL_ID_DOMAIN);
    input.extend_from_slice(path_bytes);
    crate::shared::hex::encode(&hmac_sha256(&APPROVAL_HMAC_KEY, &input))
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> &[u8] {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> &[u8] {
    path.to_str().unwrap_or("").as_bytes()
}

/// HMAC-SHA256(K, M).
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut padded = [0u8; BLOCK];
    if key.len() > BLOCK {
        let mut hasher = Sha256::new();
        hasher.update(key);
        let digest = hasher.finalize();
        padded[..32].copy_from_slice(&digest);
    } else {
        padded[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0u8; BLOCK];
    let mut opad = [0u8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] = padded[i] ^ 0x36;
        opad[i] = padded[i] ^ 0x5c;
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let final_digest = outer.finalize();

    let mut out = [0u8; 32];
    out.copy_from_slice(&final_digest);
    out
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

fn approvals_dir() -> PathBuf {
    if let Some(dir) = TEST_APPROVAL_DIR_OVERRIDE.get() {
        return dir.clone();
    }
    if let Some(v) = std::env::var_os(APPROVAL_DIR_ENV)
        && !v.is_empty()
    {
        return PathBuf::from(v);
    }
    state_dir().join("armyknife").join("approvals")
}

#[cfg(target_os = "macos")]
fn state_dir() -> PathBuf {
    if let Some(home) = crate::shared::dirs::home_dir() {
        return home.join("Library").join("Application Support");
    }
    PathBuf::from("/tmp")
}

#[cfg(not(target_os = "macos"))]
fn state_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME")
        && !xdg.is_empty()
    {
        return PathBuf::from(xdg);
    }
    if let Some(home) = crate::shared::dirs::home_dir() {
        return home.join(".local").join("state");
    }
    PathBuf::from("/tmp")
}

#[cfg(unix)]
fn create_dir_secure(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        return Ok(());
    }
    // Tighten loose permissions left by a prior process. `mkdir` honours
    // umask, so a pre-existing approvals dir is not guaranteed to be 0700.
    let meta = std::fs::metadata(dir)?;
    let mut perms = meta.permissions();
    if perms.mode() & 0o077 != 0 {
        perms.set_mode(0o700);
        std::fs::set_permissions(dir, perms)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn create_dir_secure(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(unix)]
fn write_file_secure(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents)?;

    // `OpenOptions::mode` only applies when the file is created; an
    // existing file keeps its old permissions. Force 0600 after writing
    // so reused approval records are never world-readable.
    let mut perms = file.metadata()?.permissions();
    if perms.mode() & 0o077 != 0 {
        perms.set_mode(0o600);
        file.set_permissions(perms)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn write_file_secure(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::init_approval_dir;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    struct Env {
        docs_dir: TempDir,
    }

    impl Env {
        fn doc(&self, name: &str, body: &str) -> PathBuf {
            let p = self.docs_dir.path().join(name);
            std::fs::write(&p, body).expect("write doc");
            p
        }
    }

    #[fixture]
    fn env() -> Env {
        init_approval_dir();
        Env {
            docs_dir: TempDir::new().expect("tempdir"),
        }
    }

    #[rstest]
    fn save_then_verify_succeeds(env: Env) {
        let path = env.doc("a.md", "hello");
        let mgr = ApprovalManager::new(&path);
        mgr.save().expect("save");
        mgr.verify().expect("verify");
    }

    #[rstest]
    fn verify_fails_when_missing(env: Env) {
        let path = env.doc("a.md", "hello");
        let mgr = ApprovalManager::new(&path);
        let err = mgr.verify().expect_err("should be NotApproved");
        assert!(matches!(err, HumanInTheLoopError::NotApproved));
    }

    #[rstest]
    fn verify_detects_tampering(env: Env) {
        let path = env.doc("a.md", "hello");
        let mgr = ApprovalManager::new(&path);
        mgr.save().expect("save");
        std::fs::write(&path, "tampered").expect("rewrite");
        let err = mgr.verify().expect_err("should be ModifiedAfterApproval");
        assert!(matches!(err, HumanInTheLoopError::ModifiedAfterApproval));
    }

    #[rstest]
    fn approval_file_does_not_appear_in_document_dir(env: Env) {
        let path = env.doc("a.md", "hello");
        ApprovalManager::new(&path).save().expect("save");

        let mut entries: Vec<String> = std::fs::read_dir(env.docs_dir.path())
            .expect("read dir")
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        entries.sort();
        assert_eq!(entries, vec!["a.md".to_string()]);
    }

    #[rstest]
    fn approval_filename_is_opaque(env: Env) {
        let path = env.doc("a.md", "hello");
        let mgr = ApprovalManager::new(&path);
        mgr.save().expect("save");

        let name = mgr
            .approve_path()
            .file_name()
            .and_then(|s| s.to_str())
            .expect("filename");
        assert_eq!(name.len(), 64);
        assert!(name.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!name.contains(".approve"));
        assert!(!name.contains("a.md"));
        // Approval file must not sit next to the document.
        assert!(
            mgr.approve_path().parent() != Some(env.docs_dir.path()),
            "approval file should not live in the document directory"
        );
    }

    #[rstest]
    fn different_paths_yield_different_ids(env: Env) {
        let a = env.doc("a.md", "same");
        let b = env.doc("b.md", "same");
        let ma = ApprovalManager::new(&a);
        let mb = ApprovalManager::new(&b);
        assert_ne!(ma.approve_path(), mb.approve_path());
    }

    #[rstest]
    fn remove_is_idempotent(env: Env) {
        let path = env.doc("a.md", "hello");
        let mgr = ApprovalManager::new(&path);
        mgr.remove().expect("remove (absent)");
        mgr.save().expect("save");
        mgr.remove().expect("remove (present)");
        assert!(!mgr.exists());
    }

    #[rstest]
    #[case::rfc4231_test_case_1(
        &[0x0b; 20],
        b"Hi There".as_slice(),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
    )]
    #[case::empty_key_empty_msg(
        b"".as_slice(),
        b"".as_slice(),
        "b613679a0814d9ec772f95d778c35fc5ff1697c493715653c6c712144292c5ad",
    )]
    fn hmac_sha256_matches_known_vectors(
        #[case] key: &[u8],
        #[case] msg: &[u8],
        #[case] expected_hex: &str,
    ) {
        let mac = hmac_sha256(key, msg);
        assert_eq!(crate::shared::hex::encode(&mac), expected_hex);
    }
}
