#[cfg(not(test))]
use std::path::Path;

#[cfg(not(test))]
use anyhow::{Context, Result};

#[cfg(all(not(test), not(any(target_os = "linux", target_os = "macos"))))]
use anyhow::bail;

#[cfg(target_os = "linux")]
fn same_uid_processes<F>(
    process_paths: impl IntoIterator<Item = std::path::PathBuf>,
    current_uid: u32,
    mut process_owner_uid: F,
) -> anyhow::Result<Vec<std::path::PathBuf>>
where
    F: FnMut(&std::path::Path) -> std::io::Result<u32>,
{
    let mut same_uid_processes = Vec::new();
    for process_path in process_paths {
        let owner_uid = match process_owner_uid(&process_path) {
            Ok(owner_uid) => owner_uid,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                continue;
            }
            Err(error) => return Err(anyhow::Error::from(error)),
        };
        if owner_uid == current_uid {
            same_uid_processes.push(process_path);
        }
    }
    Ok(same_uid_processes)
}

#[cfg(any(test, target_os = "linux"))]
fn process_fd_paths<F>(
    process_paths: impl IntoIterator<Item = std::path::PathBuf>,
    mut read_fd_paths: F,
) -> anyhow::Result<Vec<std::path::PathBuf>>
where
    F: FnMut(&std::path::Path) -> std::io::Result<Vec<std::path::PathBuf>>,
{
    let mut fd_paths = Vec::new();
    for process_path in process_paths {
        match read_fd_paths(&process_path) {
            Ok(paths) => fd_paths.extend(paths),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(anyhow::Error::from(error)
                    .context(format!("inspect file descriptors for {process_path:?}")));
            }
        }
    }
    Ok(fd_paths)
}

#[cfg(all(not(test), target_os = "macos"))]
pub(super) fn has_writable_open_handle(path: &Path) -> Result<bool> {
    let output = crate::shared::command::new("lsof")
        .args(["-nP", "-F0pfa"])
        .arg(path)
        .output()
        .context("run lsof to inspect transcript writers")?;
    if !output.status.success() {
        anyhow::bail!("lsof could not inspect transcript writers");
    }
    Ok(lsof_output_has_writable_handle(&output.stdout))
}

#[cfg(all(not(test), target_os = "linux"))]
pub(super) fn has_writable_open_handle(path: &Path) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let target = std::fs::metadata(path).context("inspect monitored transcript inode")?;
    let current_uid = std::fs::metadata("/proc/self")
        .context("inspect current process owner")?
        .uid();
    let process_paths = std::fs::read_dir("/proc")
        .context("list processes for transcript writer check")?
        .map(|process| process.map(|process| process.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .context("read process entries for transcript writer check")?;
    let process_paths = same_uid_processes(process_paths, current_uid, |process_path| {
        std::fs::metadata(process_path).map(|metadata| metadata.uid())
    })
    .context("inspect process owners for transcript writer check")?;
    let descriptor_paths = process_fd_paths(process_paths, |process_path| {
        std::fs::read_dir(process_path.join("fd"))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect()
    })?;
    for descriptor_path in descriptor_paths {
        let process_path = descriptor_path
            .parent()
            .and_then(std::path::Path::parent)
            .context("file descriptor path has no process directory")?;
        let Some(pid) = process_path
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let descriptor_metadata = match std::fs::metadata(&descriptor_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("inspect process {pid} file descriptor {descriptor_path:?}")
                });
            }
        };
        if descriptor_metadata.dev() != target.dev() || descriptor_metadata.ino() != target.ino() {
            continue;
        }
        let Some(descriptor_id) = descriptor_path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let descriptor_info =
            match std::fs::read_to_string(process_path.join("fdinfo").join(descriptor_id)) {
                Ok(descriptor_info) => descriptor_info,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("read process {pid} descriptor flags"));
                }
            };
        let flags = descriptor_info
            .lines()
            .find_map(|line| line.strip_prefix("flags:"))
            .context("process file descriptor has no flags field")?;
        let flags =
            u32::from_str_radix(flags.trim(), 8).context("parse process file descriptor flags")?;
        if flags & (libc::O_ACCMODE as u32) != libc::O_RDONLY as u32 {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(all(not(test), not(any(target_os = "macos", target_os = "linux"))))]
pub(super) fn has_writable_open_handle(_path: &Path) -> Result<bool> {
    bail!("transcript slimming requires macOS or Linux open-file inspection")
}

#[cfg(any(test, target_os = "macos"))]
fn lsof_output_has_writable_handle(output: &[u8]) -> bool {
    output
        .split(|byte| *byte == 0)
        .any(|field| field.starts_with(b"aw") || field.starts_with(b"au"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rstest::rstest;

    use super::{lsof_output_has_writable_handle, process_fd_paths};

    #[rstest]
    #[case::read_only(b"p123\0f4\0ar\0", false)]
    #[case::write_only(b"p123\0f4\0aw\0", true)]
    #[case::read_and_write(b"p123\0f4\0au\0", true)]
    fn detects_writable_open_handles_in_lsof_output(#[case] output: &[u8], #[case] expected: bool) {
        assert_eq!(lsof_output_has_writable_handle(output), expected);
    }

    #[test]
    fn continues_scanning_after_an_unrelated_process_fd_directory_permission_error() {
        let blocked_process = PathBuf::from("fixture-unrelated-ssh");
        let writer_process = PathBuf::from("fixture-transcript-writer");
        let expected = vec![writer_process.join("fd/7")];

        let result = process_fd_paths([blocked_process, writer_process], |process_path| {
            if process_path == std::path::Path::new("fixture-unrelated-ssh") {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "fixture fd directory is not readable",
                ))
            } else {
                Ok(vec![process_path.join("fd/7")])
            }
        })
        .map_err(|error| error.to_string());

        assert_eq!(result, Ok(expected));
    }
}
