#[cfg(target_os = "linux")]
use std::fs;

/// Returns an OS-provided process start token that distinguishes PID reuse.
pub(crate) fn process_start_time(pid: u32) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let command_end = stat.rfind(')')?;
        let start_ticks = stat
            .get(command_end + 1..)?
            .split_whitespace()
            .nth(19)?
            .parse::<u64>()
            .ok()?;
        Some(format!("linux:{start_ticks}"))
    }

    #[cfg(target_os = "macos")]
    {
        let pid = libc::c_int::try_from(pid).ok()?;
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
        Some(format!(
            "macos:{}:{}",
            info.pbi_start_tvsec, info.pbi_start_tvusec
        ))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

pub(crate) fn process_start_time_matches(pid: u32, expected: &str) -> bool {
    process_start_time(pid).as_deref() == Some(expected)
}
