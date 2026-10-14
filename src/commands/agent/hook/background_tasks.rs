use super::super::types::{HookInput, Session};

pub(super) fn record_launched_bash_task(session: &mut Session, input: &HookInput) {
    if let Some(task_id) = input.launched_background_task_id() {
        session.pending_bg_task_ids.insert(task_id.to_string());
    }
}
