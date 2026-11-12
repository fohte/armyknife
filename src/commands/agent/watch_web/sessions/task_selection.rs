pub(super) fn primary_task_id<'a>(
    saved_task_id: Option<&str>,
    linked_task_ids: impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    let mut first_linked_task_id = None;
    for task_id in linked_task_ids {
        first_linked_task_id.get_or_insert(task_id);
        if saved_task_id == Some(task_id) {
            return Some(task_id);
        }
    }

    first_linked_task_id
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::primary_task_id;

    #[rstest]
    #[case::saved_primary_is_linked(Some("primary"), vec!["first", "primary"], Some("primary"))]
    #[case::stale_primary_falls_back_to_first(Some("stale"), vec!["first", "second"], Some("first"))]
    #[case::missing_primary_uses_first(None, vec!["first", "second"], Some("first"))]
    #[case::no_linked_tasks_returns_none(Some("stale"), Vec::new(), None)]
    fn resolves_the_primary_task(
        #[case] saved_task_id: Option<&str>,
        #[case] linked_task_ids: Vec<&str>,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(
            primary_task_id(saved_task_id, linked_task_ids.into_iter()),
            expected,
        );
    }
}
