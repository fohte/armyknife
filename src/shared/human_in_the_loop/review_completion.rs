use std::path::Path;

use crate::shared::config::EditorConfig;

use super::editor::{ReviewEditorOutcome, run_editor_for_review};
use super::review_finish::{
    ReviewDocumentOperationLock, ReviewFinishReason, finish_ack_fifo_path, open_fifo_reader,
    remove_fifo_if_present, try_read_fifo_signal_byte,
};
use super::{
    CleanupGuard, Document, DocumentSchema, FifoSignalGuard, Result, ReviewHandler, run_editor,
    try_signal_fifo_with_byte,
};

pub(crate) trait ReviewEditorRunner {
    fn run(
        &self,
        document_path: &Path,
        window_title: Option<&str>,
        finish_fifo_path: Option<&Path>,
    ) -> Result<ReviewEditorOutcome>;
}

impl ReviewEditorRunner for EditorConfig {
    fn run(
        &self,
        document_path: &Path,
        window_title: Option<&str>,
        finish_fifo_path: Option<&Path>,
    ) -> Result<ReviewEditorOutcome> {
        match finish_fifo_path {
            Some(finish_fifo_path) => run_editor_for_review(
                &self.editor_command,
                document_path,
                window_title,
                finish_fifo_path,
            ),
            None => run_editor(&self.editor_command, document_path, window_title)
                .map(ReviewEditorOutcome::Exited)
                .map_err(Into::into),
        }
    }
}

/// Run the editor and invoke the review handler after editing has finished.
///
/// The caller creates a `FifoSignalGuard` before calling this function so the
/// parent receives completion even if initialization fails.
pub(crate) fn complete_review<S, H, E>(
    document_path: &Path,
    window_title: Option<&str>,
    handler: &H,
    editor_runner: &E,
    finish_fifo_path: Option<&Path>,
    cleanup_guard: CleanupGuard,
    mut done_fifo_guard: Option<&mut FifoSignalGuard>,
) -> Result<Option<ReviewFinishReason>>
where
    S: DocumentSchema,
    H: ReviewHandler<S>,
    E: ReviewEditorRunner,
{
    let _cleanup_guard = cleanup_guard;
    let editor_outcome = editor_runner.run(document_path, window_title, finish_fifo_path)?;
    let finish_reason = match finish_fifo_path {
        Some(path) => finalize_finish_request(document_path, path, &editor_outcome)?,
        None => match &editor_outcome {
            ReviewEditorOutcome::FinishRequested(reason) => Some(*reason),
            ReviewEditorOutcome::Exited(_) => None,
        },
    };
    if let (Some(reason), Some(done_fifo_guard)) = (finish_reason, done_fifo_guard.as_mut()) {
        done_fifo_guard.set_signal_byte(reason.signal_byte());
    }
    let editor_succeeded = match editor_outcome {
        ReviewEditorOutcome::Exited(status) => status.success(),
        ReviewEditorOutcome::FinishRequested(_) => true,
    };

    if !editor_succeeded && finish_reason.is_none() {
        eprintln!("Editor exited with non-zero status");
        return Ok(None);
    }

    let handler_result =
        Document::<S>::from_path(document_path.to_path_buf()).and_then(|document| {
            match finish_reason {
                Some(reason) => handler.on_review_finished(&document, reason),
                None => handler.on_review_complete(&document),
            }
        });
    if finish_reason.is_some() {
        let acknowledgement = if handler_result.is_ok() { b'0' } else { b'1' };
        if let Err(error) =
            try_signal_fifo_with_byte(&finish_ack_fifo_path(document_path), acknowledgement)
        {
            tracing::warn!(%error, "failed to acknowledge review finish result");
        }
    }
    handler_result?;
    Ok(finish_reason)
}

