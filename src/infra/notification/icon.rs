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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_hash_returns_8_hex_chars() {
        let hash = short_hash(b"hello");
        assert_eq!(hash.len(), 8);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn short_hash_is_deterministic() {
        assert_eq!(short_hash(b"test"), short_hash(b"test"));
    }

    #[test]
    fn short_hash_differs_for_different_input() {
        assert_ne!(short_hash(b"aaa"), short_hash(b"bbb"));
    }

    #[test]
    fn hex_encode_produces_lowercase_hex() {
        assert_eq!(hex_encode(&[0x00, 0xff, 0x0a]), "00ff0a");
    }

    #[test]
    fn ensure_icon_writes_file_to_cache_dir() {
        let dir = tempfile::tempdir().unwrap();
        temp_env::with_vars(
            [
                ("XDG_CACHE_HOME", Some(dir.path().to_str().unwrap())),
                ("HOME", Some("/nonexistent")),
            ],
            || {
                let path = ensure_icon().unwrap();
                assert!(path.exists());
                assert!(
                    path.file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .starts_with("claude-code-icon-")
                );
                assert!(
                    path.file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .ends_with(".png")
                );

                // Calling again returns the same cached path
                let path2 = ensure_icon().unwrap();
                assert_eq!(path, path2);
            },
        );
    }

    #[test]
    fn ensure_icon_returns_none_without_cache_dir() {
        temp_env::with_vars(
            [("XDG_CACHE_HOME", None::<&str>), ("HOME", None::<&str>)],
            || {
                assert!(ensure_icon().is_none());
            },
        );
    }
}
