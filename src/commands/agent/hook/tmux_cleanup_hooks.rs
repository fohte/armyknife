use crate::infra::tmux;

const HOOK_INDEX: usize = 1000;
const CLEANUP_COMMAND: &str = "run-shell -b 'a agent cleanup-stale-sessions'";
const TMUX_HOOKS: [&str; 4] = [
    "pane-exited",
    "pane-died",
    "after-kill-pane",
    "session-closed",
];

pub(super) fn configure(mut run_tmux: impl FnMut(&[&str]) -> tmux::Result<()>) -> tmux::Result<()> {
    for hook in TMUX_HOOKS {
        let indexed_hook = format!("{hook}[{HOOK_INDEX}]");
        run_tmux(&["set-hook", "-g", &indexed_hook, CLEANUP_COMMAND])?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::configure;

    #[test]
    fn configure_sets_each_global_cleanup_hook_at_the_fixed_index() {
        let mut calls = Vec::new();

        configure(|args| {
            calls.push(args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
            Ok(())
        })
        .expect("tmux hook setup should succeed");

        assert_eq!(
            calls,
            vec![
                vec![
                    "set-hook",
                    "-g",
                    "pane-exited[1000]",
                    "run-shell -b 'a agent cleanup-stale-sessions'",
                ],
                vec![
                    "set-hook",
                    "-g",
                    "pane-died[1000]",
                    "run-shell -b 'a agent cleanup-stale-sessions'",
                ],
                vec![
                    "set-hook",
                    "-g",
                    "after-kill-pane[1000]",
                    "run-shell -b 'a agent cleanup-stale-sessions'",
                ],
                vec![
                    "set-hook",
                    "-g",
                    "session-closed[1000]",
                    "run-shell -b 'a agent cleanup-stale-sessions'",
                ],
            ]
        );
    }
}
