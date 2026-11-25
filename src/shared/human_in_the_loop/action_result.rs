use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::review_finish::path_with_suffix;

const ACTION_RESULT_SUFFIX: &str = ".watch-web-action-result";
const ACTION_DOCUMENT_SNAPSHOT_SUFFIX: &str = ".watch-web-action-snapshot";
const ACTION_RESULT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReviewDocumentAction {
    PostComment,
    CreatePr,
    PostReplies,
    UpdateIssue,
}

impl ReviewDocumentAction {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::PostComment => "issue comment",
            Self::CreatePr => "pull request",
            Self::PostReplies => "pull request replies",
            Self::UpdateIssue => "issue body",
        }
    }

    fn allows_missing_document(self) -> bool {
        matches!(self, Self::CreatePr | Self::PostComment)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReviewDocumentActionResult {
    pub(crate) action: ReviewDocumentAction,
    pub(crate) succeeded: bool,
    pub(crate) message: String,
    document_hash: String,
    post_action_hash: Option<String>,
}

impl ReviewDocumentActionResult {
    pub(crate) fn new(
        action: ReviewDocumentAction,
        succeeded: bool,
        message: String,
        document_hash: String,
        post_action_hash: Option<String>,
    ) -> Self {
        Self {
            action,
            succeeded,
            message,
            document_hash,
            post_action_hash,
        }
    }

    pub(crate) fn print_for_agent(&self) {
        println!("{self}");
    }

    fn agent_message(&self) -> String {
        if self.succeeded {
            format!(
                "Watch-web {} succeeded: {}\nNo further submit or push command is needed.",
                self.action.label(),
                self.message,
            )
        } else {
            format!(
                "Watch-web {} failed: {}\nRetry this action from the watch-web review screen.",
                self.action.label(),
                self.message,
            )
        }
    }

    fn matches_document(&self, path: &Path) -> bool {
        match fs::read(path) {
            Ok(content) => {
                let hash = content_hash(&content);
                self.document_hash == hash || self.post_action_hash.as_deref() == Some(&hash)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.action.allows_missing_document()
            }
            Err(_) => false,
        }
    }
}

impl std::fmt::Display for ReviewDocumentActionResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.agent_message().fmt(formatter)
    }
}

pub(crate) fn action_result_path(document_path: &Path) -> PathBuf {
    path_with_suffix(document_path, ACTION_RESULT_SUFFIX)
}

fn action_document_snapshot_path(document_path: &Path) -> PathBuf {
    path_with_suffix(document_path, ACTION_DOCUMENT_SNAPSHOT_SUFFIX)
}

pub(crate) fn document_hash(path: &Path) -> std::io::Result<String> {
    fs::read(path).map(|content| content_hash(&content))
}

pub(crate) fn clear_action_result(document_path: &Path) -> std::io::Result<()> {
    remove_if_present(&action_result_path(document_path))?;
    remove_if_present(&action_document_snapshot_path(document_path))
}

pub(crate) fn save_action_document_snapshot(
    document_path: &Path,
    content: &str,
) -> std::io::Result<()> {
    let snapshot_path = action_document_snapshot_path(document_path);
    let parent = snapshot_path
        .parent()
        .ok_or_else(|| std::io::Error::other("action document snapshot path has no parent"))?;
    fs::create_dir_all(parent)?;
    let mut temporary_name = snapshot_path.as_os_str().to_os_string();
    temporary_name.push(format!(".{}.tmp", uuid::Uuid::new_v4()));
    let temporary_path = PathBuf::from(temporary_name);
    let save_result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        let mut file = options.open(&temporary_path)?;
        #[cfg(unix)]
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary_path, &snapshot_path)
    })();
    if save_result.is_err() {
        remove_if_present(&temporary_path)?;
    }
    save_result
}

