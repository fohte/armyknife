#[cfg(target_os = "linux")]
use std::fs;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessInfo {
    pub(crate) parent_pid: u32,
    pub(crate) comm: String,
}

/// Reads a process's parent PID and command name without spawning a process.
pub(crate) fn get_process_info(pid: u32) -> Option<ProcessInfo> {
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        parse_linux_process_stat(pid, &stat)
    }

    #[cfg(target_os = "macos")]
    {
        let pid = libc::pid_t::try_from(pid).ok()?;
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        // SAFETY: `info` points to writable storage sized for `proc_bsdinfo`.
        let size = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int,
            )
        };
        if size != std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int {
            return None;
        }

        // SAFETY: `proc_pidinfo` filled the entire struct as checked above.
        let info = unsafe { info.assume_init() };
        let parent_pid = info.pbi_ppid;
        let comm = info
            .pbi_comm
            .iter()
            .take_while(|&&byte| byte != 0)
            .map(|&byte| byte as u8)
            .collect::<Vec<_>>();
        Some(ProcessInfo {
            parent_pid,
            comm: String::from_utf8_lossy(&comm).into_owned(),
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

/// Looks up the parent PID of `pid` without spawning a process.
pub fn get_parent_pid(pid: u32) -> Option<u32> {
    get_process_info(pid).map(|info| info.parent_pid)
}

#[cfg(target_os = "linux")]
fn parse_linux_process_stat(pid: u32, stat: &str) -> Option<ProcessInfo> {
    let command_start = stat.find('(')?;
    let command_end = stat.rfind(')')?;
    if command_end <= command_start {
        return None;
    }

    let stat_pid = stat[..command_start].trim().parse::<u32>().ok()?;
    if stat_pid != pid {
        return None;
    }

    let parent_pid = stat
        .get(command_end + 1..)?
        .split_whitespace()
        .nth(1)?
        .parse::<u32>()
        .ok()?;
    Some(ProcessInfo {
        parent_pid,
        comm: stat.get(command_start + 1..command_end)?.to_string(),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn get_parent_pid_reads_the_current_process_parent() {
        // SAFETY: `getppid` has no preconditions and does not access memory through pointers.
        let parent_pid = unsafe { libc::getppid() };
        assert_eq!(
            super::get_parent_pid(std::process::id()),
            u32::try_from(parent_pid).ok(),
        );
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::{ProcessInfo, parse_linux_process_stat};
    use rstest::rstest;

    #[rstest]
    #[case::simple_command("42 (claude) S 7 8", 42, Some((7, "claude")))]
    #[case::command_with_spaces_and_parentheses(
        "42 (worker (nested) name) S 7 8",
        42,
        Some((7, "worker (nested) name")),
    )]
    #[case::different_pid("42 (claude) S 7 8", 43, None)]
    #[case::missing_parent("42 (claude)", 42, None)]
    fn parse_linux_process_stat_cases(
        #[case] stat: &str,
        #[case] pid: u32,
        #[case] expected: Option<(u32, &str)>,
    ) {
        assert_eq!(
            parse_linux_process_stat(pid, stat),
            expected.map(|(parent_pid, comm)| ProcessInfo {
                parent_pid,
                comm: comm.to_string(),
            }),
        );
    }
}
