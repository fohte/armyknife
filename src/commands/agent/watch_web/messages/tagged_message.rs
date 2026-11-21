use super::action_event;
use crate::commands::agent::watch_web::attachments::strip_attachment_paths;

pub(super) fn tagged_body(text: &str) -> Option<(&str, &str)> {
    let tag = outer_tag(text)?;
    let trimmed = text.trim();
    let opening = format!("<{tag}>");
    let closing = format!("</{tag}>");
    let body = trimmed
        .strip_prefix(&opening)?
        .strip_suffix(&closing)?
        .trim();
    Some((tag, body))
}

pub(super) fn tagged_action_body(text: &str) -> Option<(&str, &str)> {
    if let Some((tag, body)) = tagged_body(text)
        && action_event::is_action_event_tag(tag)
    {
        return Some((tag, body));
    }

    let (tag, body) = tagged_body(strip_attachment_paths(text))?;
    action_event::is_action_event_tag(tag).then_some((tag, body))
}

fn outer_tag(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let tag = trimmed.strip_prefix('<')?.split_once('>')?.0;
    if tag.is_empty()
        || tag.starts_with('/')
        || !tag
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return None;
    }
    trimmed.ends_with(&format!("</{tag}>")).then_some(tag)
}
