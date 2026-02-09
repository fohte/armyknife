use std::path::PathBuf;

/// Returns the user's home directory from the HOME environment variable.
pub fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// Returns the XDG cache directory (~/.cache or $XDG_CACHE_HOME).
pub fn cache_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        return Some(PathBuf::from(xdg));
    }
    home_dir().map(|home| home.join(".cache"))
}

/// Returns the XDG config directory (~/.config or $XDG_CONFIG_HOME).
pub fn config_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg));
    }
    home_dir().map(|home| home.join(".config"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_dir_returns_home_env() {
        temp_env::with_vars([("HOME", Some("/test/home"))], || {
            assert_eq!(home_dir(), Some(PathBuf::from("/test/home")));
        });
    }

    #[test]
    fn cache_dir_uses_xdg_cache_home_when_set() {
        temp_env::with_vars([("XDG_CACHE_HOME", Some("/custom/cache"))], || {
            assert_eq!(cache_dir(), Some(PathBuf::from("/custom/cache")));
        });
    }

    #[test]
    fn cache_dir_falls_back_to_home_dot_cache() {
        temp_env::with_vars(
            [
                ("XDG_CACHE_HOME", None::<&str>),
                ("HOME", Some("/test/home")),
            ],
            || {
                assert_eq!(cache_dir(), Some(PathBuf::from("/test/home/.cache")));
            },
        );
    }

    #[test]
    fn config_dir_uses_xdg_config_home_when_set() {
        temp_env::with_vars([("XDG_CONFIG_HOME", Some("/custom/config"))], || {
            assert_eq!(config_dir(), Some(PathBuf::from("/custom/config")));
        });
    }

    #[test]
    fn config_dir_falls_back_to_home_dot_config() {
        temp_env::with_vars(
            [
                ("XDG_CONFIG_HOME", None::<&str>),
                ("HOME", Some("/test/home")),
            ],
            || {
                assert_eq!(config_dir(), Some(PathBuf::from("/test/home/.config")));
            },
        );
    }
}
