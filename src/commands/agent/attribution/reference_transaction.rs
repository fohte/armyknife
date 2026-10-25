use std::path::Path;

use anyhow::{Context, Result};

use super::context::{self, CommitTarget, OperationContext};
use super::message;
use crate::infra::git::GitError;
use crate::infra::git::cmd::run_git;

pub(super) fn capture_target() -> Result<CommitTarget> {
    let repository = Path::new(".");
    capture_target_with(|args| run_git(repository, args.iter().copied()))
}

fn capture_target_with(
    mut run_command: impl FnMut(&[&str]) -> Result<String>,
) -> Result<CommitTarget> {
    let root = run_command(&["rev-parse", "--show-toplevel"])
        .context("failed to identify attribution repository")?;
    let reference = match run_command(&["symbolic-ref", "--quiet", "HEAD"]) {
        Ok(reference) => reference,
        Err(error) if is_command_failure(&error) => "HEAD".to_owned(),
        Err(error) => return Err(error).context("failed to identify attribution HEAD"),
    };
    let old_oid = match run_command(&["rev-parse", "--verify", "HEAD"]) {
        Ok(oid) => Some(oid),
        Err(error) if is_command_failure(&error) && reference != "HEAD" => None,
        Err(error) => return Err(error).context("failed to read attribution HEAD object"),
    };
    Ok(CommitTarget {
        repository: root,
        reference,
        old_oid,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReferenceUpdate {
    old_oid: String,
    new_oid: String,
    reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommitObject {
    kind: String,
    message: String,
}

pub(super) fn run(state: &str, input: &str, context_path: Option<&Path>) -> Result<()> {
    match state {
        "prepared" => {}
        "preparing" | "committed" | "aborted" => return Ok(()),
        other => anyhow::bail!("unsupported reference-transaction state: {other}"),
    }
    let Some(path) = context_path else {
        return Ok(());
    };
    let (operation, target) = context::claimed_final_object_context(path)?;
    let updates = parse_updates(input)?;
    let repository = run_git(Path::new("."), ["rev-parse", "--show-toplevel"])
        .context("failed to identify reference-transaction repository")?;
    verify_prepared_transaction(
        path,
        &repository,
        &target,
        &operation,
        &updates,
        context::unix_timestamp()?,
        read_commit_object,
    )?;
    Ok(())
}

fn parse_updates(input: &str) -> Result<Vec<ReferenceUpdate>> {
    let mut updates = Vec::new();
    for line in input.lines() {
        let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 {
            anyhow::bail!("invalid reference-transaction update record");
        }
        if !is_oid(fields[0])
            || !is_oid(fields[1])
            || fields[0].len() != fields[1].len()
            || fields[2].is_empty()
        {
            anyhow::bail!("invalid reference-transaction update record");
        }
        updates.push(ReferenceUpdate {
            old_oid: fields[0].to_owned(),
            new_oid: fields[1].to_owned(),
            reference: fields[2].to_owned(),
        });
    }
    if updates.is_empty() {
        anyhow::bail!("reference-transaction has no updates");
    }
    Ok(updates)
}

fn is_command_failure(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<GitError>()
        .is_some_and(|error| matches!(error, GitError::CommandFailed(_)))
}

fn is_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn matching_target_updates<'a>(
    target: &CommitTarget,
    updates: &'a [ReferenceUpdate],
) -> Vec<&'a ReferenceUpdate> {
    updates
        .iter()
        .filter(|update| {
            update.reference == target.reference && old_oid_matches(target, &update.old_oid)
        })
        .collect()
}

fn old_oid_matches(target: &CommitTarget, old_oid: &str) -> bool {
    match target.old_oid.as_deref() {
        Some(expected) => expected.eq_ignore_ascii_case(old_oid),
        None => old_oid.bytes().all(|byte| byte == b'0'),
    }
}

fn verify_prepared_transaction(
    context_path: &Path,
    repository: &str,
    target: &CommitTarget,
    operation: &OperationContext,
    updates: &[ReferenceUpdate],
    now: i64,
    mut read_object: impl FnMut(&str) -> Result<CommitObject>,
) -> Result<bool> {
    if repository != target.repository {
        return Ok(false);
    }
    let matching = matching_target_updates(target, updates);
    if matching.is_empty() {
        return Ok(false);
    }
    let verification = (|| {
        let [update] = matching.as_slice() else {
            anyhow::bail!("reference-transaction contains ambiguous attribution targets");
        };
        if context::is_expired(operation, now) {
            anyhow::bail!("attribution context is stale");
        }
        let object = read_object(&update.new_oid)?;
        verify_commit_object(&object, operation)
    })();
    consume_after_verification(context_path, verification)?;
    Ok(true)
}

fn verify_commit_object(object: &CommitObject, operation: &OperationContext) -> Result<()> {
    if object.kind != "commit" {
        anyhow::bail!("new reference target is not a commit object");
    }
    message::verify_trailers(&object.message, operation)
}

fn consume_after_verification(context_path: &Path, verification: Result<()>) -> Result<()> {
    let cleanup = context::consume(context_path);
    match (verification, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(verification_error), Ok(())) => Err(verification_error),
        (Ok(()), Err(cleanup_error)) => Err(cleanup_error),
        (Err(verification_error), Err(cleanup_error)) => Err(anyhow::anyhow!(
            "{verification_error:#}; failed to consume attribution context: {cleanup_error:#}"
        )),
    }
}

fn read_commit_object(oid: &str) -> Result<CommitObject> {
    let repository = Path::new(".");
    let kind = run_git(repository, ["cat-file", "-t", oid])
        .with_context(|| format!("failed to inspect new reference target {oid}"))?;
    let message = run_git(repository, ["show", "-s", "--format=%B", oid])
        .with_context(|| format!("failed to read new reference target {oid}"))?;
    Ok(CommitObject { kind, message })
}

#[cfg(test)]
mod tests {
    use super::{
        CommitObject, ReferenceUpdate, capture_target_with, parse_updates, verify_commit_object,
        verify_prepared_transaction,
    };
    use crate::commands::agent::attribution::Provider;
    use crate::commands::agent::attribution::context::{
        self, CommitTarget, ModelIdentity, OperationContext,
    };
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    const OLD_OID: &str = "1111111111111111111111111111111111111111";
    const NEW_OID: &str = "2222222222222222222222222222222222222222";
    const ZERO_OID: &str = "0000000000000000000000000000000000000000";

    #[derive(Clone, Copy)]
    enum GitCommandOutput {
        Value(&'static str),
        CommandFailure,
    }

    #[fixture]
    fn context_dir() -> TempDir {
        tempfile::tempdir().expect("create test context directory")
    }

    fn operation() -> OperationContext {
        OperationContext::new(
            Provider::Codex,
            ModelIdentity::known("fixture-model-red".to_owned()).expect("valid fixture model"),
            "fixture-tool-use-red".to_owned(),
        )
        .expect("create fixture operation")
    }

    fn target(repository: &str, reference: &str, old_oid: Option<&str>) -> CommitTarget {
        CommitTarget {
            repository: repository.to_owned(),
            reference: reference.to_owned(),
            old_oid: old_oid.map(str::to_owned),
        }
    }

    fn update(old_oid: &str, new_oid: &str, reference: &str) -> ReferenceUpdate {
        ReferenceUpdate {
            old_oid: old_oid.to_owned(),
            new_oid: new_oid.to_owned(),
            reference: reference.to_owned(),
        }
    }

    fn attributed_object() -> CommitObject {
        CommitObject {
            kind: "commit".to_owned(),
            message: indoc! {"
                Fixture subject

                AI-Tool: Codex
                AI-Model: fixture-model-red
            "}
            .to_owned(),
        }
    }

    fn capture_target_for(
        symbolic_ref: GitCommandOutput,
        head_oid: GitCommandOutput,
    ) -> CommitTarget {
        capture_target_with(|args| match args {
            ["rev-parse", "--show-toplevel"] => Ok("/fixture/repository".to_owned()),
            ["symbolic-ref", "--quiet", "HEAD"] => command_result(symbolic_ref),
            ["rev-parse", "--verify", "HEAD"] => command_result(head_oid),
            _ => anyhow::bail!("unexpected fixture Git command"),
        })
        .expect("capture fixture HEAD target")
    }

    fn command_result(output: GitCommandOutput) -> anyhow::Result<String> {
        match output {
            GitCommandOutput::Value(value) => Ok(value.to_owned()),
            GitCommandOutput::CommandFailure => {
                Err(crate::infra::git::GitError::CommandFailed("fixture failure".to_owned()).into())
            }
        }
    }

    #[rstest]
    #[case::attached_branch(
        GitCommandOutput::Value("refs/heads/fixture-branch"),
        GitCommandOutput::Value(OLD_OID),
        target("/fixture/repository", "refs/heads/fixture-branch", Some(OLD_OID))
    )]
    #[case::detached_head(
        GitCommandOutput::CommandFailure,
        GitCommandOutput::Value(OLD_OID),
        target("/fixture/repository", "HEAD", Some(OLD_OID))
    )]
    #[case::unborn_branch(
        GitCommandOutput::Value("refs/heads/fixture-branch"),
        GitCommandOutput::CommandFailure,
        target("/fixture/repository", "refs/heads/fixture-branch", None)
    )]
    fn captures_attached_detached_and_unborn_head_targets(
        #[case] symbolic_ref: GitCommandOutput,
        #[case] head_oid: GitCommandOutput,
        #[case] expected: CommitTarget,
    ) {
        assert_eq!(capture_target_for(symbolic_ref, head_oid), expected);
    }

    #[rstest]
    #[case::multiple_records(
        format!(
            indoc! {"
                {zero} {new} refs/heads/fixture-branch
                {old} {new} refs/heads/other-branch
            "},
            zero = ZERO_OID,
            old = OLD_OID,
            new = NEW_OID,
        ),
        vec![
            update(ZERO_OID, NEW_OID, "refs/heads/fixture-branch"),
            update(OLD_OID, NEW_OID, "refs/heads/other-branch"),
        ]
    )]
    #[case::sha256(
        format!("{} {} refs/heads/fixture-branch", "0".repeat(64), "a".repeat(64)),
        vec![update(&"0".repeat(64), &"a".repeat(64), "refs/heads/fixture-branch")]
    )]
    fn parses_reference_transaction_records(
        #[case] input: String,
        #[case] expected: Vec<ReferenceUpdate>,
    ) {
        assert_eq!(
            parse_updates(&input).expect("parse reference updates"),
            expected
        );
    }

    #[rstest]
    #[case::missing_field(
        "1111111111111111111111111111111111111111 2222222222222222222222222222222222222222",
        "invalid reference-transaction update record"
    )]
    #[case::extra_field(
        "1111111111111111111111111111111111111111 2222222222222222222222222222222222222222 refs/heads/fixture-branch extra",
        "invalid reference-transaction update record"
    )]
    #[case::invalid_oid(
        "not-an-object 2222222222222222222222222222222222222222 refs/heads/fixture-branch",
        "invalid reference-transaction update record"
    )]
    fn rejects_invalid_reference_transaction_records(
        #[case] input: &str,
        #[case] expected_error: &str,
    ) {
        assert_eq!(
            parse_updates(input).map_err(|error| error.to_string()),
            Err(expected_error.to_owned()),
        );
    }

    #[rstest]
    #[case::attached_branch(
        target("/fixture/repository", "refs/heads/fixture-branch", Some(OLD_OID)),
        update(OLD_OID, NEW_OID, "refs/heads/fixture-branch")
    )]
    #[case::detached_head(
        target("/fixture/repository", "HEAD", Some(OLD_OID)),
        update(OLD_OID, NEW_OID, "HEAD")
    )]
    #[case::unborn_head(
        target("/fixture/repository", "refs/heads/fixture-branch", None),
        update(ZERO_OID, NEW_OID, "refs/heads/fixture-branch")
    )]
    fn verifies_the_matching_head_update(
        #[case] target: CommitTarget,
        #[case] update: ReferenceUpdate,
        context_dir: TempDir,
    ) {
        let operation = operation();
        let path = context::create(&operation, context_dir.path()).expect("create context");
        context::claim_with_target(
            &path,
            context::unix_timestamp().expect("read clock"),
            Some(target.clone()),
        )
        .expect("claim context");
        let mut read_oid = None;
        let result = verify_prepared_transaction(
            &path,
            "/fixture/repository",
            &target,
            &operation,
            &[update],
            context::unix_timestamp().expect("read clock"),
            |oid| {
                read_oid = Some(oid.to_owned());
                Ok(attributed_object())
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, read_oid, context::read(&path).is_err()),
            (Ok(true), Some(NEW_OID.to_owned()), true),
        );
    }

    #[rstest]
    #[case::different_repository("/fixture/other", "refs/heads/fixture-branch", OLD_OID)]
    #[case::different_reference("/fixture/repository", "refs/heads/other-branch", OLD_OID)]
    #[case::different_old_oid("/fixture/repository", "refs/heads/fixture-branch", NEW_OID)]
    fn ignores_non_target_reference_updates(
        #[case] repository: &str,
        #[case] reference: &str,
        #[case] old_oid: &str,
        context_dir: TempDir,
    ) {
        let operation = operation();
        let path = context::create(&operation, context_dir.path()).expect("create context");
        let target = target(
            "/fixture/repository",
            "refs/heads/fixture-branch",
            Some(OLD_OID),
        );
        context::claim_with_target(
            &path,
            context::unix_timestamp().expect("read clock"),
            Some(target.clone()),
        )
        .expect("claim context");
        let mut read_oid = None;
        let result = verify_prepared_transaction(
            &path,
            repository,
            &target,
            &operation,
            &[update(old_oid, NEW_OID, reference)],
            context::unix_timestamp().expect("read clock"),
            |oid| {
                read_oid = Some(oid.to_owned());
                Ok(attributed_object())
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, read_oid, context::read(&path).is_ok()),
            (Ok(false), None, true),
        );
    }

    #[rstest]
    fn rejects_duplicate_matching_updates(context_dir: TempDir) {
        let updates = [
            update(OLD_OID, NEW_OID, "refs/heads/fixture-branch"),
            update(OLD_OID, NEW_OID, "refs/heads/fixture-branch"),
        ];
        let operation = operation();
        let path = context::create(&operation, context_dir.path()).expect("create context");
        let target = target(
            "/fixture/repository",
            "refs/heads/fixture-branch",
            Some(OLD_OID),
        );
        context::claim_with_target(
            &path,
            context::unix_timestamp().expect("read clock"),
            Some(target.clone()),
        )
        .expect("claim context");
        let error = verify_prepared_transaction(
            &path,
            "/fixture/repository",
            &target,
            &operation,
            &updates,
            context::unix_timestamp().expect("read clock"),
            |_| Ok(attributed_object()),
        )
        .expect_err("duplicate updates are ambiguous");

        assert_eq!(
            (error.to_string(), context::read(&path).is_err()),
            (
                "reference-transaction contains ambiguous attribution targets".to_owned(),
                true,
            ),
        );
    }

    #[rstest]
    fn accepts_a_commit_with_matching_trailers() {
        assert_eq!(
            verify_commit_object(&attributed_object(), &operation())
                .map_err(|error| error.to_string()),
            Ok(()),
        );
    }

    #[rstest]
    #[case::non_commit("tree", "new reference target is not a commit object")]
    #[case::wrong_tool(
        "commit",
        "commit message attribution does not match its operation context"
    )]
    fn rejects_invalid_final_objects(#[case] kind: &str, #[case] expected_error: &str) {
        let mut object = attributed_object();
        object.kind = kind.to_owned();
        if kind == "commit" {
            object.message = object.message.replace("AI-Tool: Codex", "AI-Tool: Other");
        }
        let error = verify_commit_object(&object, &operation())
            .expect_err("invalid final object must fail closed");

        assert_eq!(error.to_string(), expected_error);
    }

    #[rstest]
    fn consumes_context_after_a_matching_object_is_verified(context_dir: TempDir) {
        let operation = operation();
        let path = context::create(&operation, context_dir.path()).expect("create context");
        let target = target(
            "/fixture/repository",
            "refs/heads/fixture-branch",
            Some(OLD_OID),
        );
        context::claim_with_target(
            &path,
            context::unix_timestamp().expect("read clock"),
            Some(target.clone()),
        )
        .expect("claim context");
        let matched = verify_prepared_transaction(
            &path,
            "/fixture/repository",
            &target,
            &operation,
            &[update(OLD_OID, NEW_OID, "refs/heads/fixture-branch")],
            context::unix_timestamp().expect("read clock"),
            |_| Ok(attributed_object()),
        )
        .expect("verify final object");

        assert_eq!((matched, context::read(&path).is_err()), (true, true));
    }

    #[rstest]
    fn consumes_context_after_a_matching_object_is_rejected(context_dir: TempDir) {
        let operation = operation();
        let path = context::create(&operation, context_dir.path()).expect("create context");
        let target = target(
            "/fixture/repository",
            "refs/heads/fixture-branch",
            Some(OLD_OID),
        );
        context::claim_with_target(
            &path,
            context::unix_timestamp().expect("read clock"),
            Some(target.clone()),
        )
        .expect("claim context");
        let result = verify_prepared_transaction(
            &path,
            "/fixture/repository",
            &target,
            &operation,
            &[update(OLD_OID, NEW_OID, "refs/heads/fixture-branch")],
            context::unix_timestamp().expect("read clock"),
            |_| {
                Ok(CommitObject {
                    kind: "commit".to_owned(),
                    message: "Fixture subject".to_owned(),
                })
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, context::read(&path).is_err()),
            (
                Err("commit message has no managed attribution trailer block".to_owned()),
                true,
            ),
        );
    }

    #[rstest]
    fn consumes_context_when_the_matching_update_is_ambiguous(context_dir: TempDir) {
        let operation = operation();
        let path = context::create(&operation, context_dir.path()).expect("create context");
        let target = target(
            "/fixture/repository",
            "refs/heads/fixture-branch",
            Some(OLD_OID),
        );
        context::claim_with_target(
            &path,
            context::unix_timestamp().expect("read clock"),
            Some(target.clone()),
        )
        .expect("claim context");
        let update = update(OLD_OID, NEW_OID, "refs/heads/fixture-branch");
        let result = verify_prepared_transaction(
            &path,
            "/fixture/repository",
            &target,
            &operation,
            &[update.clone(), update],
            context::unix_timestamp().expect("read clock"),
            |_| Ok(attributed_object()),
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, context::read(&path).is_err()),
            (
                Err("reference-transaction contains ambiguous attribution targets".to_owned()),
                true,
            ),
        );
    }

    #[rstest]
    fn consumes_context_when_the_matching_object_cannot_be_read(context_dir: TempDir) {
        let operation = operation();
        let path = context::create(&operation, context_dir.path()).expect("create context");
        let target = target(
            "/fixture/repository",
            "refs/heads/fixture-branch",
            Some(OLD_OID),
        );
        context::claim_with_target(
            &path,
            context::unix_timestamp().expect("read clock"),
            Some(target.clone()),
        )
        .expect("claim context");
        let result = verify_prepared_transaction(
            &path,
            "/fixture/repository",
            &target,
            &operation,
            &[update(OLD_OID, NEW_OID, "refs/heads/fixture-branch")],
            context::unix_timestamp().expect("read clock"),
            |_| anyhow::bail!("fixture object reader failure"),
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, context::read(&path).is_err()),
            (Err("fixture object reader failure".to_owned()), true),
        );
    }

    #[rstest]
    fn consumes_an_expired_matching_context_and_rejects_the_update(context_dir: TempDir) {
        let operation = operation();
        let path = context::create(&operation, context_dir.path()).expect("create context");
        let target = target(
            "/fixture/repository",
            "refs/heads/fixture-branch",
            Some(OLD_OID),
        );
        context::claim_with_target(
            &path,
            context::unix_timestamp().expect("read clock"),
            Some(target.clone()),
        )
        .expect("claim context");
        let mut read_object = false;
        let result = verify_prepared_transaction(
            &path,
            "/fixture/repository",
            &target,
            &operation,
            &[update(OLD_OID, NEW_OID, "refs/heads/fixture-branch")],
            context::unix_timestamp().expect("read clock") + 601,
            |_| {
                read_object = true;
                Ok(attributed_object())
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, context::read(&path).is_err(), read_object),
            (Err("attribution context is stale".to_owned()), true, false),
        );
    }

    #[rstest]
    fn leaves_context_available_for_an_unrelated_reference_update(context_dir: TempDir) {
        let operation = operation();
        let path = context::create(&operation, context_dir.path()).expect("create context");
        let target = target(
            "/fixture/repository",
            "refs/heads/fixture-branch",
            Some(OLD_OID),
        );
        context::claim_with_target(
            &path,
            context::unix_timestamp().expect("read clock"),
            Some(target.clone()),
        )
        .expect("claim context");
        let matched = verify_prepared_transaction(
            &path,
            "/fixture/repository",
            &target,
            &operation,
            &[update(OLD_OID, NEW_OID, "refs/heads/other-branch")],
            context::unix_timestamp().expect("read clock"),
            |_| Ok(attributed_object()),
        )
        .expect("ignore unrelated transaction");

        assert_eq!((matched, context::read(&path).is_ok()), (false, true));
    }

    #[rstest]
    #[case::prepared("prepared")]
    #[case::preparing("preparing")]
    #[case::committed("committed")]
    #[case::aborted("aborted")]
    fn contextless_reference_updates_do_not_require_git(#[case] state: &str) {
        assert_eq!(
            super::run(state, "", None).map_err(|error| error.to_string()),
            Ok(()),
        );
    }

    #[rstest]
    fn prepared_reference_update_fails_when_context_was_never_claimed(context_dir: TempDir) {
        let operation = operation();
        let path = context::create(&operation, context_dir.path()).expect("create context");
        let result = super::run("prepared", "", Some(&path)).map_err(|error| error.to_string());

        assert_eq!(
            (result, context::read(&path).is_ok()),
            (
                Err("attribution context has no matching commit-message claim".to_owned()),
                true,
            ),
        );
    }

    #[test]
    fn unrelated_phase_does_not_consume_a_context() {
        let context_dir = tempfile::tempdir().expect("create test context directory");
        let operation = operation();
        let path = context::create(&operation, context_dir.path()).expect("create context");
        context::claim_with_target(
            &path,
            context::unix_timestamp().expect("read clock"),
            Some(target(
                "/fixture/repository",
                "refs/heads/fixture-branch",
                Some(OLD_OID),
            )),
        )
        .expect("claim context");

        assert_eq!(
            (
                super::run("aborted", "", Some(&path)).map_err(|error| error.to_string()),
                context::read(&path).is_ok(),
            ),
            (Ok(()), true),
        );
    }
}
