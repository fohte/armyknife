use std::collections::{BTreeSet, HashMap};

use anyhow::{Context, Result, ensure};
use indoc::indoc;

use super::model::{CreatedPullRequest, PrSeriesPlan, Repository};

const START_MARKER: &str = "<!-- pr-series:start -->";
const END_MARKER: &str = "<!-- pr-series:end -->";

pub fn render_section(
    plan: &PrSeriesPlan,
    current_entry_id: &str,
    current_body: &str,
    repository_privacy: &HashMap<String, bool>,
    created_pull_requests: &[CreatedPullRequest],
) -> Result<String> {
    let current = plan
        .pull_requests
        .iter()
        .find(|entry| entry.id == current_entry_id)
        .with_context(|| format!("pull request `{current_entry_id}` is missing from the plan"))?;
    let current_repository = Repository::parse(&current.repository)?;
    let current_is_private = *repository_privacy
        .get(&current_repository.key())
        .with_context(|| format!("privacy is unknown for {}", current_repository.full_name()))?;
    let body_language_source = body_without_managed_section(current_body)?;
    let mut visible_entries = Vec::new();
    for entry in &plan.pull_requests {
        let repository = Repository::parse(&entry.repository)?;
        let is_private = *repository_privacy
            .get(&repository.key())
            .with_context(|| format!("privacy is unknown for {}", repository.full_name()))?;
        if current_is_private || !is_private {
            visible_entries.push((entry, repository));
        }
    }
    ensure!(
        visible_entries
            .iter()
            .any(|(entry, _)| entry.id == current_entry_id),
        "current pull request `{current_entry_id}` is hidden by repository privacy filtering"
    );

    let visible_ids = visible_entries
        .iter()
        .map(|(entry, _)| entry.id.as_str())
        .collect::<BTreeSet<_>>();
    let created_by_id = created_pull_requests
        .iter()
        .map(|pull_request| (pull_request.entry_id.as_str(), pull_request))
        .collect::<HashMap<_, _>>();
    let node_ids = visible_entries
        .iter()
        .enumerate()
        .map(|(index, (entry, _))| (entry.id.as_str(), format!("P{}", index + 1)))
        .collect::<HashMap<_, _>>();

    // GitHub's 120px default clips labels; 400px exceeds MAX_LABEL_WIDTH_PX (200px).
    let mut diagram =
        String::from("---\nconfig:\n  flowchart:\n    wrappingWidth: 400\n---\nflowchart LR\n");
    let repositories = visible_entries
        .iter()
        .map(|(_, repository)| repository.key())
        .collect::<BTreeSet<_>>();
    if repositories.len() > 1 {
        for (repository_index, repository_key) in repositories.iter().enumerate() {
            let repository = visible_entries
                .iter()
                .find(|(_, repository)| repository.key() == *repository_key)
                .map(|(_, repository)| repository)
                .context("visible repository was not found")?;
            diagram.push_str(&format!(
                "  subgraph repo{repository_index}[\"{}\"]\n",
                escape_mermaid_text(&repository.full_name())
            ));
            append_nodes(
                &mut diagram,
                visible_entries
                    .iter()
                    .filter(|(_, candidate)| candidate.key() == *repository_key),
                &node_ids,
                current_entry_id,
                &body_language_source,
                "    ",
            );
            diagram.push_str("  end\n");
        }
    } else {
        append_nodes(
            &mut diagram,
            visible_entries.iter(),
            &node_ids,
            current_entry_id,
            &body_language_source,
            "  ",
        );
    }

    for (entry, _) in &visible_entries {
        for dependency in &entry.depends_on {
            if !visible_ids.contains(dependency.as_str()) {
                continue;
            }
            let from = node_ids
                .get(dependency.as_str())
                .context("dependency node was not rendered")?;
            let to = node_ids
                .get(entry.id.as_str())
                .context("pull request node was not rendered")?;
            diagram.push_str(&format!("  {from} --> {to}\n"));
        }
    }

    for (entry, _) in &visible_entries {
        if let Some(pull_request) = created_by_id.get(entry.id.as_str()) {
            let node_id = node_ids
                .get(entry.id.as_str())
                .context("pull request node was not rendered")?;
            diagram.push_str(&format!(
                "  click {node_id} href \"{}\" _blank\n",
                pull_request.url
            ));
        }
    }
    diagram.push_str("  classDef current stroke:#3fb950,stroke-width:3px\n");
    let planned_nodes = visible_entries
        .iter()
        .filter(|(entry, _)| !created_by_id.contains_key(entry.id.as_str()))
        .filter_map(|(entry, _)| node_ids.get(entry.id.as_str()))
        .map(String::as_str)
        .collect::<Vec<_>>();
    if !planned_nodes.is_empty() {
        diagram.push_str("  classDef planned stroke-dasharray:5 5\n");
    }
    let current_node = node_ids
        .get(current_entry_id)
        .context("current pull request node was not rendered")?;
    diagram.push_str(&format!("  class {current_node} current\n"));
    if !planned_nodes.is_empty() {
        diagram.push_str(&format!("  class {} planned\n", planned_nodes.join(",")));
    }

    let section = format!(
        indoc! {"
            {START_MARKER}
            ### {name}

            {purpose}

            ```mermaid
            {diagram}```
            {END_MARKER}
        "},
        START_MARKER = START_MARKER,
        END_MARKER = END_MARKER,
        name = plan.name,
        purpose = plan.purpose,
        diagram = diagram,
    );
    Ok(section.trim_end_matches('\n').to_string())
}

pub fn replace_managed_section(body: &str, section: &str) -> Result<String> {
    let Some(section_range) = managed_section_range(body)? else {
        if body.is_empty() {
            return Ok(section.to_string());
        }
        let separator = if body.ends_with('\n') {
            "\n"
        } else {
            concat!("\n", "\n")
        };
        return Ok(format!("{body}{separator}{section}"));
    };

    Ok(format!(
        "{}{}{}",
        &body[..section_range.start],
        section,
        &body[section_range.end..]
    ))
}

pub fn preserve_managed_section(existing_body: &str, submitted_body: &str) -> Result<String> {
    if managed_section_range(submitted_body)?.is_some() {
        return Ok(submitted_body.to_string());
    }
    let Some(existing_range) = managed_section_range(existing_body)? else {
        return Ok(submitted_body.to_string());
    };
    replace_managed_section(submitted_body, &existing_body[existing_range])
}

fn body_without_managed_section(body: &str) -> Result<String> {
    let Some(section_range) = managed_section_range(body)? else {
        return Ok(body.to_string());
    };
    Ok(format!(
        "{}{}",
        &body[..section_range.start],
        &body[section_range.end..]
    ))
}

fn managed_section_range(body: &str) -> Result<Option<std::ops::Range<usize>>> {
    let starts = body.match_indices(START_MARKER).collect::<Vec<_>>();
    let ends = body.match_indices(END_MARKER).collect::<Vec<_>>();
    if starts.is_empty() && ends.is_empty() {
        return Ok(None);
    }
    ensure!(
        starts.len() == 1 && ends.len() == 1,
        "PR series markers must occur exactly once"
    );

    let (start_index, _) = starts[0];
    let (end_index, _) = ends[0];
    ensure!(
        start_index < end_index,
        "PR series markers are out of order"
    );
    ensure!(
        marker_is_on_its_own_line(body, start_index, START_MARKER)
            && marker_is_on_its_own_line(body, end_index, END_MARKER),
        "PR series markers must be on their own lines"
    );
    Ok(Some(start_index..end_index + END_MARKER.len()))
}

fn append_nodes<'a>(
    output: &mut String,
    entries: impl Iterator<Item = &'a (&'a super::model::PrSeriesEntry, Repository)>,
    node_ids: &HashMap<&str, String>,
    current_entry_id: &str,
    body_language_source: &str,
    indent: &str,
) {
    for (entry, _) in entries {
        let Some(node_id) = node_ids.get(entry.id.as_str()) else {
            continue;
        };
        let current_label = if entry.id == current_entry_id {
            if contains_japanese(body_language_source) {
                "<br/>今回の PR"
            } else {
                "<br/>This PR"
            }
        } else {
            ""
        };
        output.push_str(&format!(
            "{indent}{node_id}(\"{}{current_label}\")\n",
            escape_mermaid_text(&entry.summary)
        ));
    }
}