pub(crate) fn take_action_document_snapshot(
    document_path: &Path,
) -> std::io::Result<Option<String>> {
    let snapshot_path = action_document_snapshot_path(document_path);
    match fs::read_to_string(&snapshot_path) {
        Ok(content) => {
            fs::remove_file(snapshot_path)?;
            Ok(Some(content))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn save_action_result(
    document_path: &Path,
    result: &ReviewDocumentActionResult,
) -> std::io::Result<()> {
    let result_path = action_result_path(document_path);
    let parent = result_path
        .parent()
        .ok_or_else(|| std::io::Error::other("action result path has no parent"))?;
    fs::create_dir_all(parent)?;
    let temporary_path = path_with_suffix(&result_path, ".tmp");
    fs::write(
        &temporary_path,
        serde_json::to_vec(result).map_err(std::io::Error::other)?,
    )?;
    fs::rename(temporary_path, result_path)
}

pub(crate) fn read_action_result(
    document_path: &Path,
) -> std::io::Result<Option<ReviewDocumentActionResult>> {
    let result_path = action_result_path(document_path);
    match fs::read(&result_path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(std::io::Error::other),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

pub(crate) fn successful_action_result(
    document_path: &Path,
    action: ReviewDocumentAction,
) -> std::io::Result<Option<ReviewDocumentActionResult>> {
    let Some(result) = read_action_result(document_path)? else {
        return Ok(None);
    };
    if !result.succeeded {
        return Ok(None);
    }
    if result.action != action || !result.matches_document(document_path) {
        remove_if_present(&action_result_path(document_path))?;
        return Ok(None);
    }
    Ok(Some(result))
}

pub(crate) fn successful_action_results_in_directory(
    directory: &Path,
    action: ReviewDocumentAction,
) -> std::io::Result<Vec<ReviewDocumentActionResult>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut result_paths = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(ACTION_RESULT_SUFFIX))
        })
        .collect::<Vec<_>>();
    result_paths.sort();

    let mut results = Vec::new();
    for result_path in result_paths {
        let Some(document_name) = result_path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(ACTION_RESULT_SUFFIX))
        else {
            continue;
        };
        let document_path = result_path.with_file_name(document_name);
        if let Some(result) = successful_action_result(&document_path, action)? {
            results.push(result);
        }
    }
    Ok(results)
}

pub(crate) fn wait_for_action_result(
    document_path: &Path,
) -> std::io::Result<ReviewDocumentActionResult> {
    wait_for_action_result_until(document_path, Instant::now() + ACTION_RESULT_TIMEOUT)
}

