use anyhow::{Context, Result};

pub(in crate::commands::agent) fn add_url(crit_urls: &mut Vec<String>, url: &str) {
    if let Ok(new_url) = reqwest::Url::parse(url)
        && new_url.path() == "/"
        && new_url.query().is_none()
        && new_url.fragment().is_none()
        && let Some(position) = crit_urls.iter().rposition(|existing| {
            reqwest::Url::parse(existing).is_ok_and(|existing_url| {
                existing_url.path() != "/"
                    && existing_url.scheme() == new_url.scheme()
                    && existing_url.host_str() == new_url.host_str()
                    && existing_url.port_or_known_default() == new_url.port_or_known_default()
            })
        })
    {
        let existing_url = crit_urls.remove(position);
        crit_urls.push(existing_url);
        return;
    }

    if let Some(position) = crit_urls.iter().position(|existing| existing == url) {
        crit_urls.remove(position);
    }
    crit_urls.push(url.to_string());
}

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

#[cfg(test)]
mod tests {
    use super::add_url;
    use rstest::rstest;

    #[rstest]
    #[case::root_url_keeps_same_origin_pathful_url(
        vec![
            "http://review.example.test:43210/preview".to_string(),
            "http://other.example.test:43210/review".to_string(),
        ],
        "http://review.example.test:43210/",
        vec![
            "http://other.example.test:43210/review".to_string(),
            "http://review.example.test:43210/preview".to_string(),
        ],
    )]
    #[case::newest_same_origin_pathful_url_is_promoted(
        vec![
            "http://review.example.test:43210/preview".to_string(),
            "http://review.example.test:43210/next".to_string(),
            "http://other.example.test:43210/review".to_string(),
        ],
        "http://review.example.test:43210/",
        vec![
            "http://review.example.test:43210/preview".to_string(),
            "http://other.example.test:43210/review".to_string(),
            "http://review.example.test:43210/next".to_string(),
        ],
    )]
    #[case::different_origin_adds_root_url(
        vec!["http://review.example.test:43210/preview".to_string()],
        "http://other.example.test:43210/",
        vec![
            "http://review.example.test:43210/preview".to_string(),
            "http://other.example.test:43210/".to_string(),
        ],
    )]
    #[case::different_scheme_adds_root_url(
        vec!["http://review.example.test:43210/preview".to_string()],
        "https://review.example.test:43210/",
        vec![
            "http://review.example.test:43210/preview".to_string(),
            "https://review.example.test:43210/".to_string(),
        ],
    )]
    #[case::different_port_adds_root_url(
        vec!["http://review.example.test:43210/preview".to_string()],
        "http://review.example.test:43211/",
        vec![
            "http://review.example.test:43210/preview".to_string(),
            "http://review.example.test:43211/".to_string(),
        ],
    )]
    #[case::pathful_url_is_added_normally(
        vec!["http://review.example.test:43210/preview".to_string()],
        "http://review.example.test:43210/next",
        vec![
            "http://review.example.test:43210/preview".to_string(),
            "http://review.example.test:43210/next".to_string(),
        ],
    )]
    #[case::root_url_is_added_when_origin_has_no_pathful_url(
        vec!["http://review.example.test:43210/".to_string()],
        "http://review.example.test:43210/",
        vec!["http://review.example.test:43210/".to_string()],
    )]
    #[case::root_url_with_query_is_recorded(
        vec!["http://review.example.test:43210/preview".to_string()],
        "http://review.example.test:43210/?id=42",
        vec![
            "http://review.example.test:43210/preview".to_string(),
            "http://review.example.test:43210/?id=42".to_string(),
        ],
    )]
    #[case::root_url_with_fragment_is_recorded(
        vec!["http://review.example.test:43210/preview".to_string()],
        "http://review.example.test:43210/#section",
        vec![
            "http://review.example.test:43210/preview".to_string(),
            "http://review.example.test:43210/#section".to_string(),
        ],
    )]
    fn adds_crit_url(
        #[case] mut crit_urls: Vec<String>,
        #[case] url: &str,
        #[case] expected: Vec<String>,
    ) {
        add_url(&mut crit_urls, url);

        assert_eq!(crit_urls, expected);
    }
}
