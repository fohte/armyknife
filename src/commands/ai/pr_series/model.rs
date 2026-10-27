use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use unicode_width::UnicodeWidthChar;

use crate::infra::tq::{TqChecklist, TqChecklistItem, TqGithubLink};

const MAX_LABEL_WIDTH_PX: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrSeriesPlan {
    pub name: String,
    pub purpose: String,
    pub pull_requests: Vec<PrSeriesEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrSeriesEntry {
    pub id: String,
    pub summary: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub repository: String,
    pub tq_leaf_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Repository {
    pub owner: String,
    pub name: String,
}

impl Repository {
    pub fn parse(value: &str) -> Result<Self> {
        let Some((owner, name)) = value.split_once('/') else {
            bail!("repository must use the `owner/repo` format");
        };
        ensure!(
            !owner.is_empty()
                && !name.is_empty()
                && !name.contains('/')
                && is_github_path_component(owner)
                && is_github_path_component(name),
            "invalid repository `{value}`"
        );
        Ok(Self {
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }

    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn key(&self) -> String {
        self.full_name().to_ascii_lowercase()
    }

    pub fn matches(&self, owner: &str, name: &str) -> bool {
        self.owner.eq_ignore_ascii_case(owner) && self.name.eq_ignore_ascii_case(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatedPullRequest {
    pub entry_id: String,
    pub repository: Repository,
    pub number: u64,
    pub url: String,
}

impl PrSeriesPlan {
    pub fn validate(&self) -> Result<()> {
        validate_single_line("series name", &self.name)?;
        validate_single_line("series purpose", &self.purpose)?;
        ensure!(
            !self.pull_requests.is_empty(),
            "plan must contain at least one pull request"
        );

        let mut entries_by_id = HashMap::new();
        let mut leaf_ids = HashSet::new();
        for entry in &self.pull_requests {
            validate_identifier(&entry.id)?;
            validate_single_line("pull request summary", &entry.summary)?;
            ensure!(
                estimated_label_width_px(&entry.summary) < MAX_LABEL_WIDTH_PX,
                "summary for pull request `{}` does not fit within the 200px Mermaid label width",
                entry.id
            );
            Repository::parse(&entry.repository)?;
            ensure!(
                !entry.tq_leaf_id.trim().is_empty(),
                "tqLeafId must not be empty"
            );
            ensure!(
                leaf_ids.insert(&entry.tq_leaf_id),
                "tqLeafId `{}` is used more than once",
                entry.tq_leaf_id
            );
            ensure!(
                entries_by_id.insert(entry.id.as_str(), entry).is_none(),
                "pull request id `{}` is used more than once",
                entry.id
            );
        }

        let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();
        let mut remaining_dependencies: HashMap<&str, usize> = HashMap::new();
        for entry in &self.pull_requests {
            let mut dependencies = HashSet::new();
            for dependency in &entry.depends_on {
                ensure!(
                    dependencies.insert(dependency.as_str()),
                    "pull request `{}` repeats dependency `{dependency}`",
                    entry.id
                );
                ensure!(
                    dependency != &entry.id,
                    "pull request `{}` cannot depend on itself",
                    entry.id
                );
                ensure!(
                    entries_by_id.contains_key(dependency.as_str()),
                    "pull request `{}` depends on unknown id `{dependency}`",
                    entry.id
                );
                dependents
                    .entry(dependency.as_str())
                    .or_default()
                    .push(entry.id.as_str());
            }
            remaining_dependencies.insert(entry.id.as_str(), entry.depends_on.len());
        }

        let mut ready = self
            .pull_requests
            .iter()
            .filter(|entry| entry.depends_on.is_empty())
            .map(|entry| entry.id.as_str())
            .collect::<VecDeque<_>>();
        let mut visited = 0;
        while let Some(id) = ready.pop_front() {
            visited += 1;
            for dependent in dependents.get(id).into_iter().flatten() {
                let count = remaining_dependencies
                    .get_mut(dependent)
                    .ok_or_else(|| anyhow::anyhow!("missing dependency count for `{dependent}`"))?;
                *count -= 1;
                if *count == 0 {
                    ready.push_back(dependent);
                }
            }
        }
        ensure!(
            visited == self.pull_requests.len(),
            "pull request dependencies contain a cycle"
        );

        Ok(())
    }
}

pub fn validate_checklist_leaf_ids(plan: &PrSeriesPlan, checklists: &[TqChecklist]) -> Result<()> {
    let mut leaf_counts = HashMap::<&str, usize>::new();
    for item in checklists.iter().flat_map(|checklist| &checklist.items) {
        collect_leaf_counts(item, &mut leaf_counts);
    }

    for entry in &plan.pull_requests {
        match leaf_counts
            .get(entry.tq_leaf_id.as_str())
            .copied()
            .unwrap_or_default()
        {
            0 => bail!(
                "tqLeafId `{}` for pull request `{}` does not identify a checklist leaf",
                entry.tq_leaf_id,
                entry.id
            ),
            1 => {}
            _ => bail!(
                "tqLeafId `{}` for pull request `{}` is ambiguous",
                entry.tq_leaf_id,
                entry.id
            ),
        }
    }
    Ok(())
}

pub fn resolve_created_pull_requests(
    plan: &PrSeriesPlan,
    checklists: &[TqChecklist],
    github_links: &[TqGithubLink],
) -> Result<Vec<CreatedPullRequest>> {
    validate_checklist_leaf_ids(plan, checklists)?;
    let leaves = checklist_leaves(checklists);
    let mut links_by_id = HashMap::new();
    for link in github_links {
        ensure!(
            links_by_id.insert(link.id.as_str(), link).is_none(),
            "task contains duplicate GitHub link id `{}`",
            link.id
        );
    }

    let mut created = Vec::new();
    let mut pull_request_keys = HashSet::new();
    for entry in &plan.pull_requests {
        let Some(link_id) = leaves
            .get(entry.tq_leaf_id.as_str())
            .and_then(|item| item.github_link_id.as_deref())
        else {
            continue;
        };
        let link = links_by_id
            .get(link_id)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("tq GitHub link `{link_id}` was not found"))?;
        ensure!(
            link.kind.as_deref() == Some("pull_request"),
            "tq link `{link_id}` for pull request `{}` is not a pull request",
            entry.id
        );
        let owner = link
            .owner
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("tq pull request link `{link_id}` has no owner"))?;
        let name = link
            .repo
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("tq pull request link `{link_id}` has no repository"))?;
        let number = link
            .number
            .ok_or_else(|| anyhow::anyhow!("tq pull request link `{link_id}` has no number"))?;
        let repository = Repository::parse(&entry.repository)?;
        ensure!(
            repository.matches(owner, name),
            "tq pull request link `{link_id}` points to `{owner}/{name}`, expected `{}`",
            repository.full_name()
        );
        let pull_request_key = format!("{}/{number}", repository.key());
        ensure!(
            pull_request_keys.insert(pull_request_key),
            "the same GitHub pull request is linked to multiple plan entries"
        );
        created.push(CreatedPullRequest {
            entry_id: entry.id.clone(),
            url: format!(
                "https://github.com/{}/{}/pull/{number}",
                repository.owner, repository.name
            ),
            repository,
            number,
        });
    }
    Ok(created)
}

fn collect_leaf_counts<'a>(item: &'a TqChecklistItem, counts: &mut HashMap<&'a str, usize>) {
    if item.children.is_empty() {
        *counts.entry(item.id.as_str()).or_default() += 1;
        return;
    }
    for child in &item.children {
        collect_leaf_counts(child, counts);
    }
}

fn checklist_leaves(checklists: &[TqChecklist]) -> HashMap<&str, &TqChecklistItem> {
    fn collect<'a>(item: &'a TqChecklistItem, leaves: &mut HashMap<&'a str, &'a TqChecklistItem>) {
        if item.children.is_empty() {
            leaves.insert(item.id.as_str(), item);
            return;
        }
        for child in &item.children {
            collect(child, leaves);
        }
    }

    let mut leaves = HashMap::new();
    for item in checklists.iter().flat_map(|checklist| &checklist.items) {
        collect(item, &mut leaves);
    }
    leaves
}

fn validate_identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.chars().all(
                |character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            ),
        "pull request ids must contain only ASCII letters, digits, `-`, or `_`"
    );
    Ok(())
}

