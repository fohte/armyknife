use serde_json::Value;

use super::super::SkillSuggestion;

pub(super) fn parse_claude_listing(content: Option<&Value>) -> Vec<SkillSuggestion> {
    super::super::claude_content_text(content)
        .lines()
        .filter_map(parse_skill_bullet)
        .collect()
}

pub(super) fn capture_codex_listing(record: &Value, skills: &mut Vec<SkillSuggestion>) {
    if record.get("type").and_then(Value::as_str) != Some("response_item") {
        return;
    }

    let Some(payload) = record.get("payload") else {
        return;
    };
    if payload.get("type").and_then(Value::as_str) != Some("message")
        || payload.get("role").and_then(Value::as_str) != Some("developer")
    {
        return;
    }

    let Some(content) = payload.get("content").and_then(Value::as_array) else {
        return;
    };
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("input_text") {
            continue;
        }
        let Some(text) = block.get("text").and_then(Value::as_str) else {
            continue;
        };
        if let Some(listing) = parse_codex_available_skills(text) {
            *skills = listing;
        }
    }
}

fn parse_codex_available_skills(text: &str) -> Option<Vec<SkillSuggestion>> {
    let text = text.trim();
    let text = text.strip_prefix("<skills_instructions>")?;
    let text = text.strip_suffix("</skills_instructions>")?;
    let mut in_available_skills = false;
    let mut skills = Vec::new();
    let mut found_section = false;

    for line in text.lines() {
        let heading = line.trim();
        if heading == "### Available skills" {
            in_available_skills = true;
            found_section = true;
            continue;
        }
        if in_available_skills && (heading.starts_with("## ") || heading.starts_with("### ")) {
            break;
        }
        if !in_available_skills {
            continue;
        }
        if let Some(skill) = parse_skill_bullet(heading) {
            skills.push(skill);
        }
    }

    found_section.then_some(skills)
}

fn parse_skill_bullet(line: &str) -> Option<SkillSuggestion> {
    let entry = line.trim().strip_prefix("- ")?;
    let (name, description) = entry.split_once(": ").unwrap_or((entry, ""));
    let name = name.trim().trim_matches('`').trim_matches('*');
    let description = description.trim();
    (!name.is_empty()).then(|| SkillSuggestion {
        name: name.to_owned(),
        description: description.to_owned(),
    })
}
