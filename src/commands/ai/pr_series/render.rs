use std::collections::HashMap;

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

    let visible_by_id = visible_entries
        .iter()
        .map(|(entry, _)| (entry.id.as_str(), *entry))
        .collect::<HashMap<_, _>>();
    let created_by_id = created_pull_requests
        .iter()
        .map(|pull_request| (pull_request.entry_id.as_str(), pull_request))
        .collect::<HashMap<_, _>>();
    let body_is_japanese = contains_japanese(&body_language_source);
    let render_entry =
        |entry| render_pull_request(entry, current_entry_id, body_is_japanese, &created_by_id);
    let mut rendered_steps = Vec::new();

    if let Some(steps) = &plan.steps {
        for step in steps {
            let entries = step
                .pull_requests
                .iter()
                .filter_map(|id| visible_by_id.get(id.as_str()).copied())
                .collect::<Vec<_>>();
            match entries.as_slice() {
                [] => {}
                [entry] => rendered_steps.push(render_entry(entry)),
                _ => {
                    let summary = step.summary.as_deref().context(
                        "step summary is required when multiple pull requests are visible",
                    )?;
                    let step_number = rendered_steps.len() + 1;
                    let indent = " ".repeat(step_number.to_string().len() + 2);
                    let mut rendered_group = vec![escape_markdown_text(summary)];
                    for entry in entries {
                        rendered_group.push(format!("{indent}- {}", render_entry(entry)));
                    }
                    rendered_steps.push(rendered_group.join("\n"));
                }
            }
        }
    } else {
        for (entry, _) in &visible_entries {
            rendered_steps.push(render_entry(entry));
        }
    }

    let numbered_steps = rendered_steps
        .into_iter()
        .enumerate()
        .map(|(index, step)| format!("{}. {step}", index + 1))
        .collect::<Vec<_>>()
        .join("\n");
    let section = format!(
        indoc! {"
            {START_MARKER}
            ### {name}

            {steps}
            {END_MARKER}
        "},
        START_MARKER = START_MARKER,
        END_MARKER = END_MARKER,
        name = escape_markdown_heading(&plan.name),
        steps = numbered_steps,
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

fn render_pull_request(
    entry: &super::model::PrSeriesEntry,
    current_entry_id: &str,
    body_is_japanese: bool,
    created_by_id: &HashMap<&str, &CreatedPullRequest>,
) -> String {
    if entry.id == current_entry_id {
        let current_label = if body_is_japanese {
            "今回の PR"
        } else {
            "this PR"
        };
        format!(
            "**{}** ({current_label})",
            escape_markdown_text(&entry.summary)
        )
    } else if let Some(pull_request) = created_by_id.get(entry.id.as_str()) {
        format!(
            "{}#{}",
            pull_request.repository.full_name(),
            pull_request.number
        )
    } else {
        let not_created_label = if body_is_japanese {
            "未作成"
        } else {
            "not created yet"
        };
        format!(
            "{} ({not_created_label})",
            escape_markdown_text(&entry.summary)
        )
    }
}

fn marker_is_on_its_own_line(body: &str, index: usize, marker: &str) -> bool {
    let line_start = index == 0 || matches!(body.as_bytes().get(index - 1), Some(b'\n' | b'\r'));
    let after = index + marker.len();
    let line_end = after == body.len() || matches!(body.as_bytes().get(after), Some(b'\n' | b'\r'));
    line_start && line_end
}

#[derive(Clone, Copy)]
enum MarkdownEscapeContext {
    Text,
    Heading,
}

fn escape_markdown_text(value: &str) -> String {
    escape_markdown(value, MarkdownEscapeContext::Text)
}

fn escape_markdown_heading(value: &str) -> String {
    escape_markdown(value, MarkdownEscapeContext::Heading)
}

fn escape_markdown(value: &str, context: MarkdownEscapeContext) -> String {
    let characters: Vec<char> = match context {
        MarkdownEscapeContext::Text => {
            // GitHub preserves full-width spaces and NBSP, so trimming them changes the rendered text.
            value.trim_start_matches([' ', '\t']).chars().collect()
        }
        MarkdownEscapeContext::Heading => value.chars().collect(),
    };
    let leading_digit_end = characters
        .iter()
        .take_while(|character| character.is_ascii_digit())
        .count();
    let has_leading_ordered_list_marker = leading_digit_end > 0
        && matches!(characters.get(leading_digit_end), Some(&'.') | Some(&')'));
    let closing_hash_index = match context {
        MarkdownEscapeContext::Text => None,
        MarkdownEscapeContext::Heading => atx_closing_hash_index(&characters),
    };

    characters
        .iter()
        .enumerate()
        .map(|(index, character)| {
            // CommonMark cannot open or close emphasis with an underscore between alphanumerics.
            let has_alphanumeric_neighbors = index > 0
                && index + 1 < characters.len()
                && characters[index - 1].is_alphanumeric()
                && characters[index + 1].is_alphanumeric();
            let is_leading_block_marker = matches!(context, MarkdownEscapeContext::Text)
                && index == 0
                && matches!(character, '#' | '>' | '-' | '+');
            let is_leading_ordered_list_marker = matches!(context, MarkdownEscapeContext::Text)
                && has_leading_ordered_list_marker
                && index == leading_digit_end;
            // GFM autolinks preserve escapes such as \& in the label and href, so break the "://" and "www." triggers to keep URLs as plain text.
            let is_url_scheme_delimiter = *character == ':'
                && characters.get(index + 1) == Some(&'/')
                && characters.get(index + 2) == Some(&'/');
            let is_www_domain_delimiter = *character == '.'
                && index >= 3
                && characters[index - 3] == 'w'
                && characters[index - 2] == 'w'
                && characters[index - 1] == 'w';
            let should_escape = matches!(character, '\\' | '`' | '*' | '~' | '[' | ']' | '<' | '&')
                || (*character == '_' && !has_alphanumeric_neighbors)
                || is_leading_block_marker
                || is_leading_ordered_list_marker
                || is_url_scheme_delimiter
                || is_www_domain_delimiter
                || closing_hash_index == Some(index);

            if should_escape {
                format!("\\{character}")
            } else {
                character.to_string()
            }
        })
        .collect()
}

fn atx_closing_hash_index(characters: &[char]) -> Option<usize> {
    let last_content_index = characters
        .iter()
        .rposition(|character| *character != ' ' && *character != '\t')?;
    if characters[last_content_index] != '#' {
        return None;
    }

    let mut hash_start = last_content_index;
    while hash_start > 0 && characters[hash_start - 1] == '#' {
        hash_start -= 1;
    }
    (hash_start == 0 || matches!(characters[hash_start - 1], ' ' | '\t')).then_some(hash_start)
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

    use indoc::indoc;

    use super::*;
    use crate::commands::ai::pr_series::model::{PrSeriesEntry, PrSeriesPlan, PrSeriesStep};

    fn plan() -> PrSeriesPlan {
        PrSeriesPlan {
            name: "Notification preferences".to_string(),
            purpose: "Let users choose which notifications they receive.".to_string(),
            steps: None,
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

    #[derive(Debug)]
    struct RenderCase {
        plan: PrSeriesPlan,
        current_entry_id: &'static str,
        current_body: &'static str,
        repository_privacy: HashMap<String, bool>,
        created_pull_requests: Vec<CreatedPullRequest>,
        expected: String,
    }

    fn legacy_render_case() -> RenderCase {
        RenderCase {
            plan: plan(),
            current_entry_id: "docs",
            current_body: "Existing body",
            repository_privacy: privacy(),
            created_pull_requests: created_docs(),
            expected: indoc! {"
                <!-- pr-series:start -->
                ### Notification preferences

                1. Add preference tables (not created yet)
                2. **Document the settings** (this PR)
                <!-- pr-series:end -->
            "}
            .trim_end_matches('\n')
            .to_string(),
        }
    }

    fn grouped_render_case() -> RenderCase {
        let mut plan = plan();
        plan.steps = Some(vec![
            PrSeriesStep {
                summary: None,
                pull_requests: vec!["credentials".to_string()],
            },
            PrSeriesStep {
                summary: Some("Add preference storage and document the settings.".to_string()),
                pull_requests: vec!["schema".to_string(), "docs".to_string()],
            },
        ]);
        RenderCase {
            plan,
            current_entry_id: "docs",
            current_body: "Existing body",
            repository_privacy: privacy(),
            created_pull_requests: created_docs(),
            expected: indoc! {"
                <!-- pr-series:start -->
                ### Notification preferences

                1. Add preference storage and document the settings.
                   - Add preference tables (not created yet)
                   - **Document the settings** (this PR)
                <!-- pr-series:end -->
            "}
            .trim_end_matches('\n')
            .to_string(),
        }
    }

    fn private_japanese_render_case() -> RenderCase {
        RenderCase {
            plan: plan(),
            current_entry_id: "credentials",
            current_body: "設定本文",
            repository_privacy: privacy(),
            created_pull_requests: created_private_infra(),
            expected: indoc! {"
                <!-- pr-series:start -->
                ### Notification preferences

                1. Add preference tables (未作成)
                2. **Rotate mail credentials** (今回の PR)
                3. acme/notify-docs#31
                <!-- pr-series:end -->
            "}
            .trim_end_matches('\n')
            .to_string(),
        }
    }

    fn one_visible_pull_request_render_case() -> RenderCase {
        let mut plan = plan();
        plan.steps = Some(vec![PrSeriesStep {
            summary: Some("Set up storage and rotate credentials.".to_string()),
            pull_requests: vec!["credentials".to_string(), "docs".to_string()],
        }]);
        RenderCase {
            plan,
            current_entry_id: "docs",
            current_body: "Existing body",
            repository_privacy: privacy(),
            created_pull_requests: created_docs(),
            expected: indoc! {"
                <!-- pr-series:start -->
                ### Notification preferences

                1. **Document the settings** (this PR)
                <!-- pr-series:end -->
            "}
            .trim_end_matches('\n')
            .to_string(),
        }
    }

    fn escaped_summary_render_case() -> RenderCase {
        let mut plan = plan();
        plan.pull_requests[0].summary = "Build [preference] tables".to_string();
        plan.pull_requests[2].summary = "Document `settings`".to_string();
        plan.steps = Some(vec![PrSeriesStep {
            summary: Some("Group *storage* with [docs](guide)".to_string()),
            pull_requests: vec!["schema".to_string(), "docs".to_string()],
        }]);
        RenderCase {
            plan,
            current_entry_id: "docs",
            current_body: "Existing body",
            repository_privacy: privacy(),
            created_pull_requests: created_docs(),
            expected: indoc! {r"
                <!-- pr-series:start -->
                ### Notification preferences

                1. Group \*storage\* with \[docs\](guide)
                   - Build \[preference\] tables (not created yet)
                   - **Document \`settings\`** (this PR)
                <!-- pr-series:end -->
            "}
            .trim_end_matches('\n')
            .to_string(),
        }
    }

    fn created_non_current_nested_pr_render_case() -> RenderCase {
        let mut plan = plan();
        plan.steps = Some(vec![
            PrSeriesStep {
                summary: Some("Store preferences and document the settings.".to_string()),
                pull_requests: vec!["schema".to_string(), "docs".to_string()],
            },
            PrSeriesStep {
                summary: None,
                pull_requests: vec!["credentials".to_string()],
            },
        ]);
        plan.pull_requests[2].depends_on.clear();
        let created = vec![CreatedPullRequest {
            entry_id: "schema".to_string(),
            repository: Repository {
                owner: "acme".to_string(),
                name: "notify-api".to_string(),
            },
            number: 12,
            url: "https://github.com/acme/notify-api/pull/12".to_string(),
        }];
        RenderCase {
            plan,
            current_entry_id: "docs",
            current_body: "Existing body",
            repository_privacy: privacy(),
            created_pull_requests: created,
            expected: indoc! {"
                <!-- pr-series:start -->
                ### Notification preferences

                1. Store preferences and document the settings.
                   - acme/notify-api#12
                   - **Document the settings** (this PR)
                <!-- pr-series:end -->
            "}
            .trim_end_matches('\n')
            .to_string(),
        }
    }

    fn tenth_step_nested_list_render_case() -> RenderCase {
        let pull_requests = (1..=11)
            .map(|number| PrSeriesEntry {
                id: format!("task-{number}"),
                summary: format!("Implement notification stage {number}"),
                depends_on: Vec::new(),
                repository: "example/notify-api".to_string(),
                tq_leaf_id: format!("leaf-{number}"),
            })
            .collect();
        let steps = (1..=9)
            .map(|number| PrSeriesStep {
                summary: None,
                pull_requests: vec![format!("task-{number}")],
            })
            .chain(std::iter::once(PrSeriesStep {
                summary: Some("Complete the final notification step".to_string()),
                pull_requests: vec!["task-10".to_string(), "task-11".to_string()],
            }))
            .collect();
        RenderCase {
            plan: PrSeriesPlan {
                name: "Notification workflow".to_string(),
                purpose: "Implement notification delivery in stages.".to_string(),
                pull_requests,
                steps: Some(steps),
            },
            current_entry_id: "task-11",
            current_body: "Existing body",
            repository_privacy: HashMap::from([("example/notify-api".to_string(), false)]),
            created_pull_requests: Vec::new(),
            expected: indoc! {"
                <!-- pr-series:start -->
                ### Notification workflow

                1. Implement notification stage 1 (not created yet)
                2. Implement notification stage 2 (not created yet)
                3. Implement notification stage 3 (not created yet)
                4. Implement notification stage 4 (not created yet)
                5. Implement notification stage 5 (not created yet)
                6. Implement notification stage 6 (not created yet)
                7. Implement notification stage 7 (not created yet)
                8. Implement notification stage 8 (not created yet)
                9. Implement notification stage 9 (not created yet)
                10. Complete the final notification step
                    - Implement notification stage 10 (not created yet)
                    - **Implement notification stage 11** (this PR)
                <!-- pr-series:end -->
            "}
            .trim_end_matches('\n')
            .to_string(),
        }
    }

    #[rstest::rstest]
    #[case::legacy_plan(legacy_render_case())]
    #[case::grouped_step_filters_private_entries(grouped_render_case())]
    #[case::private_repository_uses_japanese_labels(private_japanese_render_case())]
    #[case::single_visible_pr_omits_step_summary(one_visible_pull_request_render_case())]
    #[case::summaries_escape_markdown_syntax(escaped_summary_render_case())]
    #[case::created_non_current_pr_uses_reference_in_nested_list(
        created_non_current_nested_pr_render_case()
    )]
    #[case::tenth_step_uses_four_space_nested_list_indentation(tenth_step_nested_list_render_case())]
    fn renders_steps_as_expected(#[case] case: RenderCase) {
        assert_eq!(
            render_section(
                &case.plan,
                case.current_entry_id,
                case.current_body,
                &case.repository_privacy,
                &case.created_pull_requests,
            )
            .unwrap(),
            case.expected,
        );
    }

    #[rstest::rstest]
    #[case::html_tags(r#"<b>x</b>"#, r"\<b>x\</b>")]
    #[case::emphasis("Release *notes*", r"Release \*notes\*")]
    #[case::scheme_autolink("https://example.test", r"https\://example.test")]
    #[case::www_autolink("www.example.test", r"www\.example.test")]
    #[case::closing_hash("Drop #", r"Drop \#")]
    #[case::closing_hash_with_trailing_spaces("Drop #  ", r"Drop \#  ")]
    #[case::closing_hash_only("#", r"\#")]
    #[case::closing_hash_sequence_only("##", r"\##")]
    #[case::hash_inside_word("C#", "C#")]
    #[case::hashes_not_at_end("Drop ##x", "Drop ##x")]
    #[case::leading_hyphen("- item", "- item")]
    #[case::leading_quote("> quote", "> quote")]
    #[case::leading_ordered_list_marker("1. item", "1. item")]
    #[case::leading_hash("# heading", "# heading")]
    fn renders_series_names_with_heading_specific_markdown_escapes(
        #[case] name: &str,
        #[case] expected_name: &str,
    ) {
        let mut plan = plan();
        plan.name = name.to_string();
        let actual =
            render_section(&plan, "docs", "Existing body", &privacy(), &created_docs()).unwrap();
        let expected = format!(
            indoc! {"
                <!-- pr-series:start -->
                ### {name}

                1. Add preference tables (not created yet)
                2. **Document the settings** (this PR)
                <!-- pr-series:end -->
            "},
            name = expected_name,
        );

        assert_eq!(actual, expected.trim_end_matches('\n'));
    }

    #[rstest::rstest]
    #[case::backslash("\\", "\\\\")]
    #[case::backtick("`", r"\`")]
    #[case::asterisk("*", r"\*")]
    #[case::tilde("~", r"\~")]
    #[case::left_bracket("[", r"\[")]
    #[case::right_bracket("]", r"\]")]
    #[case::less_than("<", r"\<")]
    #[case::ampersand("&", r"\&")]
    #[case::underscore_between_ascii_alphanumeric("config_key", "config_key")]
    #[case::underscore_between_japanese_alphanumeric("設定_項目", "設定_項目")]
    #[case::underscore_next_to_punctuation("「_x_」", "「\\_x\\_」")]
    #[case::leading_underscore("_prefix", "\\_prefix")]
    #[case::trailing_underscore("suffix_", "suffix\\_")]
    #[case::underscore_next_to_whitespace("before _ middle", "before \\_ middle")]
    #[case::leading_hash("# heading", r"\# heading")]
    #[case::leading_quote("> quote", r"\> quote")]
    #[case::leading_hyphen("- item", r"\- item")]
    #[case::leading_plus("+ item", r"\+ item")]
    #[case::leading_exclamation("!notice", "!notice")]
    #[case::leading_double_quote(r#""quoted" summary"#, r#""quoted" summary"#)]
    #[case::leading_percent("% complete", "% complete")]
    #[case::leading_apostrophe("'quoted' summary", "'quoted' summary")]
    #[case::leading_parenthesis("(draft) docs", "(draft) docs")]
    #[case::leading_right_parenthesis(") note", ") note")]
    #[case::leading_comma(", note", ", note")]
    #[case::leading_slash("/path", "/path")]
    #[case::leading_semicolon("; note", "; note")]
    #[case::leading_equals("= note", "= note")]
    #[case::leading_question_mark("? note", "? note")]
    #[case::leading_caret("^ note", "^ note")]
    #[case::leading_left_brace("{draft}", "{draft}")]
    #[case::leading_right_brace("} draft", "} draft")]
    #[case::leading_pipe("| draft", "| draft")]
    #[case::leading_thematic_break("---", r"\---")]
    #[case::leading_numbered_list_period("2026. release", r"2026\. release")]
    #[case::leading_numbered_list_parenthesis("12) continuation", r"12\) continuation")]
    #[case::number_sequence_without_list_delimiter("123task. release", "123task. release")]
    #[case::non_ascii_number_sequence("２０２６. release", "２０２６. release")]
    #[case::numbered_list_delimiter_after_text("task 2. update", "task 2. update")]
    #[case::leading_spaces_before_marker("   # heading", r"\# heading")]
    #[case::leading_tab_before_numbered_list("\t12. item", r"12\. item")]
    #[case::leading_spaces_are_removed("   plain summary", "plain summary")]
    #[case::leading_ideographic_space_is_preserved("\u{3000}# heading", "\u{3000}# heading")]
    #[case::leading_nonbreaking_space_is_preserved("\u{00a0}# heading", "\u{00a0}# heading")]
    #[case::https_url_is_not_linked(
        "see https://example.test/?a=1&b=2 now",
        r"see https\://example.test/?a=1\&b=2 now"
    )]
    #[case::ftp_url_is_not_linked(
        "see ftp://example.test/?a=1&b=2 now",
        r"see ftp\://example.test/?a=1\&b=2 now"
    )]
    #[case::www_url_is_not_linked(
        "see www.example.test/?a=1&b=2 now",
        r"see www\.example.test/?a=1\&b=2 now"
    )]
    #[case::uppercase_www_is_not_escaped(
        "see WWW.example.test/?a=1&b=2 now",
        r"see WWW.example.test/?a=1\&b=2 now"
    )]
    #[case::other_ascii_punctuation_is_preserved(
        r#"a.b, c/d (e) 'f' "g" h? i! j=k %m {n} o|p q^r s;t x+y a-b a>b :tada: Fix: x @user #12 $x^2$"#,
        r#"a.b, c/d (e) 'f' "g" h? i! j=k %m {n} o|p q^r s;t x+y a-b a>b :tada: Fix: x @user #12 $x^2$"#,
    )]
    fn renders_summaries_with_only_required_markdown_escapes(
        #[case] summary: &str,
        #[case] expected_summary: &str,
    ) {
        let mut plan = plan();
        plan.pull_requests[2].summary = summary.to_string();
        let actual =
            render_section(&plan, "docs", "Existing body", &privacy(), &created_docs()).unwrap();
        let expected = format!(
            indoc! {"
                <!-- pr-series:start -->
                ### Notification preferences

                1. Add preference tables (not created yet)
                2. **{expected_summary}** (this PR)
                <!-- pr-series:end -->
            "},
            expected_summary = expected_summary,
        );

        assert_eq!(actual, expected.trim_end_matches('\n'));
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

            1. Add preference tables (not created yet)
            2. **Document the settings** (this PR)
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

            1. Add the sample feature
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

            1. Add the sample feature
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

            1. Add the sample feature
            <!-- pr-series:end -->
        "}
        .trim_end_matches('\n');

        assert_eq!(
            replace_managed_section("Existing text", section).unwrap(),
            indoc! {"
                Existing text

                <!-- pr-series:start -->
                ### New series

                1. Add the sample feature
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
