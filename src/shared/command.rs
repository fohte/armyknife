use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

/// Process-lifetime cache mapping a command name to its resolved PATH location.
///
/// `PATH` is assumed stable for the lifetime of the process, which holds for a
/// short-lived CLI invocation.
fn resolution_cache() -> &'static Mutex<HashMap<String, Option<PathBuf>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Builds a [`Command`] for `program`, resolving the program name to an
/// absolute path via a cached PATH lookup.
///
/// `Command::new("tmux")` makes the OS scan `PATH` on every spawn; passing an
/// already-resolved absolute path skips that repeated work. A `program` that
/// contains a path separator, or that cannot be found in PATH, is handed to
/// [`Command::new`] unchanged.
pub fn new(program: impl AsRef<OsStr>) -> Command {
    Command::new(resolve(program))
}

/// Resolves `program` to an absolute path using the cached PATH lookup.
///
/// Returns `program` unchanged when it already contains a path separator (the
/// OS does not PATH-search such names) or when it is not found in PATH.
pub fn resolve(program: impl AsRef<OsStr>) -> OsString {
    let program = program.as_ref();
    match program.to_str() {
        Some(name) if !name.contains('/') => {
            find_command_path(name).map_or_else(|| program.to_os_string(), PathBuf::into_os_string)
        }
        // A name with a path separator, or a non-UTF8 name that cannot key the
        // cache, is used as given.
        _ => program.to_os_string(),
    }
}

/// Check if a command is available in PATH.
pub fn is_command_available(cmd: &str) -> bool {
    find_command_path(cmd).is_some()
}

/// Find the full path of a command in PATH.
/// Returns the first matching executable path, or None if not found.
///
/// The result is cached for the process lifetime, so repeated lookups of the
/// same command skip re-scanning every PATH entry.
pub fn find_command_path(cmd: &str) -> Option<PathBuf> {
    if let Some(cached) = resolution_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(cmd)
    {
        return cached.clone();
    }

    // Run the filesystem scan without holding the cache lock.
    let resolved = scan_path_for_command(cmd);
    resolution_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(cmd.to_string(), resolved.clone());
    resolved
}

/// Scans every `PATH` entry for an executable named `cmd` (uncached).
fn scan_path_for_command(cmd: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;

    std::env::split_paths(&path_var)
        .map(|dir| dir.join(cmd))
        .find(|path| is_executable(path))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::existing("sh", true)]
    #[case::nonexistent("definitely-not-a-real-command-12345", false)]
    fn is_command_available_cases(#[case] cmd: &str, #[case] expected: bool) {
        assert_eq!(is_command_available(cmd), expected);
    }

    #[test]
    fn find_command_path_returns_absolute_path_for_existing_command() {
        let path =
            find_command_path("sh").expect("'sh' command not found in PATH, required for test");
        assert!(path.is_absolute());
        assert!(path.ends_with("sh"));
    }

    #[test]
    fn find_command_path_returns_none_for_nonexistent_command() {
        assert!(find_command_path("definitely-not-a-real-command-12345").is_none());
    }

    #[rstest]
    #[case::path_with_separator_is_unchanged("/usr/bin/env")]
    #[case::nonexistent_name_is_unchanged("definitely-not-a-real-command-12345")]
    fn resolve_returns_input_unchanged(#[case] input: &str) {
        assert_eq!(resolve(input), OsString::from(input));
    }

    #[test]
    fn resolve_returns_absolute_path_for_existing_command() {
        assert_eq!(Some(PathBuf::from(resolve("sh"))), find_command_path("sh"));
    }

    #[test]
    fn new_builds_command_with_resolved_program() {
        let cmd = new("sh");
        assert_eq!(cmd.get_program(), resolve("sh").as_os_str());
    }
}
