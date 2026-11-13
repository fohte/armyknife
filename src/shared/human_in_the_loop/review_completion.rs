use std::path::Path;

use crate::shared::config::EditorConfig;

use super::editor::{ReviewEditorOutcome, run_editor_for_review};
use super::review_finish::{
    ReviewDocumentOperationLock, finish_ack_fifo_path, open_fifo_reader, remove_fifo_if_present,
    try_read_fifo_signal,
};
use super::{
    CleanupGuard, Document, DocumentSchema, Result, ReviewHandler, run_editor,
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
) -> Result<()>
where
    S: DocumentSchema,
    H: ReviewHandler<S>,
    E: ReviewEditorRunner,
{
    let _cleanup_guard = cleanup_guard;
    let editor_outcome = editor_runner.run(document_path, window_title, finish_fifo_path)?;
    let finish_requested = match finish_fifo_path {
        Some(path) => finalize_finish_request(document_path, path, &editor_outcome)?,
        None => matches!(&editor_outcome, ReviewEditorOutcome::FinishRequested),
    };
    let editor_succeeded = match editor_outcome {
        ReviewEditorOutcome::Exited(status) => status.success(),
        ReviewEditorOutcome::FinishRequested => true,
    };

    if !editor_succeeded && !finish_requested {
        eprintln!("Editor exited with non-zero status");
        return Ok(());
    }

    let handler_result = Document::<S>::from_path(document_path.to_path_buf())
        .and_then(|document| handler.on_review_complete(&document));
    if finish_requested {
        let acknowledgement = if handler_result.is_ok() { b'0' } else { b'1' };
        if let Err(error) =
            try_signal_fifo_with_byte(&finish_ack_fifo_path(document_path), acknowledgement)
        {
            tracing::warn!(%error, "failed to acknowledge review finish result");
        }
    }
    handler_result
}

fn finalize_finish_request(
    document_path: &Path,
    finish_fifo_path: &Path,
    editor_outcome: &ReviewEditorOutcome,
) -> Result<bool> {
    let _operation_lock = ReviewDocumentOperationLock::acquire(document_path)?;
    let request_received = if matches!(editor_outcome, ReviewEditorOutcome::FinishRequested) {
        true
    } else {
        let mut finish_reader = open_fifo_reader(finish_fifo_path)?;
        try_read_fifo_signal(&mut finish_reader)?
    };

    remove_fifo_if_present(finish_fifo_path)?;
    Ok(request_received)
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
    use crate::shared::human_in_the_loop::{LockGuard, create_fifo_at_path, signal_fifo};

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
            signal_fifo(finish_fifo_path);
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
    fn finish_request_stops_editor_and_runs_handler_with_saved_document(
        review_document: ReviewDocumentFixture,
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
        create_fifo_at_path(&finish_fifo_path).expect("finish FIFO should be created");
        create_fifo_at_path(&finish_ack_fifo_path(&document_path))
            .expect("acknowledgement FIFO should be created");
        let mut ack_reader = open_fifo_reader(&finish_ack_fifo_path(&document_path))
            .expect("acknowledgement reader should be opened");
        let termination_count = Arc::new(AtomicUsize::new(0));
        let runner = FakeReviewEditorRunner {
            termination_count: termination_count.clone(),
            saved_document: saved_document.to_string(),
        };
        let completed_documents = Arc::new(std::sync::Mutex::new(Vec::new()));
        let handler = FinishTestHandler {
            completed_documents: completed_documents.clone(),
        };

        let result = complete_review::<FinishTestSchema, _, _>(
            &document_path,
            None,
            &handler,
            &runner,
            Some(&finish_fifo_path),
            cleanup_guard,
        )
        .map_err(|error| error.to_string());
        let mut ack_signal = [0_u8; 1];
        let ack_bytes_read = ack_reader
            .read(&mut ack_signal)
            .expect("review process should acknowledge the request");
        let completed_documents = completed_documents
            .lock()
            .expect("completed document observations should be readable")
            .clone();

        assert_eq!(
            (
                result,
                termination_count.load(Ordering::SeqCst),
                completed_documents,
                ack_bytes_read,
                ack_signal,
            ),
            (
                Ok(()),
                1,
                vec![(document_path, true, saved_document.to_string())],
                1,
                *b"0",
            ),
        );
    }
}
