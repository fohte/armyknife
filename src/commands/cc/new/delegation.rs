use anyhow::Result;

use crate::commands::cc::store as cc_store;

/// Context information injected into the prompt when --agent is used
struct DelegationContext<'a> {
    branch: &'a str,
    base: &'a str,
    delegator_cwd: &'a str,
    worktree_cwd: &'a str,
}

/// Resolve the final prompt, optionally wrapping it with delegation context.
/// When `agent` is true, wraps the prompt in a `<delegated-task>` XML envelope.
pub(super) fn resolve_prompt(
    agent: bool,
    prompt: Option<&str>,
    branch: &str,
    base: &str,
    delegator_cwd: &str,
    worktree_cwd: &str,
) -> Option<String> {
    match (agent, prompt) {
        (true, Some(p)) => Some(build_delegated_prompt(
            p,
            &DelegationContext {
                branch,
                base,
                delegator_cwd,
                worktree_cwd,
            },
        )),
        (_, p) => p.map(String::from),
    }
}

/// Build a delegated prompt by wrapping the original prompt with context XML
fn build_delegated_prompt(prompt: &str, ctx: &DelegationContext) -> String {
    indoc::formatdoc! {"
        <delegated-task>
        <context>
        - Source: Delegated from another Claude Code session
        - Branch: {branch}
        - Base: {base}
        - Delegator CWD: {delegator_cwd}
        - Worktree CWD: {worktree_cwd}
        - Contact the delegator via SendMessage only for one of these two reasons, never for anything else -- not progress updates, not a completion or PR-ready report, not clarifying questions: (1) a premise in these instructions turns out to be wrong, or (2) you were blocked waiting on something under the delegator's control (another repo's fix, a package publish, a prior PR merge, etc.) and it just cleared. Resolve the delegator's name at runtime with `a cc peer parent | jq -r '.[0].name // empty'`. If that is empty, the delegator may be paused -- run `a cc peer wake $(a cc peer parent | jq -r '.[0].session_id')` to resume it and get back a usable name
        </context>
        <instructions>
        {prompt}
        </instructions>
        </delegated-task>",
        branch = ctx.branch,
        base = ctx.base,
        delegator_cwd = ctx.delegator_cwd,
        worktree_cwd = ctx.worktree_cwd,
        prompt = prompt,
    }
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
            // Parent session not found or load error; use parent_id alone
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
    #[case::single_line(
        "fohte/fix-auth-bug",
        "origin/master",
        "/home/user/repo",
        "/home/user/repo/.worktrees/fix-auth-bug",
        "Fix the auth bug",
        indoc! {"
            <delegated-task>
            <context>
            - Source: Delegated from another Claude Code session
            - Branch: fohte/fix-auth-bug
            - Base: origin/master
            - Delegator CWD: /home/user/repo
            - Worktree CWD: /home/user/repo/.worktrees/fix-auth-bug
            - Contact the delegator via SendMessage only for one of these two reasons, never for anything else -- not progress updates, not a completion or PR-ready report, not clarifying questions: (1) a premise in these instructions turns out to be wrong, or (2) you were blocked waiting on something under the delegator's control (another repo's fix, a package publish, a prior PR merge, etc.) and it just cleared. Resolve the delegator's name at runtime with `a cc peer parent | jq -r '.[0].name // empty'`. If that is empty, the delegator may be paused -- run `a cc peer wake $(a cc peer parent | jq -r '.[0].session_id')` to resume it and get back a usable name
            </context>
            <instructions>
            Fix the auth bug
            </instructions>
            </delegated-task>"},
    )]
    #[case::multiline_prompt(
        "fohte/feature-x",
        "origin/main",
        "/tmp/repo",
        "/tmp/repo/.worktrees/feature-x",
        indoc! {"
            ## Background
            Some context

            ## Goal
            Implement feature X"},
        indoc! {"
            <delegated-task>
            <context>
            - Source: Delegated from another Claude Code session
            - Branch: fohte/feature-x
            - Base: origin/main
            - Delegator CWD: /tmp/repo
            - Worktree CWD: /tmp/repo/.worktrees/feature-x
            - Contact the delegator via SendMessage only for one of these two reasons, never for anything else -- not progress updates, not a completion or PR-ready report, not clarifying questions: (1) a premise in these instructions turns out to be wrong, or (2) you were blocked waiting on something under the delegator's control (another repo's fix, a package publish, a prior PR merge, etc.) and it just cleared. Resolve the delegator's name at runtime with `a cc peer parent | jq -r '.[0].name // empty'`. If that is empty, the delegator may be paused -- run `a cc peer wake $(a cc peer parent | jq -r '.[0].session_id')` to resume it and get back a usable name
            </context>
            <instructions>
            ## Background
            Some context

            ## Goal
            Implement feature X
            </instructions>
            </delegated-task>"},
    )]
    fn build_delegated_prompt_wraps_with_xml(
        #[case] branch: &str,
        #[case] base: &str,
        #[case] delegator_cwd: &str,
        #[case] worktree_cwd: &str,
        #[case] prompt: &str,
        #[case] expected: &str,
    ) {
        let ctx = DelegationContext {
            branch,
            base,
            delegator_cwd,
            worktree_cwd,
        };
        let result = build_delegated_prompt(prompt.trim_start(), &ctx);

        assert_eq!(result, expected.trim_start());
    }

    #[rstest]
    #[case::agent_wraps_prompt(true, Some("do something"), true)]
    #[case::no_agent_passes_through(false, Some("do something"), false)]
    #[case::agent_without_prompt(true, None, false)]
    #[case::no_agent_no_prompt(false, None, false)]
    fn resolve_prompt_wraps_only_when_agent_and_prompt(
        #[case] agent: bool,
        #[case] prompt: Option<&str>,
        #[case] expect_wrapped: bool,
    ) {
        let result = resolve_prompt(
            agent,
            prompt,
            "fohte/test",
            "origin/main",
            "/cwd",
            "/worktree",
        );

        match (prompt, expect_wrapped) {
            (None, _) => assert_eq!(result, None),
            (Some(p), true) => {
                let expected = build_delegated_prompt(
                    p,
                    &DelegationContext {
                        branch: "fohte/test",
                        base: "origin/main",
                        delegator_cwd: "/cwd",
                        worktree_cwd: "/worktree",
                    },
                );
                assert_eq!(result, Some(expected));
            }
            (Some(p), false) => {
                assert_eq!(result, Some(p.to_string()));
            }
        }
    }
}
