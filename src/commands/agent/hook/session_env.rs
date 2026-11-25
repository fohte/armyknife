use std::fs::{self, File, FileTimes};
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

use crate::commands::agent::types::Engine;

pub(super) fn refresh(engine: Engine, session_id: &str) {
    if engine != Engine::Claude || !is_single_path_component(session_id) {
        return;
    }

    let Some(config_dir) = std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| crate::shared::dirs::home_dir().map(|home_dir| home_dir.join(".claude")))
    else {
        return;
    };
    let session_env_dir = config_dir.join("session-env").join(session_id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn parent_traversal_does_not_refresh_a_directory_outside_session_env() {
        let home_dir = tempfile::TempDir::new().expect("home directory should be created");
        let outside_dir = home_dir.path().join("outside-session");
        std::fs::create_dir(&outside_dir).expect("outside directory should be created");
        let old_time = SystemTime::now() - Duration::from_secs(60 * 60 * 24);
        File::open(&outside_dir)
            .expect("outside directory should open")
            .set_times(FileTimes::new().set_modified(old_time))
            .expect("old timestamp should be set");
        let home = home_dir.path().to_str().expect("home path should be UTF-8");

        temp_env::with_vars(
            [("HOME", Some(home)), ("CLAUDE_CONFIG_DIR", None::<&str>)],
            || refresh(Engine::Claude, "../../outside-session"),
        );
        let modified = fs::metadata(&outside_dir)
            .expect("outside directory should remain")
            .modified()
            .expect("outside directory timestamp should be available");

        assert_eq!((modified > old_time, outside_dir.exists()), (false, true));
    }
}
