use std::path::PathBuf;

/// Returns the user's home directory from the HOME environment variable.
/// Empty values are treated as unset.
pub fn home_dir() -> Option<PathBuf> {
    non_empty_env("HOME").map(PathBuf::from)
}

/// Returns the XDG cache directory (~/.cache or $XDG_CACHE_HOME).
/// Empty values are treated as unset per XDG Base Directory Specification.
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(xdg) = non_empty_env("XDG_CACHE_HOME") {
        return Some(PathBuf::from(xdg));
    }
    home_dir().map(|home| home.join(".cache"))
}

/// Returns the XDG config directory (~/.config or $XDG_CONFIG_HOME).
/// Empty values are treated as unset per XDG Base Directory Specification.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(xdg) = non_empty_env("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg));
    }
    home_dir().map(|home| home.join(".config"))
}

/// Returns the user's state directory for application data.
#[cfg(target_os = "macos")]
pub fn state_dir() -> Option<PathBuf> {
    home_dir().map(|home| home.join("Library").join("Application Support"))
}

/// Returns the user's state directory for application data.
#[cfg(not(target_os = "macos"))]
pub fn state_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME")
        && !path.is_empty()
    {
        return Some(PathBuf::from(path));
    }
    home_dir().map(|home| home.join(".local").join("state"))
}

/// Returns the value of an environment variable, treating empty strings as unset.
fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(target_os = "macos"))]
    use rstest::rstest;

    #[test]
    fn home_dir_returns_home_env() {
        temp_env::with_vars([("HOME", Some("/test/home"))], || {
            assert_eq!(home_dir(), Some(PathBuf::from("/test/home")));
        });
    }

    #[test]
    fn home_dir_treats_empty_as_unset() {
        temp_env::with_vars([("HOME", Some(""))], || {
            assert_eq!(home_dir(), None);
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

    #[test]
    fn cache_dir_treats_empty_xdg_as_unset() {
        temp_env::with_vars(
            [("XDG_CACHE_HOME", Some("")), ("HOME", Some("/test/home"))],
            || {
                assert_eq!(cache_dir(), Some(PathBuf::from("/test/home/.cache")));
            },
        );
    }

    #[test]
    fn config_dir_treats_empty_xdg_as_unset() {
        temp_env::with_vars(
            [("XDG_CONFIG_HOME", Some("")), ("HOME", Some("/test/home"))],
            || {
                assert_eq!(config_dir(), Some(PathBuf::from("/test/home/.config")));
            },
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn state_dir_uses_application_support_under_home() {
        temp_env::with_vars(
            [
                ("HOME", Some("/test/home")),
                ("XDG_STATE_HOME", Some("/custom/state")),
            ],
            || {
                assert_eq!(
                    state_dir(),
                    Some(PathBuf::from("/test/home/Library/Application Support")),
                );
            },
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[rstest]
    #[case::xdg_state_home(Some("/custom/state"), Some("/test/home"), Some("/custom/state"))]
    #[case::empty_xdg_falls_back(Some(""), Some("/test/home"), Some("/test/home/.local/state"))]
    #[case::home_fallback(None, Some("/test/home"), Some("/test/home/.local/state"))]
    #[case::no_directory(None, None, None)]
    fn state_dir_uses_xdg_or_home(
        #[case] xdg_state_home: Option<&str>,
        #[case] home: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        temp_env::with_vars([("XDG_STATE_HOME", xdg_state_home), ("HOME", home)], || {
            assert_eq!(state_dir(), expected.map(PathBuf::from));
        });
    }
}
