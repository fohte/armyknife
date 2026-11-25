use std::fs::{self, File, FileTimes};
use std::path::{Component, Path};
use std::time::SystemTime;

use crate::commands::agent::types::Engine;

pub(super) fn refresh(engine: Engine, session_id: &str) {
    if engine != Engine::Claude || !is_single_path_component(session_id) {
        return;
    }

    let Some(home_dir) = crate::shared::dirs::home_dir() else {
        return;
    };
    let session_env_dir = home_dir
        .join(".claude")
        .join("session-env")
        .join(session_id);
    let Ok(metadata) = fs::symlink_metadata(&session_env_dir) else {
        return;
    };
    if !metadata.file_type().is_dir() {
        return;
    }

    if let Ok(directory) = File::open(session_env_dir) {
        let _ = directory.set_times(FileTimes::new().set_modified(SystemTime::now()));
    }
}

fn is_single_path_component(session_id: &str) -> bool {
    let mut components = Path::new(session_id).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}
