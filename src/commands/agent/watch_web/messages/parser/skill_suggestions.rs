use std::io::{self, BufRead};

use serde_json::Value;

use crate::commands::agent::types::Engine;

use super::super::SkillSuggestion;

pub(super) fn parse_claude_listing(content: Option<&Value>) -> Vec<SkillSuggestion> {
    super::super::claude_content_text(content)
        .lines()
        .filter_map(parse_skill_bullet)
        .collect()
}

pub(super) fn capture_codex_listing(record: &Value, skills: &mut Vec<SkillSuggestion>) {
    if let Some(listing) = parse_codex_listing(record) {
        *skills = listing;
    }
}

pub(in crate::commands::agent::watch_web::messages) fn read_transcript_listing(
    reader: impl BufRead,
    engine: Engine,
) -> io::Result<Option<Vec<SkillSuggestion>>> {
    let mut skills: Option<Vec<SkillSuggestion>> = None;
    for line in reader.lines() {
        let line = line?;
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match engine {
            Engine::Claude => {
                let Some(attachment) = record.get("attachment") else {
                    continue;
                };
                if attachment.get("type").and_then(Value::as_str) != Some("skill_listing") {
                    continue;
                }
                let listing = parse_claude_listing(attachment.get("content"));
                if attachment.get("isInitial").and_then(Value::as_bool) == Some(false) {
                    let current = skills.get_or_insert_with(Vec::new);
                    for skill in listing {
                        if let Some(existing) = current
                            .iter_mut()
                            .find(|existing| existing.name == skill.name)
                        {
                            existing.description = skill.description;
                        } else {
                            current.push(skill);
                        }
                    }
                } else {
                    skills = Some(listing);
                }
            }
            Engine::Codex => {
                if let Some(listing) = parse_codex_listing(&record) {
                    skills = Some(listing);
                }
            }
        }
    }
    Ok(skills)
}

fn parse_codex_listing(record: &Value) -> Option<Vec<SkillSuggestion>> {
    if record.get("type").and_then(Value::as_str) != Some("response_item") {
        return None;
    }

    let payload = record.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("message")
        || payload.get("role").and_then(Value::as_str) != Some("developer")
    {
        return None;
    }

    let content = payload.get("content").and_then(Value::as_array)?;
    let mut listing: Option<Vec<SkillSuggestion>> = None;
    for block in content {
        if block.get("type").and_then(Value::as_str) != Some("input_text") {
            continue;
        }
        let Some(text) = block.get("text").and_then(Value::as_str) else {
            continue;
        };
        if let Some(parsed) = parse_codex_available_skills(text) {
            listing = Some(parsed);
        }
    }
    listing
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