fn wait_for_action_result_until(
    document_path: &Path,
    deadline: Instant,
) -> std::io::Result<ReviewDocumentActionResult> {
    loop {
        if let Some(result) = read_action_result(document_path)? {
            return Ok(result);
        }
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Timed out waiting for the watch-web action result. The action may still be running in watch-web; do not retry submit or push.",
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub(crate) fn action_result_after_review_completion(
    completion_signal: u8,
    document_path: &Path,
) -> std::io::Result<Option<ReviewDocumentActionResult>> {
    if completion_signal == b'1' {
        wait_for_action_result(document_path).map(Some)
    } else {
        Ok(None)
    }
}

fn content_hash(content: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(content);
    crate::shared::hex::encode(&digest.finalize())
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ReviewDocumentAction, ReviewDocumentActionResult, action_result_after_review_completion,
        document_hash, save_action_document_snapshot, save_action_result, successful_action_result,
        take_action_document_snapshot, wait_for_action_result_until,
    };
    use rstest::rstest;
    use std::fs;
    use std::time::Instant;
    use tempfile::TempDir;

    #[test]
    fn missing_action_result_timeout_warns_against_repeating_the_action() {
        let directory = TempDir::new().expect("fixture directory should be created");
        let document_path = directory.path().join("review.md");
        let error = wait_for_action_result_until(&document_path, Instant::now())
            .expect_err("missing action result should time out");

        assert_eq!(
            (error.kind(), error.to_string()),
            (
                std::io::ErrorKind::TimedOut,
                "Timed out waiting for the watch-web action result. The action may still be running in watch-web; do not retry submit or push.".to_owned(),
            ),
        );
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_staging_does_not_truncate_a_symlink_target() {
        use super::{action_document_snapshot_path, path_with_suffix};
        use std::os::unix::fs::symlink;

        let directory = TempDir::new().expect("fixture directory should be created");
        let document_path = directory.path().join("review.md");
        let protected_path = directory.path().join("protected.txt");
        fs::write(&protected_path, "protected content").expect("protected file should be written");
        let staging_path = path_with_suffix(&action_document_snapshot_path(&document_path), ".tmp");
        symlink(&protected_path, &staging_path).expect("staging symlink should be created");

        save_action_document_snapshot(&document_path, "Approved document.")
            .expect("approved document snapshot should be saved");
        let observed = (
            fs::read_to_string(&protected_path).expect("protected file should remain readable"),
            take_action_document_snapshot(&document_path)
                .expect("approved document snapshot should be read"),
        );

        assert_eq!(
            observed,
            (
                "protected content".to_owned(),
                Some("Approved document.".to_owned()),
            ),
        );
    }

    #[rstest]
    #[case::issue_comment(ReviewDocumentAction::PostComment)]
    #[case::pr_draft(ReviewDocumentAction::CreatePr)]
    #[case::pr_replies(ReviewDocumentAction::PostReplies)]
    #[case::issue_body(ReviewDocumentAction::UpdateIssue)]
    fn successful_watch_web_action_remains_available_for_repeated_commands(
        #[case] action: ReviewDocumentAction,
    ) {
        let directory = TempDir::new().expect("fixture directory should be created");
        let document_path = directory.path().join("review.md");
        fs::write(&document_path, "Approved document.")
            .expect("fixture document should be written");
        let result = ReviewDocumentActionResult::new(
            action,
            true,
            "Operation result.".to_owned(),
            document_hash(&document_path).expect("document hash should be computed"),
            Some("post-action-hash".to_owned()),
        );
        save_action_result(&document_path, &result).expect("action result should be saved");

        let observed = (
            successful_action_result(&document_path, action).expect("action result should be read"),
            successful_action_result(&document_path, action)
                .expect("action result should remain available"),
        );

        assert_eq!(observed, (Some(result.clone()), Some(result)));
    }

    #[rstest]
    #[case::issue_comment(ReviewDocumentAction::PostComment)]
    #[case::pr_draft(ReviewDocumentAction::CreatePr)]
    #[case::pr_replies(ReviewDocumentAction::PostReplies)]
    #[case::issue_body(ReviewDocumentAction::UpdateIssue)]
    fn changed_document_does_not_reuse_a_previous_action_result(
        #[case] action: ReviewDocumentAction,
    ) {
        use super::action_result_path;

        let directory = TempDir::new().expect("fixture directory should be created");
        let document_path = directory.path().join("review.md");
        fs::write(&document_path, "Original document.")
            .expect("fixture document should be written");
        let result = ReviewDocumentActionResult::new(
            action,
            true,
            "Operation result.".to_owned(),
            document_hash(&document_path).expect("document hash should be computed"),
            None,
        );
        save_action_result(&document_path, &result).expect("action result should be saved");
        fs::write(&document_path, "Changed document.").expect("updated document should be written");

        let observed = (
            successful_action_result(&document_path, action)
                .expect("stale action result should be checked"),
            action_result_path(&document_path).exists(),
        );

        assert_eq!(observed, (None, false));
    }

    #[rstest]
    #[case::issue_comment(ReviewDocumentAction::PostComment)]
    #[case::pr_draft(ReviewDocumentAction::CreatePr)]
    #[case::pr_replies(ReviewDocumentAction::PostReplies)]
    #[case::issue_body(ReviewDocumentAction::UpdateIssue)]
    fn post_action_document_hash_keeps_a_successful_result_available(
        #[case] action: ReviewDocumentAction,
    ) {
        let directory = TempDir::new().expect("fixture directory should be created");
        let document_path = directory.path().join("review.md");
        fs::write(&document_path, "Approved document.")
            .expect("fixture document should be written");
        let original_hash =
            document_hash(&document_path).expect("document hash should be computed");
        fs::write(&document_path, "Updated after action.")
            .expect("post-action document should be written");
        let post_action_hash =
            document_hash(&document_path).expect("document hash should be computed");
        let result = ReviewDocumentActionResult::new(
            action,
            true,
            "Operation result.".to_owned(),
            original_hash,
            Some(post_action_hash),
        );
        save_action_result(&document_path, &result).expect("action result should be saved");

        assert_eq!(
            successful_action_result(&document_path, action)
                .expect("post-action result should be read"),
            Some(result),
        );
    }

    #[rstest]
    #[case::issue_comment(ReviewDocumentAction::PostComment)]
    #[case::pr_draft(ReviewDocumentAction::CreatePr)]
    #[case::pr_replies(ReviewDocumentAction::PostReplies)]
    #[case::issue_body(ReviewDocumentAction::UpdateIssue)]
    fn failed_action_result_is_not_used_for_idempotency(#[case] action: ReviewDocumentAction) {
        let directory = TempDir::new().expect("fixture directory should be created");
        let document_path = directory.path().join("review.md");
        fs::write(&document_path, "Approved document.")
            .expect("fixture document should be written");
        let result = ReviewDocumentActionResult::new(
            action,
            false,
            "Temporary failure.".to_owned(),
            document_hash(&document_path).expect("document hash should be computed"),
            None,
        );
        save_action_result(&document_path, &result).expect("action result should be saved");

        assert_eq!(
            successful_action_result(&document_path, action)
                .expect("failed action result should be read"),
            None,
        );
    }

    #[rstest]
    #[case::plain_finish(b'0', false)]
    #[case::action_finish(b'1', true)]
    fn only_action_completion_waits_for_a_result(
        #[case] completion_signal: u8,
        #[case] has_action_result: bool,
    ) {
        let directory = TempDir::new().expect("fixture directory should be created");
        let document_path = directory.path().join("review.md");
        fs::write(&document_path, "Approved document.")
            .expect("fixture document should be written");
        let result = ReviewDocumentActionResult::new(
            ReviewDocumentAction::CreatePr,
            true,
            "https://example.invalid/pull/42".to_owned(),
            document_hash(&document_path).expect("document hash should be computed"),
            None,
        );
        if has_action_result {
            save_action_result(&document_path, &result).expect("action result should be saved");
        }

        let observed = action_result_after_review_completion(completion_signal, &document_path)
            .expect("completion result should be read");

        assert_eq!(observed, has_action_result.then_some(result));
    }

    #[rstest]
    #[case::issue_comment(
        ReviewDocumentAction::PostComment,
        "Watch-web issue comment succeeded: Operation result.\nNo further submit or push command is needed."
    )]
    #[case::pr_draft(
        ReviewDocumentAction::CreatePr,
        "Watch-web pull request succeeded: Operation result.\nNo further submit or push command is needed."
    )]
    #[case::pr_replies(
        ReviewDocumentAction::PostReplies,
        "Watch-web pull request replies succeeded: Operation result.\nNo further submit or push command is needed."
    )]
    #[case::issue_body(
        ReviewDocumentAction::UpdateIssue,
        "Watch-web issue body succeeded: Operation result.\nNo further submit or push command is needed."
    )]
    fn agent_message_describes_the_completed_action(
        #[case] action: ReviewDocumentAction,
        #[case] expected: &str,
    ) {
        let result = ReviewDocumentActionResult::new(
            action,
            true,
            "Operation result.".to_owned(),
            "document-hash".to_owned(),
            None,
        );

        assert_eq!(result.to_string(), expected);
    }

    #[rstest]
    #[case::issue_comment(
        ReviewDocumentAction::PostComment,
        "Watch-web issue comment failed: Temporary failure.\nRetry this action from the watch-web review screen."
    )]
    #[case::pr_draft(
        ReviewDocumentAction::CreatePr,
        "Watch-web pull request failed: Temporary failure.\nRetry this action from the watch-web review screen."
    )]
    #[case::pr_replies(
        ReviewDocumentAction::PostReplies,
        "Watch-web pull request replies failed: Temporary failure.\nRetry this action from the watch-web review screen."
    )]
    #[case::issue_body(
        ReviewDocumentAction::UpdateIssue,
        "Watch-web issue body failed: Temporary failure.\nRetry this action from the watch-web review screen."
    )]
    fn failed_action_message_points_to_watch_web_retry(
        #[case] action: ReviewDocumentAction,
        #[case] expected: &str,
    ) {
        let result = ReviewDocumentActionResult::new(
            action,
            false,
            "Temporary failure.".to_owned(),
            "document-hash".to_owned(),
            None,
        );

        assert_eq!(result.to_string(), expected);
    }
}
