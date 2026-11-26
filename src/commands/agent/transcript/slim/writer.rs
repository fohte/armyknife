#[cfg(not(test))]
use std::path::Path;

#[cfg(not(test))]
use anyhow::{Context, Result, bail};

#[cfg(all(not(test), target_os = "macos"))]
pub(super) fn has_writable_open_handle(path: &Path) -> Result<bool> {
    let output = crate::shared::command::new("lsof")
        .args(["-nP", "-F0pfa"])
        .arg(path)
        .output()
        .context("run lsof to inspect transcript writers")?;
    if !output.status.success() {
        bail!("lsof could not inspect transcript writers");
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
    for process in
        std::fs::read_dir("/proc").context("list processes for transcript writer check")?
    {
        let process = process.context("read process entry")?;
        let Some(pid) = process
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let process_path = process.path();
        let process_metadata = match std::fs::metadata(&process_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("inspect process {pid} owner"));
            }
        };
        if process_metadata.uid() != current_uid {
            continue;
        }
        let fd_directory = process_path.join("fd");
        let file_descriptors = match std::fs::read_dir(&fd_directory) {
            Ok(file_descriptors) => file_descriptors,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect file descriptors for process {pid}"));
            }
        };
        for file_descriptor in file_descriptors {
            let file_descriptor = file_descriptor.context("read process file descriptor")?;
            let descriptor_path = file_descriptor.path();
            let descriptor_metadata = match std::fs::metadata(&descriptor_path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("inspect process {pid} file descriptor {descriptor_path:?}")
                    });
                }
            };
            if descriptor_metadata.dev() != target.dev()
                || descriptor_metadata.ino() != target.ino()
            {
                continue;
            }
            let Some(descriptor_id) = file_descriptor.file_name().to_str().map(str::to_owned)
            else {
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
            let flags = u32::from_str_radix(flags.trim(), 8)
                .context("parse process file descriptor flags")?;
            if flags & (libc::O_ACCMODE as u32) != libc::O_RDONLY as u32 {
                return Ok(true);
            }
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
    use rstest::rstest;

    use super::lsof_output_has_writable_handle;

    #[rstest]
    #[case::read_only(b"p123\0f4\0ar\0", false)]
    #[case::write_only(b"p123\0f4\0aw\0", true)]
    #[case::read_and_write(b"p123\0f4\0au\0", true)]
    fn detects_writable_open_handles_in_lsof_output(#[case] output: &[u8], #[case] expected: bool) {
        assert_eq!(lsof_output_has_writable_handle(output), expected);
    }
}
