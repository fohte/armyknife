use anyhow::Result;

use crate::commands::agent::store as cc_store;

struct SessionContext<'a> {
    branch: Option<&'a str>,
    base: Option<&'a str>,
    parent_cwd: &'a str,
    cwd: &'a str,
}

pub(super) fn resolve_prompt(
    prompt: Option<&str>,
    branch: Option<&str>,
    base: Option<&str>,
    parent_cwd: &str,
    cwd: &str,
) -> Option<String> {
    prompt.map(|prompt| {
        build_session_context_prompt(
            prompt,
            &SessionContext {
                branch,
                base,
                parent_cwd,
                cwd,
            },
        )
    })
}

fn build_session_context_prompt(prompt: &str, context: &SessionContext<'_>) -> String {
    let mut context_lines = vec![format!("- Working directory: {}", context.cwd)];
    if let Some(branch) = context.branch {
        context_lines.push(format!("- Branch: {branch}"));
    }
    if let Some(base) = context.base {
        context_lines.push(format!("- Base: {base}"));
    }
    context_lines.push(format!(
        "- Parent working directory: {}",
        context.parent_cwd
    ));
    let context = context_lines.join("\n");

    indoc::formatdoc! {"
        <session-context>
        {context}
        </session-context>
        <instructions>
        {prompt}
        </instructions>"}
    .trim_start()
    .to_string()
}

/// Build comma-separated ancestor chain for a child session.
///
/// Loads the parent session from the store and prepends its own ancestor chain,
/// producing: `grandparent_id,parent_id` (root-to-immediate-parent order).
/// Falls back to just the parent_session_id if the parent session cannot be loaded.
pub(super) fn build_ancestor_chain(parent_session_id: &str) -> Result<String> {
    match cc_store::load_session(parent_session_id) {
        Ok(Some(parent)) => {
            let mut ancestors = parent.ancestor_session_ids;
            ancestors.push(parent_session_id.to_string());
            Ok(ancestors.join(","))
        }
        _ => {
            // Parent session not found or load error; use parent_id alone.
            Ok(parent_session_id.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    #[rstest]
    #[case::complete_context(
        "sample/fix-widget",
        "origin/main",
        "/workspace/sample-repo",
        "/workspace/sample-repo/.worktrees/fix-widget",
        "Fix the widget",
        indoc! {"
            <session-context>
            - Working directory: /workspace/sample-repo/.worktrees/fix-widget
            - Branch: sample/fix-widget
            - Base: origin/main
            - Parent working directory: /workspace/sample-repo
            </session-context>
            <instructions>
            Fix the widget
            </instructions>"},
    )]
    #[case::cwd_without_branch_or_base(
        "",
        "",
        "/workspace/sample-repo",
        "/workspace/sample-repo",
        indoc! {"
            Inspect the repository

            Report findings"},
        indoc! {"
            <session-context>
            - Working directory: /workspace/sample-repo
            - Parent working directory: /workspace/sample-repo
            </session-context>
            <instructions>
            Inspect the repository

            Report findings
            </instructions>"},
    )]
    fn resolve_prompt_always_attaches_generic_context(
        #[case] branch: &str,
        #[case] base: &str,
        #[case] parent_cwd: &str,
        #[case] cwd: &str,
        #[case] prompt: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(
            resolve_prompt(
                Some(prompt),
                (!branch.is_empty()).then_some(branch),
                (!base.is_empty()).then_some(base),
                parent_cwd,
                cwd,
            ),
            Some(expected.to_string())
        );
    }

    #[test]
    fn no_prompt_remains_absent() {
        assert_eq!(
            resolve_prompt(
                None,
                None,
                None,
                "/workspace/sample-repo",
                "/workspace/sample-repo"
            ),
            None,
        );
    }
}
