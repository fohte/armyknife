pub(super) fn check_can_delete_comment(
    comment_author: &str,
    current_user: &str,
    allow_delete: bool,
    database_id: i64,
) -> anyhow::Result<()> {
    if allow_delete {
        Ok(())
    } else if comment_author == current_user {
        anyhow::bail!(
            "Cannot delete comment (database_id: {}). Use --allow-delete to allow.",
            database_id
        )
    } else {
        anyhow::bail!(
            "Cannot delete other user's comment (database_id: {}, author: {}). Use --allow-delete to allow.",
            database_id,
            comment_author
        )
    }
}

pub(super) fn check_can_edit_comment(
    comment_author: &str,
    current_user: &str,
    edit_others: bool,
    filename: &str,
) -> anyhow::Result<()> {
    if comment_author == current_user || edit_others {
        Ok(())
    } else {
        anyhow::bail!(
            "Cannot edit other user's comment: {} (author: {}). Use --edit-others to allow.",
            filename,
            comment_author
        )
    }
}