fn validate_single_line(field: &str, value: &str) -> Result<()> {
    ensure!(!value.trim().is_empty(), "{field} must not be empty");
    ensure!(
        !value.chars().any(char::is_control),
        "{field} must be a single line"
    );
    ensure!(
        !value.contains("<!-- pr-series:"),
        "{field} must not contain PR series markers"
    );
    Ok(())
}

fn is_github_path_component(value: &str) -> bool {
    value != "."
        && value != ".."
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn estimated_label_width_px(value: &str) -> usize {
    value.chars().map(estimated_char_width_px).sum()
}

fn estimated_char_width_px(character: char) -> usize {
    match character {
        ' ' => 5,
        'i' | 'l' | 'I' => 5,
        'j' | 'r' | 't' | 'f' | '!' | '.' | ',' | ':' | ';' | '\'' | '`' => 6,
        'm' | 'M' => 14,
        'w' => 12,
        'W' => 16,
        _ if character.is_ascii_uppercase() => 12,
        _ if character.is_ascii_lowercase() => 9,
        _ if character.is_ascii_digit() => 10,
        '@' => 16,
        '%' => 14,
        _ if character.is_ascii_punctuation() => 10,
        _ => UnicodeWidthChar::width(character).unwrap_or_default() * 9,
    }
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::rstest;

    use super::*;

    fn plan_with_entries(pull_requests: Vec<PrSeriesEntry>) -> PrSeriesPlan {
        PrSeriesPlan {
            name: "Notification preferences".to_string(),
            purpose: "Let users choose which notifications they receive.".to_string(),
            pull_requests,
        }
    }

    fn entry(id: &str, depends_on: &[&str], repository: &str, leaf: &str) -> PrSeriesEntry {
        PrSeriesEntry {
            id: id.to_string(),
            summary: format!("Add {id} support"),
            depends_on: depends_on.iter().map(|value| value.to_string()).collect(),
            repository: repository.to_string(),
            tq_leaf_id: leaf.to_string(),
        }
    }

    #[test]
    fn validates_an_acyclic_plan_with_cross_repository_dependencies() {
        let plan = plan_with_entries(vec![
            entry("schema", &[], "acme/notify-api", "leaf-schema"),
            entry("settings", &["schema"], "acme/notify-web", "leaf-settings"),
        ]);

        assert_eq!(plan.validate().map_err(|error| error.to_string()), Ok(()));
    }

    #[rstest]
    #[case::unknown_dependency(
        vec![entry("settings", &["missing"], "acme/notify-web", "leaf-settings")],
        "pull request `settings` depends on unknown id `missing`"
    )]
    #[case::cycle(
        vec![
            entry("schema", &["settings"], "acme/notify-api", "leaf-schema"),
            entry("settings", &["schema"], "acme/notify-web", "leaf-settings"),
        ],
        "pull request dependencies contain a cycle"
    )]
    #[case::wide_summary(
        vec![PrSeriesEntry {
            id: "wide".to_string(),
            summary: "W".repeat(15),
            depends_on: Vec::new(),
            repository: "acme/notify-api".to_string(),
            tq_leaf_id: "leaf-wide".to_string(),
        }],
        "summary for pull request `wide` does not fit within the 200px Mermaid label width"
    )]
    #[case::exact_width(
        vec![PrSeriesEntry {
            id: "wide".to_string(),
            summary: format!("{}{}", "あ".repeat(10), "i".repeat(4)),
            depends_on: Vec::new(),
            repository: "acme/notify-api".to_string(),
            tq_leaf_id: "leaf-wide".to_string(),
        }],
        "summary for pull request `wide` does not fit within the 200px Mermaid label width"
    )]
    fn rejects_invalid_dependencies_and_labels(
        #[case] entries: Vec<PrSeriesEntry>,
        #[case] expected_error: &str,
    ) {
        assert_eq!(
            plan_with_entries(entries)
                .validate()
                .err()
                .map(|error| error.to_string()),
            Some(expected_error.to_string()),
        );
    }

    #[test]
    fn rejects_tq_leaf_ids_that_do_not_identify_checklist_leaves() {
        let plan = plan_with_entries(vec![entry(
            "schema",
            &[],
            "acme/notify-api",
            "missing-leaf",
        )]);
        let checklists = vec![TqChecklist {
            id: "checklist-1".to_string(),
            name: "Implementation".to_string(),
            items: vec![TqChecklistItem {
                id: "batch-1".to_string(),
                content: "Batch 1".to_string(),
                checked_at: None,
                github_link_id: None,
                children: vec![TqChecklistItem {
                    id: "leaf-schema".to_string(),
                    content: "Add schema".to_string(),
                    checked_at: None,
                    github_link_id: None,
                    children: Vec::new(),
                }],
            }],
        }];

        assert_eq!(
            validate_checklist_leaf_ids(&plan, &checklists)
                .err()
                .map(|error| error.to_string()),
            Some(
                "tqLeafId `missing-leaf` for pull request `schema` does not identify a checklist leaf"
                    .to_string()
            ),
        );
    }

    #[test]
    fn resolves_created_pull_requests_from_leaf_github_links() {
        let plan = plan_with_entries(vec![entry("schema", &[], "acme/notify-api", "leaf-schema")]);
        let checklists: Vec<TqChecklist> = serde_json::from_str(indoc! {r#"
            [
              {
                "id": "checklist-1",
                "name": "Implementation",
                "items": [
                  {
                    "id": "batch-1",
                    "content": "Batch 1",
                    "children": [
                      {
                        "id": "leaf-schema",
                        "content": "Add schema",
                        "githubLinkId": "link-schema",
                        "children": []
                      }
                    ]
                  }
                ]
              }
            ]
        "#})
        .unwrap();
        let links: Vec<TqGithubLink> = serde_json::from_str(indoc! {r#"
            [
              {
                "id": "link-schema",
                "kind": "pull_request",
                "owner": "ACME",
                "repo": "notify-api",
                "number": 17,
                "url": "https://github.com/ACME/notify-api/pull/17"
              }
            ]
        "#})
        .unwrap();

        assert_eq!(
            resolve_created_pull_requests(&plan, &checklists, &links).unwrap(),
            vec![CreatedPullRequest {
                entry_id: "schema".to_string(),
                repository: Repository {
                    owner: "acme".to_string(),
                    name: "notify-api".to_string(),
                },
                number: 17,
                url: "https://github.com/acme/notify-api/pull/17".to_string(),
            }],
        );
    }
}
