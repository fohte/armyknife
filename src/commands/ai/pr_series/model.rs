use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::infra::tq::{TqChecklist, TqChecklistItem, TqGithubLink};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrSeriesPlan {
    pub name: String,
    pub purpose: String,
    pub pull_requests: Vec<PrSeriesEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<PrSeriesStep>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrSeriesStep {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub pull_requests: Vec<String>,
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

        if let Some(steps) = &self.steps {
            let mut step_by_entry = HashMap::new();
            for (step_index, step) in steps.iter().enumerate() {
                ensure!(
                    !step.pull_requests.is_empty(),
                    "step {} must contain at least one pull request",
                    step_index + 1
                );
                if let Some(summary) = &step.summary {
                    validate_single_line("step summary", summary)?;
                }
                ensure!(
                    step.pull_requests.len() < 2 || step.summary.is_some(),
                    "step {} must have a summary when it contains multiple pull requests",
                    step_index + 1
                );

                for entry_id in &step.pull_requests {
                    ensure!(
                        entries_by_id.contains_key(entry_id.as_str()),
                        "step {} references unknown pull request id `{entry_id}`",
                        step_index + 1
                    );
                    ensure!(
                        step_by_entry
                            .insert(entry_id.as_str(), step_index)
                            .is_none(),
                        "pull request `{entry_id}` belongs to multiple steps"
                    );
                }
            }

            for entry in &self.pull_requests {
                let step_index =
                    step_by_entry
                        .get(entry.id.as_str())
                        .copied()
                        .ok_or_else(|| {
                            anyhow::anyhow!("pull request `{}` does not belong to a step", entry.id)
                        })?;
                for dependency in &entry.depends_on {
                    let dependency_step = step_by_entry
                        .get(dependency.as_str())
                        .copied()
                        .ok_or_else(|| {
                            anyhow::anyhow!("pull request `{dependency}` does not belong to a step")
                        })?;
                    ensure!(
                        dependency_step < step_index,
                        "pull request `{}` depends on `{dependency}` from the same or a later step",
                        entry.id
                    );
                }
            }
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

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::{fixture, rstest};

    use super::*;

    fn plan_with_entries(pull_requests: Vec<PrSeriesEntry>) -> PrSeriesPlan {
        PrSeriesPlan {
            name: "Notification preferences".to_string(),
            purpose: "Let users choose which notifications they receive.".to_string(),
            pull_requests,
            steps: None,
        }
    }

    fn plan_with_steps(
        pull_requests: Vec<PrSeriesEntry>,
        steps: Vec<PrSeriesStep>,
    ) -> PrSeriesPlan {
        PrSeriesPlan {
            steps: Some(steps),
            ..plan_with_entries(pull_requests)
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

    fn step(summary: Option<&str>, pull_requests: &[&str]) -> PrSeriesStep {
        PrSeriesStep {
            summary: summary.map(str::to_string),
            pull_requests: pull_requests.iter().map(|id| id.to_string()).collect(),
        }
    }

    fn valid_cross_repository_plan() -> PrSeriesPlan {
        plan_with_entries(vec![
            entry("schema", &[], "acme/notify-api", "leaf-schema"),
            entry("settings", &["schema"], "acme/notify-web", "leaf-settings"),
        ])
    }

    fn valid_long_summary_plan() -> PrSeriesPlan {
        let mut long_entry = entry("long", &[], "acme/notify-api", "leaf-long");
        long_entry.summary = "W".repeat(300);
        plan_with_entries(vec![long_entry])
    }

    fn valid_step_order_plan() -> PrSeriesPlan {
        plan_with_steps(
            vec![
                entry("schema", &[], "acme/notify-api", "leaf-schema"),
                entry("docs", &[], "acme/notify-docs", "leaf-docs"),
                entry("settings", &["schema"], "acme/notify-web", "leaf-settings"),
            ],
            vec![
                step(Some("Add schema and documentation"), &["schema", "docs"]),
                step(None, &["settings"]),
            ],
        )
    }

    #[rstest]
    #[case::acyclic_cross_repository_dependencies(valid_cross_repository_plan())]
    #[case::long_summary_without_label_width_limit(valid_long_summary_plan())]
    #[case::dependencies_in_earlier_steps(valid_step_order_plan())]
    fn accepts_valid_plans(#[case] plan: PrSeriesPlan) {
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
    #[case::multiline_summary(
        vec![PrSeriesEntry {
            id: "multiline".to_string(),
            summary: indoc! {"
                first line
                second line
            "}
            .to_string(),
            depends_on: Vec::new(),
            repository: "acme/notify-api".to_string(),
            tq_leaf_id: "leaf-multiline".to_string(),
        }],
        "pull request summary must be a single line"
    )]
    fn rejects_invalid_dependencies_and_multiline_summaries(
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

    #[rstest]
    #[case::missing_multi_pull_request_summary(
        vec![entry("schema", &[], "acme/notify-api", "leaf-schema"), entry("settings", &[], "acme/notify-web", "leaf-settings")],
        vec![step(None, &["schema", "settings"])],
        "step 1 must have a summary when it contains multiple pull requests"
    )]
    #[case::empty_step(
        vec![entry("schema", &[], "acme/notify-api", "leaf-schema")],
        vec![step(None, &[])],
        "step 1 must contain at least one pull request"
    )]
    #[case::unknown_pull_request(
        vec![entry("schema", &[], "acme/notify-api", "leaf-schema")],
        vec![step(None, &["missing"])],
        "step 1 references unknown pull request id `missing`"
    )]
    #[case::pull_request_in_multiple_steps(
        vec![entry("schema", &[], "acme/notify-api", "leaf-schema")],
        vec![step(None, &["schema"]), step(None, &["schema"])],
        "pull request `schema` belongs to multiple steps"
    )]
    #[case::duplicate_pull_request_in_one_step(
        vec![entry("schema", &[], "acme/notify-api", "leaf-schema")],
        vec![step(Some("Repeat the schema work"), &["schema", "schema"])],
        "pull request `schema` belongs to multiple steps"
    )]
    #[case::pull_request_without_step(
        vec![entry("schema", &[], "acme/notify-api", "leaf-schema")],
        vec![],
        "pull request `schema` does not belong to a step"
    )]
    #[case::dependency_in_same_step(
        vec![entry("schema", &[], "acme/notify-api", "leaf-schema"), entry("settings", &["schema"], "acme/notify-web", "leaf-settings")],
        vec![step(Some("Build and document the settings"), &["schema", "settings"])],
        "pull request `settings` depends on `schema` from the same or a later step"
    )]
    #[case::dependency_in_later_step(
        vec![entry("schema", &["settings"], "acme/notify-api", "leaf-schema"), entry("settings", &[], "acme/notify-web", "leaf-settings")],
        vec![step(None, &["schema"]), step(None, &["settings"])],
        "pull request `schema` depends on `settings` from the same or a later step"
    )]
    #[case::multiline_step_summary(
        vec![entry("schema", &[], "acme/notify-api", "leaf-schema")],
        vec![step(
            Some(indoc! {"
                first line
                second line
            "}),
            &["schema"]
        )],
        "step summary must be a single line"
    )]
    fn rejects_invalid_steps(
        #[case] entries: Vec<PrSeriesEntry>,
        #[case] steps: Vec<PrSeriesStep>,
        #[case] expected_error: &str,
    ) {
        assert_eq!(
            plan_with_steps(entries, steps)
                .validate()
                .err()
                .map(|error| error.to_string()),
            Some(expected_error.to_string()),
        );
    }

    #[rstest]
    #[case::legacy_without_steps(
        indoc! {r#"
            {
              "name": "Notification preferences",
              "purpose": "Let users choose which notifications they receive.",
              "pullRequests": [
                {
                  "id": "schema",
                  "summary": "Add preference tables",
                  "dependsOn": [],
                  "repository": "acme/notify-api",
                  "tqLeafId": "leaf-schema"
                }
              ]
            }
        "#},
        plan_with_entries(vec![PrSeriesEntry {
            id: "schema".to_string(),
            summary: "Add preference tables".to_string(),
            depends_on: Vec::new(),
            repository: "acme/notify-api".to_string(),
            tq_leaf_id: "leaf-schema".to_string(),
        }]),
    )]
    #[case::steps_use_camel_case_with_optional_summary(
        indoc! {r#"
            {
              "name": "Notification preferences",
              "purpose": "Let users choose which notifications they receive.",
              "pullRequests": [
                {
                  "id": "schema",
                  "summary": "Add preference tables",
                  "dependsOn": [],
                  "repository": "acme/notify-api",
                  "tqLeafId": "leaf-schema"
                },
                {
                  "id": "settings",
                  "summary": "Add the settings page",
                  "dependsOn": ["schema"],
                  "repository": "acme/notify-web",
                  "tqLeafId": "leaf-settings"
                }
              ],
              "steps": [
                { "pullRequests": ["schema"] },
                {
                  "summary": "Add and document the settings experience",
                  "pullRequests": ["settings"]
                }
              ]
            }
        "#},
        plan_with_steps(
            vec![
                PrSeriesEntry {
                    id: "schema".to_string(),
                    summary: "Add preference tables".to_string(),
                    depends_on: Vec::new(),
                    repository: "acme/notify-api".to_string(),
                    tq_leaf_id: "leaf-schema".to_string(),
                },
                PrSeriesEntry {
                    id: "settings".to_string(),
                    summary: "Add the settings page".to_string(),
                    depends_on: vec!["schema".to_string()],
                    repository: "acme/notify-web".to_string(),
                    tq_leaf_id: "leaf-settings".to_string(),
                },
            ],
            vec![
                step(None, &["schema"]),
                step(
                    Some("Add and document the settings experience"),
                    &["settings"],
                ),
            ],
        ),
    )]
    fn reads_plan_json(#[case] json: &str, #[case] expected: PrSeriesPlan) {
        let plan: PrSeriesPlan = serde_json::from_str(json).unwrap();

        assert_eq!(plan, expected);
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

    #[fixture]
    fn single_pr_plan() -> PrSeriesPlan {
        plan_with_entries(vec![entry("schema", &[], "acme/notify-api", "leaf-schema")])
    }

    #[fixture]
    fn linked_checklists() -> Vec<TqChecklist> {
        vec![TqChecklist {
            id: "checklist-1".to_string(),
            name: "Implementation".to_string(),
            items: vec![TqChecklistItem {
                id: "leaf-schema".to_string(),
                content: "Add preference tables".to_string(),
                checked_at: None,
                github_link_id: Some("link-schema".to_string()),
                children: Vec::new(),
            }],
        }]
    }

    #[rstest]
    #[case::matching_repository(
        TqGithubLink {
            id: "link-schema".to_string(),
            kind: Some("pull_request".to_string()),
            owner: Some("ACME".to_string()),
            repo: Some("notify-api".to_string()),
            number: Some(17),
            title: Some("Add preference tables".to_string()),
            url: Some("https://github.com/ACME/notify-api/pull/17".to_string()),
            state: Some("open".to_string()),
        },
        Ok(vec![CreatedPullRequest {
            entry_id: "schema".to_string(),
            repository: Repository {
                owner: "acme".to_string(),
                name: "notify-api".to_string(),
            },
            number: 17,
            url: "https://github.com/acme/notify-api/pull/17".to_string(),
        }]),
    )]
    #[case::different_repository(
        TqGithubLink {
            id: "link-schema".to_string(),
            kind: Some("pull_request".to_string()),
            owner: Some("acme".to_string()),
            repo: Some("notify-other".to_string()),
            number: Some(23),
            title: Some("Add preference tables".to_string()),
            url: Some("https://github.com/acme/notify-other/pull/23".to_string()),
            state: Some("open".to_string()),
        },
        Err("tq pull request link `link-schema` points to `acme/notify-other`, expected `acme/notify-api`".to_string()),
    )]
    fn resolves_created_pull_requests(
        single_pr_plan: PrSeriesPlan,
        linked_checklists: Vec<TqChecklist>,
        #[case] github_link: TqGithubLink,
        #[case] expected: std::result::Result<Vec<CreatedPullRequest>, String>,
    ) {
        let actual =
            resolve_created_pull_requests(&single_pr_plan, &linked_checklists, &[github_link])
                .map_err(|error| error.to_string());

        assert_eq!(actual, expected);
    }
}
