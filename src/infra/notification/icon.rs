use std::path::PathBuf;

use crate::shared::cache;

const ICON_BYTES: &[u8] = include_bytes!("../../../assets/claude-code-icon.png");

/// Returns the path to the notification icon, writing it to the cache directory if needed.
/// The filename includes a content hash so the cached file is replaced when the binary is
/// updated with a different icon.
/// Returns None if the cache directory is unavailable or the file cannot be written.
pub fn ensure_icon() -> Option<PathBuf> {
    let dir = cache::base_dir()?;
    // Use a truncated hash of the icon bytes to bust the cache on binary updates
    let hash = short_hash(ICON_BYTES);
    let filename = format!("claude-code-icon-{hash}.png");
    let path = dir.join(&filename);

    if path.exists() {
        return Some(path);
    }

    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(&path, ICON_BYTES).ok()?;

    Some(path)
}

/// Returns the first 8 hex characters of the SHA-256 digest.
fn short_hash(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(data);
    hex_encode(&digest[..4])
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
