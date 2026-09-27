use anyhow::{Context, Result};

pub(super) fn parse_port(url: &str) -> Result<u16> {
    let (_, rest) = url
        .split_once("://")
        .with_context(|| format!("Invalid crit review URL: {url}"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let port = if authority.starts_with('[') {
        let (_, port) = authority
            .rsplit_once("]:")
            .with_context(|| format!("Crit review URL has no port: {url}"))?;
        port
    } else {
        authority
            .rsplit_once(':')
            .map(|(_, port)| port)
            .with_context(|| format!("Crit review URL has no port: {url}"))?
    };
    let port = port
        .parse::<u16>()
        .with_context(|| format!("Invalid port in crit review URL: {url}"))?;
    anyhow::ensure!(port != 0, "Crit review URL has an invalid port: {url}");
    Ok(port)
}
