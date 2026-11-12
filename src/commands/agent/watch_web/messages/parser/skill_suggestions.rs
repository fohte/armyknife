use serde_json::Value;

use super::super::SkillSuggestion;

pub(super) fn parse_claude_listing(content: Option<&Value>) -> Vec<SkillSuggestion> {
    super::super::claude_content_text(content)
        .lines()
        .filter_map(|line| {
            let entry = line.trim().strip_prefix("- ")?;
            let (name, description) = entry.split_once(':')?;
            let name = name.trim();
            let description = description.trim();
            (!name.is_empty() && !description.is_empty()).then(|| SkillSuggestion {
                name: name.to_owned(),
                description: description.to_owned(),
            })
        })
        .collect()
}

pub(super) fn capture_codex_listing(value: &Value, skills: &mut Vec<SkillSuggestion>) {
    match value {
        Value::String(text) => {
            if let Some(listing) = parse_codex_available_skills(text) {
                *skills = listing;
            }
        }
        Value::Array(values) => {
            for value in values {
                capture_codex_listing(value, skills);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                capture_codex_listing(value, skills);
            }
        }
        _ => {}
    }
}

fn parse_codex_available_skills(text: &str) -> Option<Vec<SkillSuggestion>> {
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
        let Some(entry) = heading.strip_prefix("- ") else {
            continue;
        };
        let Some((name, description)) = entry.split_once(':') else {
            continue;
        };
        let name = name.trim().trim_matches('`').trim_matches('*');
        let description = description.trim();
        if !name.is_empty() && !description.is_empty() {
            skills.push(SkillSuggestion {
                name: name.to_owned(),
                description: description.to_owned(),
            });
        }
    }

    found_section.then_some(skills)
}
