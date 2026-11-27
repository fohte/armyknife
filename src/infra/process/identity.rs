#[cfg(target_os = "linux")]
use std::fs;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ProcessIdentity {
    pub(super) parent_pid: u32,
    pub(super) start_time: ProcessStartTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ProcessStartTime {
    #[cfg(any(target_os = "linux", test))]
    Linux(u64),
    #[cfg(any(target_os = "macos", test))]
    MacOs { seconds: u64, microseconds: u64 },
}

impl ProcessStartTime {
    fn token(&self) -> String {
        match self {
            #[cfg(any(target_os = "linux", test))]
            Self::Linux(ticks) => format!("linux:{ticks}"),
            #[cfg(any(target_os = "macos", test))]
            Self::MacOs {
                seconds,
                microseconds,
            } => format!("macos:{seconds}:{microseconds}"),
        }
    }
}

pub(super) fn process_identity(pid: u32) -> Option<ProcessIdentity> {
    #[cfg(target_os = "linux")]
    {
        let stat = fs::read(format!("/proc/{pid}/stat")).ok()?;
        parse_linux_process_stat(pid, &stat)
    }

    #[cfg(target_os = "macos")]
    {
        let pid = libc::pid_t::try_from(pid).ok()?;
        let info = read_process_bsdinfo(pid)?;
        Some(ProcessIdentity {
            parent_pid: info.pbi_ppid,
            start_time: ProcessStartTime::MacOs {
                seconds: info.pbi_start_tvsec,
                microseconds: info.pbi_start_tvusec,
            },
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

/// Returns an OS-provided process start token that distinguishes PID reuse.
pub(crate) fn process_start_time(pid: u32) -> Option<String> {
    process_identity(pid).map(|identity| identity.start_time.token())
}

pub(crate) fn process_start_time_matches(pid: u32, expected: &str) -> bool {
    process_start_time(pid).as_deref() == Some(expected)
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_process_stat(pid: u32, stat: &[u8]) -> Option<ProcessIdentity> {
    const PARENT_PID_INDEX_AFTER_COMMAND: usize = 1;
    const START_TIME_INDEX_AFTER_COMMAND: usize = 19;

    let command_start = stat.iter().position(|&byte| byte == b'(')?;
    let command_end = stat.iter().rposition(|&byte| byte == b')')?;
    if command_end <= command_start {
        return None;
    }

    let stat_pid = std::str::from_utf8(stat.get(..command_start)?)
        .ok()?
        .trim()
        .parse::<u32>()
        .ok()?;
    if stat_pid != pid {
        return None;
    }

    let mut fields = std::str::from_utf8(stat.get(command_end + 1..)?)
        .ok()?
        .split_whitespace();
    Some(ProcessIdentity {
        parent_pid: fields
            .clone()
            .nth(PARENT_PID_INDEX_AFTER_COMMAND)?
            .parse::<u32>()
            .ok()?,
        start_time: ProcessStartTime::Linux(
            fields
                .nth(START_TIME_INDEX_AFTER_COMMAND)?
                .parse::<u64>()
                .ok()?,
        ),
    })
}

#[cfg(target_os = "macos")]
fn read_process_bsdinfo(pid: libc::pid_t) -> Option<libc::proc_bsdinfo> {
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
    Some(unsafe { info.assume_init() })
}

#[cfg(test)]
mod tests {
    use super::{ProcessIdentity, ProcessStartTime, parse_linux_process_stat};
    use rstest::rstest;

    #[rstest]
    #[case::linux(ProcessStartTime::Linux(123), "linux:123")]
    #[case::macos(
        ProcessStartTime::MacOs {
            seconds: 123,
            microseconds: 456,
        },
        "macos:123:456",
    )]
    fn process_start_time_token_cases(
        #[case] start_time: ProcessStartTime,
        #[case] expected: &str,
    ) {
        assert_eq!(start_time.token(), expected);
    }

    #[rstest]
    #[case::simple_command(
        b"42 (claude) S 7 8 9 0 -1 0 0 0 0 0 0 0 0 0 0 0 0 0 12345",
        42,
        Some(ProcessIdentity {
            parent_pid: 7,
            start_time: ProcessStartTime::Linux(12345),
        }),
    )]
    #[case::command_with_spaces_and_parentheses(
        b"42 (worker (nested) name) S 7 8 9 0 -1 0 0 0 0 0 0 0 0 0 0 0 0 0 12345",
        42,
        Some(ProcessIdentity {
            parent_pid: 7,
            start_time: ProcessStartTime::Linux(12345),
        }),
    )]
    #[case::different_pid(b"42 (claude) S 7 8 9 0 -1 0 0 0 0 0 0 0 0 0 0 0 0 0 12345", 43, None)]
    #[case::missing_parent(b"42 (claude)", 42, None)]
    #[case::missing_start_time(b"42 (claude) S 7", 42, None)]
    #[case::truncated_unicode_command_name(
        b"42 (worker-\xe2\x82) S 7 8 9 0 -1 0 0 0 0 0 0 0 0 0 0 0 0 0 12345",
        42,
        Some(ProcessIdentity {
            parent_pid: 7,
            start_time: ProcessStartTime::Linux(12345),
        }),
    )]
    fn parse_linux_process_stat_cases(
        #[case] stat: &[u8],
        #[case] pid: u32,
        #[case] expected: Option<ProcessIdentity>,
    ) {
        assert_eq!(parse_linux_process_stat(pid, stat), expected);
    }
}
