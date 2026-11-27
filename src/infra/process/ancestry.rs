#[cfg(target_os = "linux")]
use std::fs;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessInfo {
    pub(crate) parent_pid: u32,
    pub(crate) argv0: String,
}

#[derive(Debug, PartialEq, Eq)]
struct ProcessIdentity<T> {
    parent_pid: u32,
    start_time: T,
}

fn process_info_if_stable<T: PartialEq>(
    before: ProcessIdentity<T>,
    argv0: String,
    after: ProcessIdentity<T>,
) -> Option<ProcessInfo> {
    (before == after).then_some(ProcessInfo {
        parent_pid: before.parent_pid,
        argv0,
    })
}

/// Reads a process's parent PID and argv[0] without spawning a process.
pub(crate) fn get_process_info(pid: u32) -> Option<ProcessInfo> {
    #[cfg(target_os = "linux")]
    {
        let before = read_process_identity(pid)?;
        let cmdline = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
        let argv0 = parse_linux_process_argv0(&cmdline)?;
        let after = read_process_identity(pid)?;
        process_info_if_stable(before, argv0, after)
    }

    #[cfg(target_os = "macos")]
    {
        let pid = libc::pid_t::try_from(pid).ok()?;
        let before = read_process_identity(pid)?;
        let argv0 = process_argv0(pid)?;
        let after = read_process_identity(pid)?;
        process_info_if_stable(before, argv0, after)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

#[cfg(target_os = "macos")]
fn process_argv0(pid: libc::pid_t) -> Option<String> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size = 0;
    // SAFETY: `mib` contains the sysctl key and `size` is a valid output pointer.
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 || size == 0 {
        return None;
    }

    let mut args = vec![0; size];
    let mut actual_size = args.len();
    // SAFETY: `mib` is valid and `args` provides `actual_size` writable bytes.
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            args.as_mut_ptr().cast(),
            &mut actual_size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 || actual_size > args.len() {
        return None;
    }
    args.truncate(actual_size);
    parse_macos_process_argv0(&args)
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_process_argv0(args: &[u8]) -> Option<String> {
    let argc_size = std::mem::size_of::<i32>();
    let argc = i32::from_ne_bytes(args.get(..argc_size)?.try_into().ok()?);
    if argc <= 0 {
        return None;
    }

    let mut offset = argc_size;
    let executable_path_len = args.get(offset..)?.iter().position(|&byte| byte == 0)?;
    offset += executable_path_len + 1;
    while args.get(offset) == Some(&0) {
        offset += 1;
    }

    let argv0 = args.get(offset..)?;
    let argv0_len = argv0.iter().position(|&byte| byte == 0)?;
    let argv0 = argv0.get(..argv0_len)?;
    if argv0.is_empty() {
        return None;
    }

    Some(String::from_utf8_lossy(argv0).into_owned())
}

/// Looks up the parent PID of `pid` without spawning a process.
pub fn get_parent_pid(pid: u32) -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        read_process_identity(pid).map(|identity| identity.parent_pid)
    }

    #[cfg(target_os = "macos")]
    {
        let pid = libc::pid_t::try_from(pid).ok()?;
        read_process_identity(pid).map(|identity| identity.parent_pid)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        None
    }
}

#[cfg(target_os = "linux")]
fn read_process_identity(pid: u32) -> Option<ProcessIdentity<u64>> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_linux_process_stat(pid, &stat)
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_process_stat(pid: u32, stat: &str) -> Option<ProcessIdentity<u64>> {
    const PARENT_PID_INDEX_AFTER_COMMAND: usize = 1;
    const START_TIME_INDEX_AFTER_COMMAND: usize = 19;

    let command_start = stat.find('(')?;
    let command_end = stat.rfind(')')?;
    if command_end <= command_start {
        return None;
    }

    let stat_pid = stat[..command_start].trim().parse::<u32>().ok()?;
    if stat_pid != pid {
        return None;
    }

    let mut fields = stat.get(command_end + 1..)?.split_whitespace();
    Some(ProcessIdentity {
        parent_pid: fields
            .clone()
            .nth(PARENT_PID_INDEX_AFTER_COMMAND)?
            .parse::<u32>()
            .ok()?,
        start_time: fields
            .nth(START_TIME_INDEX_AFTER_COMMAND)?
            .parse::<u64>()
            .ok()?,
    })
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_process_argv0(cmdline: &[u8]) -> Option<String> {
    let argv0 = cmdline.split(|&byte| byte == 0).next()?;
    if argv0.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(argv0).into_owned())
}

