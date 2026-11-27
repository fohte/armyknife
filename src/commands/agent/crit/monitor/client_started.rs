use anyhow::Result;
use clap::Args;

use super::wait_state::set_crit_wait_state;
use super::{clients, ensure_started};
use crate::commands::agent::crit::url::parse_port;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ClientStartedArgs {
    /// Crit review URL the client connected to.
    #[arg(long)]
    url: String,

    /// Process ID that lives as long as the client; its exit ends the wait.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pid: u32,
}

pub(in crate::commands::agent::crit) fn run(args: &ClientStartedArgs) -> Result<()> {
    let port = parse_port(&args.url)?;
    ensure_started(port, None)?;
    clients::record_client(port, args.pid)?;
    set_crit_wait_state(port, Some(true))?;
    tracing::info!(
        event = "agent.crit.client_started",
        port,
        client_pid = args.pid
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use rstest::rstest;

    use super::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        args: ClientStartedArgs,
    }

    #[rstest]
    #[case::valid(
        &["--url", "http://127.0.0.1:43210/review", "--pid", "4242"],
        Some(("http://127.0.0.1:43210/review", 4242)),
    )]
    #[case::zero_pid(&["--url", "http://127.0.0.1:43210/review", "--pid", "0"], None)]
    #[case::missing_pid(&["--url", "http://127.0.0.1:43210/review"], None)]
    fn parses_client_started_arguments(
        #[case] argv: &[&str],
        #[case] expected: Option<(&str, u32)>,
    ) {
        let parsed =
            TestCli::try_parse_from(std::iter::once("client-started").chain(argv.iter().copied()))
                .ok()
                .map(|cli| (cli.args.url, cli.args.pid));

        assert_eq!(parsed, expected.map(|(url, pid)| (url.to_string(), pid)),);
    }
}
