use std::path::PathBuf;

use crate::shared::cache;

const ICON_BYTES: &[u8] = include_bytes!("../../../assets/claude-code-icon.png");
const ICON_FILENAME: &str = "claude-code-icon.png";

/// Returns the path to the notification icon, writing it to the cache directory if needed.
/// Returns None if the cache directory is unavailable or the file cannot be written.
pub fn ensure_icon() -> Option<PathBuf> {
    let dir = cache::base_dir()?;
    let path = dir.join(ICON_FILENAME);

    if path.exists() {
        return Some(path);
    }

    std::fs::create_dir_all(&dir).ok()?;
    std::fs::write(&path, ICON_BYTES).ok()?;

    Some(path)
}
