//! Best-effort stacking for newly submitted pull requests.

use crate::infra::github::{PrClient, PrStackClient};

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
            let warning = indoc::formatdoc! {"
                Warning: the PR was created, but its base branch could not be checked for an open PR: {error}
                Check manually with:
                {command}
            "};
            return Some(warning.trim_end().to_string());
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
            let warning = indoc::formatdoc! {"
                Warning: the PR was created, but its base PR's stack could not be read: {error}
                Inspect its membership and run the matching command manually:
                {commands}
            "};
            return Some(warning.trim_end().to_string());
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
                let warning = indoc::formatdoc! {"
                    Warning: the PR was created, but the stack could not be created: {error}
                    Run this command to create the stack manually:
                    {command}
                "};
                Some(warning.trim_end().to_string())
            }
        };
    };

    if stack.pull_requests.last() != Some(&lower_pr.number) {
        let position = stack
            .pull_requests
            .iter()
            .position(|number| *number == lower_pr.number)
            .map(|position| position + 1);
        let position = position.map_or_else(
            || "an unknown position".to_string(),
            |position| format!("position {position} of {}", stack.pull_requests.len()),
        );
        return Some(format!(
            "Warning: the PR was created, but was not added to stack {} because its base PR {} is at {position}; only the top PR can be extended.",
            stack.number, lower_pr.number,
        ));
    }

    let stack_number = stack.number;
    match client
        .add_pull_request_to_stack(owner, repo, stack_number, pull_request)
        .await
    {
        Ok(()) => None,
        Err(error) => {
            let command = add_to_stack_command(owner, repo, stack_number, pull_request);
            let warning = indoc::formatdoc! {"
                Warning: the PR was created, but it could not be added to stack {stack_number}: {error}
                Run this command to add it manually:
                {command}
            "};
            Some(warning.trim_end().to_string())
        }
    }
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

fn add_to_stack_command(owner: &str, repo: &str, stack: u64, pull_request: u64) -> String {
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
    let add_command = add_to_stack_command_template(owner, repo, pull_request);
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

fn add_to_stack_command_template(owner: &str, repo: &str, pull_request: u64) -> String {
    indoc::formatdoc! {"
        gh api --method POST repos/{owner}/{repo}/stacks/<stack_number>/add --input - <<'JSON'
        {{\"pull_requests\":[{pull_request}]}}
        JSON
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
    use rstest::rstest;

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

    #[test]
    fn shell_quotes_branch_names_for_manual_lookup() {
        assert_eq!(
            shell_quote("head=sample-owner:feature/one'two"),
            "'head=sample-owner:feature/one'\\''two'",
        );
    }
}
