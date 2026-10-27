use super::Provider;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ModelIdentity {
    raw_slug: String,
}

impl ModelIdentity {
    pub(super) fn known(raw_slug: String) -> anyhow::Result<Self> {
        if raw_slug.trim().is_empty() || raw_slug.contains(['\n', '\r', '\0', '<', '>']) {
            anyhow::bail!("runtime model slug is empty or cannot fit in a co-author trailer");
        }
        Ok(Self { raw_slug })
    }

    fn raw_slug(&self) -> &str {
        &self.raw_slug
    }
}

pub(super) fn coauthor_trailer(
    provider: Provider,
    model: &ModelIdentity,
    effort: Option<&str>,
) -> Option<String> {
    let effort = effort.filter(|effort| {
        !effort.trim().is_empty() && !effort.contains(['\n', '\r', '\0', '<', '>'])
    });
    let model = match effort {
        Some(effort) => format!("{}, {effort}", model.raw_slug()),
        None => model.raw_slug().to_owned(),
    };
    let (name, email) = match provider {
        Provider::Codex => ("Codex", "noreply@openai.com"),
        Provider::ClaudeCode => ("Claude Code", "noreply@anthropic.com"),
    };
    Some(format!("Co-authored-by: {name} ({model}) <{email}>"))
}