#[cfg(target_os = "macos")]
fn read_process_identity(pid: libc::pid_t) -> Option<ProcessIdentity<(u64, u64)>> {
    let info = read_process_bsdinfo(pid)?;
    Some(ProcessIdentity {
        parent_pid: info.pbi_ppid,
        start_time: (info.pbi_start_tvsec, info.pbi_start_tvusec),
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
    use super::{ProcessIdentity, ProcessInfo, process_info_if_stable};
    use rstest::rstest;

    #[rstest]
    #[case::stable_identity(
        ProcessIdentity { parent_pid: 7, start_time: 42 },
        ProcessIdentity { parent_pid: 7, start_time: 42 },
        Some(ProcessInfo { parent_pid: 7, argv0: "claude".to_string() }),
    )]
    #[case::reused_pid(
        ProcessIdentity { parent_pid: 7, start_time: 42 },
        ProcessIdentity { parent_pid: 7, start_time: 43 },
        None,
    )]
    #[case::changed_parent(
        ProcessIdentity { parent_pid: 7, start_time: 42 },
        ProcessIdentity { parent_pid: 8, start_time: 42 },
        None,
    )]
    fn process_info_requires_stable_identity(
        #[case] before: ProcessIdentity<u64>,
        #[case] after: ProcessIdentity<u64>,
        #[case] expected: Option<ProcessInfo>,
    ) {
        assert_eq!(
            process_info_if_stable(before, "claude".to_string(), after),
            expected,
        );
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn get_parent_pid_reads_the_current_process_parent() {
        // SAFETY: `getppid` has no preconditions and does not access memory through pointers.
        let parent_pid = unsafe { libc::getppid() };
        assert_eq!(
            super::get_parent_pid(std::process::id()),
            u32::try_from(parent_pid).ok(),
        );
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn get_process_info_reads_the_current_process_argv0() {
        let expected_argv0 = std::env::args_os()
            .next()
            .map(|arg| arg.to_string_lossy().into_owned());
        assert_eq!(
            super::get_process_info(std::process::id()).map(|info| info.argv0),
            expected_argv0,
        );
    }
}

#[cfg(test)]
mod linux_tests {
    use super::{ProcessIdentity, parse_linux_process_argv0, parse_linux_process_stat};
    use rstest::rstest;

    #[rstest]
    #[case::simple_command(
        "42 (claude) S 7 8 9 0 -1 0 0 0 0 0 0 0 0 0 0 0 0 0 12345",
        42,
        Some(ProcessIdentity { parent_pid: 7, start_time: 12345 }),
    )]
    #[case::command_with_spaces_and_parentheses(
        "42 (worker (nested) name) S 7 8 9 0 -1 0 0 0 0 0 0 0 0 0 0 0 0 0 12345",
        42,
        Some(ProcessIdentity { parent_pid: 7, start_time: 12345 }),
    )]
    #[case::different_pid("42 (claude) S 7 8 9 0 -1 0 0 0 0 0 0 0 0 0 0 0 0 0 12345", 43, None)]
    #[case::missing_parent("42 (claude)", 42, None)]
    #[case::missing_start_time("42 (claude) S 7", 42, None)]
    fn parse_linux_process_stat_cases(
        #[case] stat: &str,
        #[case] pid: u32,
        #[case] expected: Option<ProcessIdentity<u64>>,
    ) {
        assert_eq!(parse_linux_process_stat(pid, stat), expected);
    }

    #[rstest]
    #[case::bare_name(b"claude\0--flag\0", Some("claude"))]
    #[case::path_name(b"/agent/bin/codex\0--flag\0", Some("/agent/bin/codex"))]
    #[case::empty_cmdline(b"", None)]
    #[case::empty_argv0(b"\0claude\0", None)]
    fn parse_linux_process_argv0_cases(#[case] cmdline: &[u8], #[case] expected: Option<&str>) {
        assert_eq!(parse_linux_process_argv0(cmdline).as_deref(), expected,);
    }
}

#[cfg(test)]
mod macos_tests {
    use super::parse_macos_process_argv0;
    use rstest::rstest;

    #[test]
    fn parse_macos_process_argv0_skips_exec_path_and_padding() {
        let mut args = 2_i32.to_ne_bytes().to_vec();
        args.extend_from_slice(b"/agent/versions/1.2.3\0\0/agent/bin/claude\0--flag\0");

        assert_eq!(
            parse_macos_process_argv0(&args).as_deref(),
            Some("/agent/bin/claude"),
        );
    }

    #[rstest]
    #[case::missing_argc(&[])]
    #[case::nonpositive_argc(&[0, 0, 0, 0])]
    #[case::unterminated_exec_path(&[1, 0, 0, 0, b'x'])]
    #[case::missing_argv0(&[1, 0, 0, 0, b'x', 0])]
    fn parse_macos_process_argv0_rejects_incomplete_arguments(#[case] args: &[u8]) {
        assert_eq!(parse_macos_process_argv0(args), None);
    }
}
