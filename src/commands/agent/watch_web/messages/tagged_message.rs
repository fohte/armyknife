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