fn finalize_finish_request(
    document_path: &Path,
    finish_fifo_path: &Path,
    editor_outcome: &ReviewEditorOutcome,
) -> Result<Option<ReviewFinishReason>> {
    let _operation_lock = ReviewDocumentOperationLock::acquire(document_path)?;
    let finish_reason = match editor_outcome {
        ReviewEditorOutcome::FinishRequested(reason) => Some(*reason),
        ReviewEditorOutcome::Exited(_) => {
            let mut finish_reader = open_fifo_reader(finish_fifo_path)?;
            try_read_fifo_signal_byte(&mut finish_reader)?.map(ReviewFinishReason::from_signal_byte)
        }
    };

    remove_fifo_if_present(finish_fifo_path)?;
    Ok(finish_reason)
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use indoc::indoc;
    use rstest::{fixture, rstest};
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    use super::*;
    use crate::shared::human_in_the_loop::editor::{self, ReviewEditorChild};
    use crate::shared::human_in_the_loop::{LockGuard, create_fifo_at_path};

    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
    struct FinishTestSchema {
        approved: bool,
    }

    impl DocumentSchema for FinishTestSchema {
        fn is_approved(&self) -> bool {
            self.approved
        }
    }

    struct FinishTestHandler {
        completed_documents: Arc<std::sync::Mutex<Vec<(PathBuf, bool, String)>>>,
        finish_reasons: Arc<std::sync::Mutex<Vec<ReviewFinishReason>>>,
    }

    impl ReviewHandler<FinishTestSchema> for FinishTestHandler {
        fn build_complete_args(
            &self,
            _document_path: &Path,
            _tmux_target: Option<&str>,
            _window_title: &str,
        ) -> Vec<std::ffi::OsString> {
            Vec::new()
        }

        fn on_review_complete(&self, document: &Document<FinishTestSchema>) -> Result<()> {
            self.completed_documents
                .lock()
                .expect("completed document observations should be writable")
                .push((
                    document.path.clone(),
                    document.frontmatter.approved,
                    std::fs::read_to_string(&document.path)?,
                ));
            Ok(())
        }

        fn on_review_finished(
            &self,
            document: &Document<FinishTestSchema>,
            reason: ReviewFinishReason,
        ) -> Result<()> {
            self.finish_reasons
                .lock()
                .expect("finish reasons should be writable")
                .push(reason);
            self.on_review_complete(document)
        }
    }

    struct FakeReviewEditor {
        termination_count: Arc<AtomicUsize>,
    }

    impl ReviewEditorChild for FakeReviewEditor {
        fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
            Ok(None)
        }

        fn terminate_and_wait(&mut self) -> std::io::Result<()> {
            self.termination_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct FakeReviewEditorRunner {
        termination_count: Arc<AtomicUsize>,
        saved_document: String,
        finish_signal_byte: u8,
    }

    impl ReviewEditorRunner for FakeReviewEditorRunner {
        fn run(
            &self,
            document_path: &Path,
            _window_title: Option<&str>,
            finish_fifo_path: Option<&Path>,
        ) -> Result<ReviewEditorOutcome> {
            let finish_fifo_path = finish_fifo_path.ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "finish FIFO should be passed to the editor runner",
                )
            })?;
            let mut finish_reader = open_fifo_reader(finish_fifo_path)?;
            let mut child = FakeReviewEditor {
                termination_count: self.termination_count.clone(),
            };
            std::fs::write(document_path, &self.saved_document)?;
            super::super::try_signal_fifo_with_byte(finish_fifo_path, self.finish_signal_byte)?;
            editor::wait_for_editor_or_finish_request(&mut child, &mut finish_reader)
        }
    }

    struct ReviewDocumentFixture {
        _temp_dir: TempDir,
        path: PathBuf,
    }

    #[fixture]
    fn review_document() -> ReviewDocumentFixture {
        let temp_dir = TempDir::new().expect("review directory should be created");
        let path = temp_dir.path().join("example-review.md");
        std::fs::write(&path, "Original draft.").expect("document should be written");
        ReviewDocumentFixture {
            _temp_dir: temp_dir,
            path,
        }
    }

    #[rstest]
    #[case::external_request(ReviewFinishReason::ExternalRequest, b'0', b'0')]
    #[case::replies_pushed(ReviewFinishReason::RepliesPushed, b'p', b'p')]
    fn finish_request_stops_editor_and_runs_handler_with_saved_document(
        review_document: ReviewDocumentFixture,
        #[case] finish_reason: ReviewFinishReason,
        #[case] finish_signal_byte: u8,
        #[case] expected_done_signal_byte: u8,
    ) {
        let document_path = review_document.path;
        let saved_document = indoc! {"
            ---
            approved: true
            ---
            Saved from the review panel.
        "};
        let lock = LockGuard::acquire(&document_path).expect("review lock should be acquired");
        let cleanup_guard = CleanupGuard::new(&document_path, lock.lock_id(), None)
            .expect("review process should be registered");
        let finish_fifo_path = super::super::finish_request_fifo_path(&document_path);
        let done_fifo_path = super::super::path_with_suffix(&document_path, ".done");
        create_fifo_at_path(&finish_fifo_path).expect("finish FIFO should be created");
        create_fifo_at_path(&finish_ack_fifo_path(&document_path))
            .expect("acknowledgement FIFO should be created");
        create_fifo_at_path(&done_fifo_path).expect("done FIFO should be created");
        let mut ack_reader = open_fifo_reader(&finish_ack_fifo_path(&document_path))
            .expect("acknowledgement reader should be opened");
        let mut done_reader = open_fifo_reader(&done_fifo_path).expect("done reader should open");
        let termination_count = Arc::new(AtomicUsize::new(0));
        let runner = FakeReviewEditorRunner {
            termination_count: termination_count.clone(),
            saved_document: saved_document.to_string(),
            finish_signal_byte,
        };
        let completed_documents = Arc::new(std::sync::Mutex::new(Vec::new()));
        let finish_reasons = Arc::new(std::sync::Mutex::new(Vec::new()));
        let handler = FinishTestHandler {
            completed_documents: completed_documents.clone(),
            finish_reasons: finish_reasons.clone(),
        };
        let mut done_fifo_guard = FifoSignalGuard::new(&done_fifo_path);

        let result = complete_review::<FinishTestSchema, _, _>(
            &document_path,
            None,
            &handler,
            &runner,
            Some(&finish_fifo_path),
            cleanup_guard,
            Some(&mut done_fifo_guard),
        )
        .map_err(|error| error.to_string());
        drop(done_fifo_guard);
        let mut ack_signal = [0_u8; 1];
        let ack_bytes_read = ack_reader
            .read(&mut ack_signal)
            .expect("review process should acknowledge the request");
        let mut done_signal = [0_u8; 1];
        let done_bytes_read = done_reader
            .read(&mut done_signal)
            .expect("review process should signal completion");
        let completed_documents = completed_documents
            .lock()
            .expect("completed document observations should be readable")
            .clone();
        let finish_reasons = finish_reasons
            .lock()
            .expect("finish reasons should be readable")
            .clone();

        assert_eq!(
            (
                result,
                termination_count.load(Ordering::SeqCst),
                completed_documents,
                finish_reasons,
                ack_bytes_read,
                ack_signal,
                done_bytes_read,
                done_signal,
            ),
            (
                Ok(Some(finish_reason)),
                1,
                vec![(document_path, true, saved_document.to_string())],
                vec![finish_reason],
                1,
                *b"0",
                1,
                [expected_done_signal_byte],
            ),
        );
    }
}