fn marker_is_on_its_own_line(body: &str, index: usize, marker: &str) -> bool {
    let line_start = index == 0 || matches!(body.as_bytes().get(index - 1), Some(b'\n' | b'\r'));
    let after = index + marker.len();
    let line_end = after == body.len() || matches!(body.as_bytes().get(after), Some(b'\n' | b'\r'));
    line_start && line_end
}

fn escape_mermaid_text(value: &str) -> String {
    value
        .chars()
        .map(|character| match character {
            '&' => "#amp;".to_string(),
            '"' => "#quot;".to_string(),
            '<' => "#lt;".to_string(),
            '>' => "#gt;".to_string(),
            _ => character.to_string(),
        })
        .collect()
}

fn contains_japanese(value: &str) -> bool {
    value.chars().any(|character| {
        matches!(
            character as u32,
            0x3040..=0x30ff | 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff
        )
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::commands::ai::pr_series::model::{PrSeriesEntry, PrSeriesPlan};

    fn plan() -> PrSeriesPlan {
        PrSeriesPlan {
            name: "Notification preferences".to_string(),
            purpose: "Let users choose which notifications they receive.".to_string(),
            pull_requests: vec![
                PrSeriesEntry {
                    id: "schema".to_string(),
                    summary: "Add preference tables".to_string(),
                    depends_on: Vec::new(),
                    repository: "acme/notify-api".to_string(),
                    tq_leaf_id: "leaf-schema".to_string(),
                },
                PrSeriesEntry {
                    id: "credentials".to_string(),
                    summary: "Rotate mail credentials".to_string(),
                    depends_on: vec!["schema".to_string()],
                    repository: "acme/private-infra".to_string(),
                    tq_leaf_id: "leaf-credentials".to_string(),
                },
                PrSeriesEntry {
                    id: "docs".to_string(),
                    summary: "Document the settings".to_string(),
                    depends_on: vec!["credentials".to_string()],
                    repository: "acme/notify-docs".to_string(),
                    tq_leaf_id: "leaf-docs".to_string(),
                },
            ],
        }
    }

    fn privacy() -> HashMap<String, bool> {
        HashMap::from([
            ("acme/notify-api".to_string(), false),
            ("acme/private-infra".to_string(), true),
            ("acme/notify-docs".to_string(), false),
        ])
    }

    fn created_docs() -> Vec<CreatedPullRequest> {
        vec![CreatedPullRequest {
            entry_id: "docs".to_string(),
            repository: Repository {
                owner: "acme".to_string(),
                name: "notify-docs".to_string(),
            },
            number: 31,
            url: "https://github.com/acme/notify-docs/pull/31".to_string(),
        }]
    }

    fn created_private_infra() -> Vec<CreatedPullRequest> {
        vec![
            created_docs().remove(0),
            CreatedPullRequest {
                entry_id: "credentials".to_string(),
                repository: Repository {
                    owner: "acme".to_string(),
                    name: "private-infra".to_string(),
                },
                number: 19,
                url: "https://github.com/acme/private-infra/pull/19".to_string(),
            },
        ]
    }

    #[test]
    fn public_repository_views_remove_private_nodes_and_incident_edges_without_shortcuts() {
        assert_eq!(
            render_section(
                &plan(),
                "docs",
                "Existing body",
                &privacy(),
                &created_docs()
            )
            .unwrap(),
            "<!-- pr-series:start -->\n### Notification preferences\n\nLet users choose which notifications they receive.\n\n```mermaid\n---\nconfig:\n  flowchart:\n    wrappingWidth: 400\n---\nflowchart LR\n  subgraph repo0[\"acme/notify-api\"]\n    P1(\"Add preference tables\")\n  end\n  subgraph repo1[\"acme/notify-docs\"]\n    P2(\"Document the settings<br/>This PR\")\n  end\n  click P2 href \"https://github.com/acme/notify-docs/pull/31\" _blank\n  classDef current stroke:#3fb950,stroke-width:3px\n  classDef planned stroke-dasharray:5 5\n  class P2 current\n  class P1 planned\n```\n<!-- pr-series:end -->",
        );
    }

    #[test]
    fn private_repository_views_include_private_dependencies() {
        let expected = "<!-- pr-series:start -->\n### Notification preferences\n\nLet users choose which notifications they receive.\n\n```mermaid\n---\nconfig:\n  flowchart:\n    wrappingWidth: 400\n---\nflowchart LR\n  subgraph repo0[\"acme/notify-api\"]\n    P1(\"Add preference tables\")\n  end\n  subgraph repo1[\"acme/notify-docs\"]\n    P3(\"Document the settings\")\n  end\n  subgraph repo2[\"acme/private-infra\"]\n    P2(\"Rotate mail credentials<br/>今回の PR\")\n  end\n  P1 --> P2\n  P2 --> P3\n  click P2 href \"https://github.com/acme/private-infra/pull/19\" _blank\n  click P3 href \"https://github.com/acme/notify-docs/pull/31\" _blank\n  classDef current stroke:#3fb950,stroke-width:3px\n  classDef planned stroke-dasharray:5 5\n  class P2 current\n  class P1 planned\n```\n<!-- pr-series:end -->";

        assert_eq!(
            render_section(
                &plan(),
                "credentials",
                "設定本文",
                &privacy(),
                &created_private_infra(),
            )
            .unwrap(),
            expected,
        );
    }

    #[test]
    fn language_detection_ignores_the_managed_section_on_reruns() {
        let mut plan = plan();
        plan.name = "通知設定".to_string();
        let privacy = privacy();
        let created = created_docs();
        let initial_body = "Existing English body";
        let first_section =
            render_section(&plan, "docs", initial_body, &privacy, &created).unwrap();
        let first_body = replace_managed_section(initial_body, &first_section).unwrap();
        let second_section =
            render_section(&plan, "docs", &first_body, &privacy, &created).unwrap();
        let second_body = replace_managed_section(&first_body, &second_section).unwrap();
        let expected = indoc! {"
            Existing English body

            <!-- pr-series:start -->
            ### 通知設定

            Let users choose which notifications they receive.

            ```mermaid
            ---
            config:
              flowchart:
                wrappingWidth: 400
            ---
            flowchart LR
              subgraph repo0[\"acme/notify-api\"]
                P1(\"Add preference tables\")
              end
              subgraph repo1[\"acme/notify-docs\"]
                P2(\"Document the settings<br/>This PR\")
              end
              click P2 href \"https://github.com/acme/notify-docs/pull/31\" _blank
              classDef current stroke:#3fb950,stroke-width:3px
              classDef planned stroke-dasharray:5 5
              class P2 current
              class P1 planned
            ```
            <!-- pr-series:end -->
        "}
        .trim_end_matches('\n')
        .to_string();

        assert_eq!((first_body, second_body), (expected.clone(), expected));
    }

    #[test]
    fn replaces_the_marked_section_and_preserves_both_sides() {
        let section = indoc! {"
            <!-- pr-series:start -->
            ### New series

            Purpose

            ```mermaid
            flowchart LR
            ```
            <!-- pr-series:end -->
        "}
        .trim_end_matches('\n');
        let initial = indoc! {"
            Title
            Outside before
            <!-- pr-series:start -->
            Old section
            <!-- pr-series:end -->
            Outside after
        "}
        .trim_end_matches('\n');
        let expected = indoc! {"
            Title
            Outside before
            <!-- pr-series:start -->
            ### New series

            Purpose

            ```mermaid
            flowchart LR
            ```
            <!-- pr-series:end -->
            Outside after
        "}
        .trim_end_matches('\n')
        .to_string();
        let first = replace_managed_section(initial, section).unwrap();

        assert_eq!(
            (
                first.clone(),
                replace_managed_section(&first, section).unwrap()
            ),
            (expected.clone(), expected),
        );
    }

    #[test]
    fn appends_the_section_when_no_markers_exist() {
        let section = indoc! {"
            <!-- pr-series:start -->
            ### New series

            Purpose

            ```mermaid
            flowchart LR
            ```
            <!-- pr-series:end -->
        "}
        .trim_end_matches('\n');

        assert_eq!(
            replace_managed_section("Existing text", section).unwrap(),
            indoc! {"
                Existing text

                <!-- pr-series:start -->
                ### New series

                Purpose

                ```mermaid
                flowchart LR
                ```
                <!-- pr-series:end -->
            "}
            .trim_end_matches('\n'),
        );
    }

    #[rstest::rstest]
    #[case::submitted_section_wins(
        (indoc! {"
            Existing body

            <!-- pr-series:start -->
            Existing diagram
            <!-- pr-series:end -->
        "}).trim_end_matches('\n'),
        (indoc! {"
            Updated body

            <!-- pr-series:start -->
            Draft diagram
            <!-- pr-series:end -->
        "}).trim_end_matches('\n'),
        Ok((indoc! {"
            Updated body

            <!-- pr-series:start -->
            Draft diagram
            <!-- pr-series:end -->
        "}).trim_end_matches('\n')),
    )]
    #[case::existing_without_section("Existing body", "Updated body", Ok("Updated body"))]
    #[case::existing_section_is_carried_forward(
        (indoc! {"
            Existing body

            <!-- pr-series:start -->
            Existing diagram
            <!-- pr-series:end -->
        "}).trim_end_matches('\n'),
        "Updated body",
        Ok((indoc! {"
            Updated body

            <!-- pr-series:start -->
            Existing diagram
            <!-- pr-series:end -->
        "}).trim_end_matches('\n')),
    )]
    #[case::existing_partial_marker(
        (indoc! {"
            Existing body
            <!-- pr-series:start -->
        "}).trim_end_matches('\n'),
        "Updated body",
        Err("PR series markers must occur exactly once"),
    )]
    #[case::submitted_partial_marker(
        (indoc! {"
            Existing body

            <!-- pr-series:start -->
            Existing diagram
            <!-- pr-series:end -->
        "}).trim_end_matches('\n'),
        (indoc! {"
            Updated body
            <!-- pr-series:start -->
        "}).trim_end_matches('\n'),
        Err("PR series markers must occur exactly once"),
    )]
    fn preserves_the_existing_managed_section(
        #[case] existing_body: &str,
        #[case] submitted_body: &str,
        #[case] expected: std::result::Result<&str, &str>,
    ) {
        assert_eq!(
            preserve_managed_section(existing_body, submitted_body)
                .map_err(|error| error.to_string()),
            expected.map(str::to_string).map_err(str::to_string),
        );
    }

    #[rstest::rstest]
    #[case::partial("<!-- pr-series:start -->\nbody")]
    #[case::duplicate(indoc! {"
        <!-- pr-series:start -->
        <!-- pr-series:start -->
        <!-- pr-series:end -->
    "})]
    fn rejects_partial_or_duplicate_markers(#[case] body: &str) {
        assert_eq!(
            replace_managed_section(body, "section")
                .err()
                .map(|error| error.to_string()),
            Some("PR series markers must occur exactly once".to_string()),
        );
    }
}
