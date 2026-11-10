use std::path::Path;

pub(super) fn is_session_store_change(event: &notify::Event) -> bool {
    let is_database_file = |path: &Path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "sessions.db" || name == "sessions.db-wal")
    };

    match event.kind {
        notify::EventKind::Modify(
            notify::event::ModifyKind::Data(_) | notify::event::ModifyKind::Any,
        ) => event.paths.iter().any(|path| is_database_file(path)),
        notify::EventKind::Create(
            notify::event::CreateKind::File | notify::event::CreateKind::Any,
        ) => event
            .paths
            .iter()
            .any(|path| path.file_name().and_then(|name| name.to_str()) == Some("sessions.db")),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::path::PathBuf;

    #[rstest]
    #[case::database_data(
        "/sessions/sessions.db",
        notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any
        )),
        true
    )]
    #[case::wal_data(
        "/sessions/sessions.db-wal",
        notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any
        )),
        true
    )]
    #[case::generic_database_modify(
        "/sessions/sessions.db",
        notify::EventKind::Modify(notify::event::ModifyKind::Any),
        true
    )]
    #[case::database_create(
        "/sessions/sessions.db",
        notify::EventKind::Create(notify::event::CreateKind::File),
        true
    )]
    #[case::generic_database_create(
        "/sessions/sessions.db",
        notify::EventKind::Create(notify::event::CreateKind::Any),
        true
    )]
    #[case::wal_create_ignored(
        "/sessions/sessions.db-wal",
        notify::EventKind::Create(notify::event::CreateKind::File),
        false
    )]
    #[case::wal_remove_ignored(
        "/sessions/sessions.db-wal",
        notify::EventKind::Remove(notify::event::RemoveKind::File),
        false
    )]
    #[case::database_remove_ignored(
        "/sessions/sessions.db",
        notify::EventKind::Remove(notify::event::RemoveKind::File),
        false
    )]
    #[case::shared_memory_ignored(
        "/sessions/sessions.db-shm",
        notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any
        )),
        false
    )]
    #[case::legacy_json_ignored(
        "/sessions/old-session.json",
        notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any
        )),
        false
    )]
    fn filters_session_store_events(
        #[case] path: &str,
        #[case] kind: notify::EventKind,
        #[case] expected: bool,
    ) {
        let event = notify::Event {
            kind,
            paths: vec![PathBuf::from(path)],
            attrs: Default::default(),
        };

        assert_eq!(is_session_store_change(&event), expected);
    }
}
