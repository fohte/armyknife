//! Best-effort stacking for newly submitted pull requests.

use std::fmt;

use crate::infra::github::{PrClient, PrStackClient, PullRequestStack};

pub(super) async fn stack_created_pull_request(
    client: &(impl PrClient + PrStackClient),
    owner: &str,
    repo: &str,
    base_branch: &str,
    pull_request: u64,
) -> Option<String> {
    let lower_pr = match client
        .get_open_pr_for_branch(owner, repo, base_branch)
        .await
    {
        Ok(Some(pr)) => pr,
        Ok(_) => return None,
        Err(error) => {
            let command = format!(
                "gh api repos/{owner}/{repo}/pulls -f {} -f 'state=open'",
                shell_quote(&format!("head={owner}:{base_branch}")),
            );
            let instructions = indoc::formatdoc! {"
                Check manually with:
                {command}
            "};
            return Some(format_warning(
                &format!(
                    "the PR was created, but its base branch could not be checked for an open PR: {error}"
                ),
                Some(&instructions),
            ));
        }
    };

    let stack = match client
        .get_stack_for_pull_request(owner, repo, lower_pr.number)
        .await
    {
        Ok(stack) => stack,
        Err(error) => {
            let commands =
                stack_lookup_recovery_commands(owner, repo, lower_pr.number, pull_request);
            let instructions = indoc::formatdoc! {"
                Inspect its membership and run the matching command manually:
                {commands}
            "};
            return Some(format_warning(
                &format!("the PR was created, but its base PR's stack could not be read: {error}"),
                Some(&instructions),
            ));
        }
    };

    let Some(stack) = stack else {
        return match client
            .create_stack(owner, repo, &[lower_pr.number, pull_request])
            .await
        {
            Ok(()) => None,
            Err(error) => {
                let command = create_stack_command(owner, repo, lower_pr.number, pull_request);
                let instructions = indoc::formatdoc! {"
                    Run this command to create the stack manually:
                    {command}
                "};
                Some(format_warning(
                    &format!("the PR was created, but the stack could not be created: {error}"),
                    Some(&instructions),
                ))
            }
        };
    };

    if stack.position != stack.size {
        return Some(base_pr_not_at_top_warning(&stack, lower_pr.number));
    }

    let stack_number = stack.number;
    match client
        .add_pull_request_to_stack(owner, repo, stack_number, pull_request)
        .await
    {
        Ok(()) => None,
        Err(error) => {
            let command = add_to_stack_command(owner, repo, stack_number, pull_request);
            let instructions = indoc::formatdoc! {"
                Run this command to add it manually:
                {command}
            "};
            Some(format_warning(
                &format!(
                    "the PR was created, but it could not be added to stack {stack_number}: {error}"
                ),
                Some(&instructions),
            ))
        }
    }
}

fn format_warning(reason: &str, instructions: Option<&str>) -> String {
    let warning = if let Some(instructions) = instructions {
        indoc::formatdoc! {"
            Warning: {reason}
            {instructions}
        "}
    } else {
        indoc::formatdoc! {"
            Warning: {reason}
        "}
    };
    warning.trim_end().to_string()
}

fn base_pr_not_at_top_warning(stack: &PullRequestStack, lower_pr: u64) -> String {
    format_warning(
        &format!(
            "the PR was created, but was not added to stack {} because its base PR {lower_pr} is at position {} of {}; only the top PR can be extended.",
            stack.number, stack.position, stack.size,
        ),
        None,
    )
}

fn create_stack_command(owner: &str, repo: &str, lower_pr: u64, pull_request: u64) -> String {
    indoc::formatdoc! {"
        gh api --method POST repos/{owner}/{repo}/stacks --input - <<'JSON'
        {{\"pull_requests\":[{lower_pr},{pull_request}]}}
        JSON
    "}
    .trim_end()
    .to_string()
}

fn add_to_stack_command(
    owner: &str,
    repo: &str,
    stack: impl fmt::Display,
    pull_request: u64,
) -> String {
    indoc::formatdoc! {"
        gh api --method POST repos/{owner}/{repo}/stacks/{stack}/add --input - <<'JSON'
        {{\"pull_requests\":[{pull_request}]}}
        JSON
    "}
    .trim_end()
    .to_string()
}

fn stack_lookup_recovery_commands(
    owner: &str,
    repo: &str,
    lower_pr: u64,
    pull_request: u64,
) -> String {
    let inspect_command = shell_quote(&format!(
        "repos/{owner}/{repo}/stacks?pull_request={lower_pr}"
    ));
    let create_command = create_stack_command(owner, repo, lower_pr, pull_request);
    let add_command = add_to_stack_command(owner, repo, "<stack_number>", pull_request);
    indoc::formatdoc! {"
        Inspect stack membership:
          gh api {inspect_command}
        If no stack exists, create one:
          {create_command}
        If the base PR is at the top of a stack, replace <stack_number> and add the new PR:
          {add_command}
    "}
    .trim_end()
    .to_string()
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::github::{
        CreatePrParams, CreatedPullRequest, PrInfo, PrState, UpdatePrParams,
    };
    use rstest::rstest;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Scenario {
        BaseLookupFailure,
        StackLookupFailure,
        StackCreationFailure,
        StackAddFailure,
        BasePrNotAtTop,
    }

    struct FakeStackClient {
        scenario: Scenario,
    }

    impl PrClient for FakeStackClient {
        async fn create_pull_request(
            &self,
            _params: CreatePrParams,
        ) -> anyhow::Result<CreatedPullRequest> {
            Ok(CreatedPullRequest {
                number: 18,
                url: "https://github.com/sample-owner/sample-repo/pull/18".to_string(),
            })
        }

        async fn update_pull_request(&self, _params: UpdatePrParams) -> anyhow::Result<String> {
            Ok(String::new())
        }

        async fn get_pr_for_branch(
            &self,
            _owner: &str,
            _repo: &str,
            _branch: &str,
        ) -> anyhow::Result<Option<PrInfo>> {
            Ok(None)
        }

        async fn get_open_pr_for_branch(
            &self,
            _owner: &str,
            _repo: &str,
            _branch: &str,
        ) -> anyhow::Result<Option<PrInfo>> {
            if self.scenario == Scenario::BaseLookupFailure {
                anyhow::bail!("base lookup failed");
            }
            Ok(Some(PrInfo {
                number: 17,
                title: "Base change".to_string(),
                state: PrState::Open,
                url: "https://github.com/sample-owner/sample-repo/pull/17".to_string(),
            }))
        }
    }

    impl PrStackClient for FakeStackClient {
        async fn get_stack_for_pull_request(
            &self,
            _owner: &str,
            _repo: &str,
            _pull_request: u64,
        ) -> anyhow::Result<Option<PullRequestStack>> {
            match self.scenario {
                Scenario::StackLookupFailure => anyhow::bail!("stack lookup failed"),
                Scenario::StackCreationFailure => Ok(None),
                Scenario::BasePrNotAtTop => Ok(Some(PullRequestStack {
                    number: 23,
                    position: 2,
                    size: 3,
                })),
                _ => Ok(Some(PullRequestStack {
                    number: 23,
                    position: 2,
                    size: 2,
                })),
            }
        }

        async fn create_stack(
            &self,
            _owner: &str,
            _repo: &str,
            _pull_requests: &[u64],
        ) -> anyhow::Result<()> {
            if self.scenario == Scenario::StackCreationFailure {
                anyhow::bail!("stack creation failed");
            }
            Ok(())
        }

        async fn add_pull_request_to_stack(
            &self,
            _owner: &str,
            _repo: &str,
            _stack_number: u64,
            _pull_request: u64,
        ) -> anyhow::Result<()> {
            if self.scenario == Scenario::StackAddFailure {
                anyhow::bail!("stack extension failed");
            }
            Ok(())
        }
    }

    #[derive(Clone, Copy)]
    enum CommandKind {
        Create,
        Add,
        InspectAndRecover,
    }

    #[rstest]
    #[case::create_stack(
        CommandKind::Create,
        indoc::indoc! {r#"
            gh api --method POST repos/sample-owner/sample-repo/stacks --input - <<'JSON'
            {"pull_requests":[17,18]}
            JSON
        "#}
    )]
    #[case::add_to_stack(
        CommandKind::Add,
        indoc::indoc! {r#"
            gh api --method POST repos/sample-owner/sample-repo/stacks/19/add --input - <<'JSON'
            {"pull_requests":[18]}
            JSON
        "#}
    )]
    #[case::inspect_and_recover(
        CommandKind::InspectAndRecover,
        indoc::indoc! {r#"
            Inspect stack membership:
              gh api 'repos/sample-owner/sample-repo/stacks?pull_request=17'
            If no stack exists, create one:
              gh api --method POST repos/sample-owner/sample-repo/stacks --input - <<'JSON'
            {"pull_requests":[17,18]}
            JSON
            If the base PR is at the top of a stack, replace <stack_number> and add the new PR:
              gh api --method POST repos/sample-owner/sample-repo/stacks/<stack_number>/add --input - <<'JSON'
            {"pull_requests":[18]}
            JSON
        "#}
    )]
    fn formats_manual_stack_commands(#[case] kind: CommandKind, #[case] expected: &str) {
        let command = match kind {
            CommandKind::Create => create_stack_command("sample-owner", "sample-repo", 17, 18),
            CommandKind::Add => add_to_stack_command("sample-owner", "sample-repo", 19, 18),
            CommandKind::InspectAndRecover => {
                stack_lookup_recovery_commands("sample-owner", "sample-repo", 17, 18)
            }
        };
        assert_eq!(command, expected.trim_end());
    }

    #[rstest]
    #[case::base_lookup_failure(
        Scenario::BaseLookupFailure,
        indoc::indoc! {r#"
            Warning: the PR was created, but its base branch could not be checked for an open PR: base lookup failed
            Check manually with:
            gh api repos/sample-owner/sample-repo/pulls -f 'head=sample-owner:main' -f 'state=open'
        "#}
    )]
    #[case::stack_lookup_failure(
        Scenario::StackLookupFailure,
        indoc::indoc! {r#"
            Warning: the PR was created, but its base PR's stack could not be read: stack lookup failed
            Inspect its membership and run the matching command manually:
            Inspect stack membership:
              gh api 'repos/sample-owner/sample-repo/stacks?pull_request=17'
            If no stack exists, create one:
              gh api --method POST repos/sample-owner/sample-repo/stacks --input - <<'JSON'
            {"pull_requests":[17,18]}
            JSON
            If the base PR is at the top of a stack, replace <stack_number> and add the new PR:
              gh api --method POST repos/sample-owner/sample-repo/stacks/<stack_number>/add --input - <<'JSON'
            {"pull_requests":[18]}
            JSON
        "#}
    )]
    #[case::stack_creation_failure(
        Scenario::StackCreationFailure,
        indoc::indoc! {r#"
            Warning: the PR was created, but the stack could not be created: stack creation failed
            Run this command to create the stack manually:
            gh api --method POST repos/sample-owner/sample-repo/stacks --input - <<'JSON'
            {"pull_requests":[17,18]}
            JSON
        "#}
    )]
    #[case::stack_add_failure(
        Scenario::StackAddFailure,
        indoc::indoc! {r#"
            Warning: the PR was created, but it could not be added to stack 23: stack extension failed
            Run this command to add it manually:
            gh api --method POST repos/sample-owner/sample-repo/stacks/23/add --input - <<'JSON'
            {"pull_requests":[18]}
            JSON
        "#}
    )]
    #[case::base_pr_not_at_top(
        Scenario::BasePrNotAtTop,
        "Warning: the PR was created, but was not added to stack 23 because its base PR 17 is at position 2 of 3; only the top PR can be extended."
    )]
    #[tokio::test]
    async fn returns_complete_warning_for_stack_failures(
        #[case] scenario: Scenario,
        #[case] expected: &str,
    ) {
        let client = FakeStackClient { scenario };

        assert_eq!(
            stack_created_pull_request(&client, "sample-owner", "sample-repo", "main", 18).await,
            Some(expected.trim_end().to_string()),
        );
    }

    #[test]
    fn shell_quotes_branch_names_for_manual_lookup() {
        assert_eq!(
            shell_quote("head=sample-owner:feature/one'two"),
            "'head=sample-owner:feature/one'\\''two'",
        );
    }
}
