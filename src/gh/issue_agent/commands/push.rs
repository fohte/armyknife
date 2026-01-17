use std::collections::{HashMap, HashSet};

use clap::Args;
use similar::{ChangeTag, TextDiff};

use crate::gh::issue_agent::models::{Comment, IssueMetadata};
use crate::gh::issue_agent::storage::IssueStorage;
use crate::git::get_owner_repo;
use crate::github::{CommentClient, IssueClient, OctocrabClient};

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct PushArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,

    /// Show what would be changed without applying
    #[arg(long)]
    pub dry_run: bool,

    /// Allow overwriting remote changes (like git push --force)
    #[arg(long)]
    pub force: bool,

    /// Allow editing other users' comments
    #[arg(long)]
    pub edit_others: bool,
}

pub async fn run(args: &PushArgs) -> Result<(), Box<dyn std::error::Error>> {
    let repo = get_repo(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;

    let storage = IssueStorage::new(&repo, issue_number as i64);

    // 1. Check if local cache exists
    if !storage.dir().exists() {
        return Err(format!(
            "Issue #{} not found locally. Run 'a gh issue-agent pull {}' first.",
            issue_number, issue_number
        )
        .into());
    }

    println!("Fetching latest from GitHub...");

    // 2. Fetch latest from GitHub
    let client = OctocrabClient::get()?;
    let (owner, repo_name) = parse_repo(&repo)?;

    let remote_issue = client.get_issue(owner, repo_name, issue_number).await?;
    let remote_comments = client.get_comments(owner, repo_name, issue_number).await?;

    // 3. Check if remote has changed since pull
    let local_metadata = storage.read_metadata()?;
    let remote_updated_at = remote_issue.updated_at.to_rfc3339();
    if let Err(msg) =
        check_remote_unchanged(&local_metadata.updated_at, &remote_updated_at, args.force)
    {
        eprintln!();
        eprintln!("{}", msg);
        eprintln!();
        return Err(
            "Remote has changed. Use --force to overwrite, or 'refresh' to update local copy."
                .into(),
        );
    }

    // 4. Detect and display changes
    let current_user = get_current_user(client).await?;
    let mut has_changes = false;

    // Compare issue body
    let local_body = storage.read_body()?;
    let remote_body = remote_issue.body.as_deref().unwrap_or("");
    if local_body != remote_body {
        println!();
        println!("=== Issue Body ===");
        print_diff(remote_body, &local_body);
        if !args.dry_run {
            println!();
            println!("Updating issue body...");
            client
                .update_issue_body(owner, repo_name, issue_number, &local_body)
                .await?;
        }
        has_changes = true;
    }

    // Compare title
    if local_metadata.title != remote_issue.title {
        println!();
        println!("=== Title ===");
        println!("- {}", remote_issue.title);
        println!("+ {}", local_metadata.title);
        if !args.dry_run {
            println!();
            println!("Updating title...");
            client
                .update_issue_title(owner, repo_name, issue_number, &local_metadata.title)
                .await?;
        }
        has_changes = true;
    }

    // Compare labels
    let remote_labels: HashSet<&str> = remote_issue
        .labels
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    let local_labels: HashSet<&str> = local_metadata.labels.iter().map(|s| s.as_str()).collect();

    if remote_labels != local_labels {
        println!();
        println!("=== Labels ===");
        let mut remote_sorted: Vec<_> = remote_labels.iter().collect();
        remote_sorted.sort();
        let mut local_sorted: Vec<_> = local_labels.iter().collect();
        local_sorted.sort();
        println!("- {:?}", remote_sorted);
        println!("+ {:?}", local_sorted);

        if !args.dry_run {
            println!();
            println!("Updating labels...");

            let (labels_to_remove, labels_to_add) =
                compute_label_changes(&local_labels, &remote_labels);

            for label in labels_to_remove {
                client
                    .remove_label(owner, repo_name, issue_number, label)
                    .await?;
            }

            if !labels_to_add.is_empty() {
                let labels: Vec<String> = labels_to_add.iter().map(|s| s.to_string()).collect();
                client
                    .add_labels(owner, repo_name, issue_number, &labels)
                    .await?;
            }
        }
        has_changes = true;
    }

    // Compare comments
    let local_comments = storage.read_comments()?;
    let remote_comments_map: HashMap<&str, &Comment> =
        remote_comments.iter().map(|c| (c.id.as_str(), c)).collect();

    for local_comment in &local_comments {
        if local_comment.is_new() {
            // New comment
            println!();
            println!("=== New Comment: {} ===", local_comment.filename);
            println!("{}", local_comment.body);

            if !args.dry_run {
                println!();
                println!("Creating comment...");
                client
                    .create_comment(owner, repo_name, issue_number, &local_comment.body)
                    .await?;

                // Remove the new comment file after successful creation
                let comment_path = storage.dir().join("comments").join(&local_comment.filename);
                std::fs::remove_file(&comment_path)?;
            }
            has_changes = true;
        } else {
            // Existing comment
            let Some(comment_id) = &local_comment.metadata.id else {
                continue;
            };

            let Some(remote_comment) = remote_comments_map.get(comment_id.as_str()) else {
                continue;
            };

            if local_comment.body == remote_comment.body {
                continue;
            }

            let author = local_comment
                .metadata
                .author
                .as_deref()
                .unwrap_or("unknown");

            // Check if editing other user's comment
            if let Err(msg) = check_can_edit_comment(
                author,
                &current_user,
                args.edit_others,
                &local_comment.filename,
            ) {
                return Err(msg.into());
            }

            println!();
            if author != current_user {
                println!(
                    "=== Comment: {} (author: {}) ===",
                    local_comment.filename, author
                );
            } else {
                println!("=== Comment: {} ===", local_comment.filename);
            }
            print_diff(&remote_comment.body, &local_comment.body);

            if !args.dry_run {
                println!();
                println!("Updating comment...");
                let database_id = local_comment
                    .metadata
                    .database_id
                    .ok_or("Comment missing databaseId")?;
                client
                    .update_comment(owner, repo_name, database_id as u64, &local_comment.body)
                    .await?;
            }
            has_changes = true;
        }
    }

    // 5-6. Show result
    println!();
    if args.dry_run {
        if has_changes {
            println!("[dry-run] Changes detected. Run without --dry-run to apply.");
        } else {
            println!("[dry-run] No changes detected.");
        }
    } else if has_changes {
        // Update local updatedAt to match remote after successful push
        let new_remote_issue = client.get_issue(owner, repo_name, issue_number).await?;
        let new_metadata = IssueMetadata::from_issue(&new_remote_issue);
        storage.save_metadata(&new_metadata)?;
        println!("Done! Changes have been pushed to GitHub.");
    } else {
        println!("No changes to push.");
    }

    Ok(())
}

/// Get repository from argument or current directory.
fn get_repo(repo_arg: &Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(repo) = repo_arg {
        return Ok(repo.clone());
    }

    // Get from git remote origin
    let (owner, repo) =
        get_owner_repo().ok_or("Failed to determine current repository. Use -R to specify.")?;

    Ok(format!("{}/{}", owner, repo))
}

/// Parse owner/repo string into (owner, repo) tuple.
fn parse_repo(repo: &str) -> Result<(&str, &str), Box<dyn std::error::Error>> {
    repo.split_once('/')
        .ok_or_else(|| format!("Invalid repository format: {}. Expected owner/repo.", repo).into())
}

/// Get current GitHub user from the API.
async fn get_current_user(client: &OctocrabClient) -> Result<String, Box<dyn std::error::Error>> {
    let user = client.client.current().user().await?;
    Ok(user.login)
}

/// Print unified diff between old and new text.
fn print_diff(old: &str, new: &str) {
    let diff = TextDiff::from_lines(old, new);

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        // change already includes newline, so use print! instead of println!
        print!("{}{}", sign, change);
    }
}

/// Check if remote has changed since the last pull.
/// Returns Ok(()) if no conflict, Err with message if changed.
fn check_remote_unchanged(
    local_updated_at: &str,
    remote_updated_at: &str,
    force: bool,
) -> Result<(), String> {
    if force || local_updated_at == remote_updated_at {
        Ok(())
    } else {
        Err(format!(
            "Remote has changed since pull. Local: {}, Remote: {}",
            local_updated_at, remote_updated_at
        ))
    }
}

/// Check if the user can edit a comment.
/// Returns Ok(()) if allowed, Err with message if not.
fn check_can_edit_comment(
    comment_author: &str,
    current_user: &str,
    edit_others: bool,
    filename: &str,
) -> Result<(), String> {
    if comment_author == current_user || edit_others {
        Ok(())
    } else {
        Err(format!(
            "Cannot edit other user's comment: {} (author: {}). Use --edit-others to allow.",
            filename, comment_author
        ))
    }
}

/// Compute label changes between local and remote.
/// Returns (labels_to_remove, labels_to_add).
fn compute_label_changes<'a>(
    local_labels: &'a HashSet<&'a str>,
    remote_labels: &'a HashSet<&'a str>,
) -> (Vec<&'a str>, Vec<&'a str>) {
    let to_remove: Vec<&str> = remote_labels.difference(local_labels).copied().collect();
    let to_add: Vec<&str> = local_labels.difference(remote_labels).copied().collect();
    (to_remove, to_add)
}

/// Format diff as a string (for testing).
#[cfg(test)]
fn format_diff(old: &str, new: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut result = String::new();

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        result.push_str(sign);
        result.push_str(&change.to_string());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    mod check_remote_unchanged_tests {
        use super::*;

        #[rstest]
        #[case::same_timestamp("2024-01-01T00:00:00Z", "2024-01-01T00:00:00Z", false)]
        #[case::force_with_different("2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z", true)]
        #[case::force_with_same("2024-01-01T00:00:00Z", "2024-01-01T00:00:00Z", true)]
        fn test_ok(#[case] local: &str, #[case] remote: &str, #[case] force: bool) {
            assert!(check_remote_unchanged(local, remote, force).is_ok());
        }

        #[rstest]
        #[case::different_timestamp("2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z", false)]
        #[case::local_newer("2024-01-02T00:00:00Z", "2024-01-01T00:00:00Z", false)]
        fn test_err(#[case] local: &str, #[case] remote: &str, #[case] force: bool) {
            let result = check_remote_unchanged(local, remote, force);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.contains("Remote has changed"));
            assert!(err.contains(local));
            assert!(err.contains(remote));
        }
    }

    mod check_can_edit_comment_tests {
        use super::*;

        #[rstest]
        #[case::own_comment("alice", "alice", false, "001_comment.md")]
        #[case::other_comment_with_edit_others("bob", "alice", true, "001_comment.md")]
        #[case::own_comment_with_edit_others("alice", "alice", true, "001_comment.md")]
        fn test_allowed(
            #[case] author: &str,
            #[case] current_user: &str,
            #[case] edit_others: bool,
            #[case] filename: &str,
        ) {
            assert!(check_can_edit_comment(author, current_user, edit_others, filename).is_ok());
        }

        #[rstest]
        #[case::other_comment_without_flag("bob", "alice", false, "001_comment.md")]
        #[case::unknown_author("unknown", "alice", false, "002_comment.md")]
        fn test_denied(
            #[case] author: &str,
            #[case] current_user: &str,
            #[case] edit_others: bool,
            #[case] filename: &str,
        ) {
            let result = check_can_edit_comment(author, current_user, edit_others, filename);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(err.contains("Cannot edit other user's comment"));
            assert!(err.contains(filename));
            assert!(err.contains(author));
            assert!(err.contains("--edit-others"));
        }
    }

    mod compute_label_changes_tests {
        use super::*;

        #[rstest]
        #[case::no_changes(
            vec!["bug", "feature"],
            vec!["bug", "feature"],
            vec![],
            vec![]
        )]
        #[case::add_one_label(
            vec!["bug", "feature", "new-label"],
            vec!["bug", "feature"],
            vec![],
            vec!["new-label"]
        )]
        #[case::remove_one_label(
            vec!["bug"],
            vec!["bug", "feature"],
            vec!["feature"],
            vec![]
        )]
        #[case::add_and_remove(
            vec!["bug", "new-label"],
            vec!["bug", "old-label"],
            vec!["old-label"],
            vec!["new-label"]
        )]
        #[case::empty_local(
            vec![],
            vec!["bug", "feature"],
            vec!["bug", "feature"],
            vec![]
        )]
        #[case::empty_remote(
            vec!["bug", "feature"],
            vec![],
            vec![],
            vec!["bug", "feature"]
        )]
        #[case::both_empty(
            vec![],
            vec![],
            vec![],
            vec![]
        )]
        fn test_label_changes(
            #[case] local_labels: Vec<&str>,
            #[case] remote_labels: Vec<&str>,
            #[case] expected_remove: Vec<&str>,
            #[case] expected_add: Vec<&str>,
        ) {
            let local: HashSet<&str> = local_labels.into_iter().collect();
            let remote: HashSet<&str> = remote_labels.into_iter().collect();
            let (mut to_remove, mut to_add) = compute_label_changes(&local, &remote);
            to_remove.sort();
            to_add.sort();
            let mut expected_remove = expected_remove;
            let mut expected_add = expected_add;
            expected_remove.sort();
            expected_add.sort();
            assert_eq!(to_remove, expected_remove);
            assert_eq!(to_add, expected_add);
        }
    }

    mod parse_repo_tests {
        use super::*;

        #[rstest]
        #[case::simple("owner/repo", "owner", "repo")]
        #[case::real_repo("fohte/armyknife", "fohte", "armyknife")]
        #[case::with_dashes("org-name/repo-name", "org-name", "repo-name")]
        #[case::with_underscores("my_org/my_repo", "my_org", "my_repo")]
        #[case::with_dots("org.name/repo.name", "org.name", "repo.name")]
        #[case::with_numbers("user123/project456", "user123", "project456")]
        fn test_valid(
            #[case] input: &str,
            #[case] expected_owner: &str,
            #[case] expected_repo: &str,
        ) {
            let (owner, repo) = parse_repo(input).unwrap();
            assert_eq!(owner, expected_owner);
            assert_eq!(repo, expected_repo);
        }

        #[rstest]
        #[case::no_slash("noslash")]
        #[case::empty("")]
        #[case::only_owner("owner")]
        fn test_invalid(#[case] input: &str) {
            let result = parse_repo(input);
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Invalid repository format"));
        }

        // split_once accepts these as valid splits, resulting in empty strings
        #[rstest]
        #[case::only_slash("/", "", "")]
        #[case::trailing_slash("owner/", "owner", "")]
        #[case::leading_slash("/repo", "", "repo")]
        fn test_edge_cases_with_slash(
            #[case] input: &str,
            #[case] expected_owner: &str,
            #[case] expected_repo: &str,
        ) {
            let (owner, repo) = parse_repo(input).unwrap();
            assert_eq!(owner, expected_owner);
            assert_eq!(repo, expected_repo);
        }

        #[rstest]
        #[case::multiple_slashes("a/b/c")]
        #[case::deeply_nested("org/repo/sub/path")]
        fn test_multiple_slashes_takes_first(#[case] input: &str) {
            // split_once only splits on the first '/', so "a/b/c" -> ("a", "b/c")
            let result = parse_repo(input);
            assert!(result.is_ok());
        }
    }

    mod get_repo_tests {
        use super::*;

        #[rstest]
        #[case::simple("owner/repo")]
        #[case::real_repo("fohte/armyknife")]
        #[case::with_special_chars("my-org/my_repo.rs")]
        fn test_with_arg_returns_as_is(#[case] repo: &str) {
            let result = get_repo(&Some(repo.to_string())).unwrap();
            assert_eq!(result, repo);
        }
    }

    mod format_diff_tests {
        use super::*;

        #[rstest]
        #[case::single_line("line1\n", "line1\n", vec![" line1"])]
        #[case::multiple_lines("line1\nline2\n", "line1\nline2\n", vec![" line1", " line2"])]
        #[case::empty("", "", vec![])]
        fn test_no_changes(
            #[case] old: &str,
            #[case] new: &str,
            #[case] expected_lines: Vec<&str>,
        ) {
            let diff = format_diff(old, new);
            for line in expected_lines {
                assert!(
                    diff.contains(line),
                    "Expected '{}' in diff:\n{}",
                    line,
                    diff
                );
            }
            // No changes should not have - or + markers (except in content)
            let lines: Vec<&str> = diff.lines().collect();
            for line in lines {
                assert!(
                    line.starts_with(' ') || line.is_empty(),
                    "Expected no changes but found: {}",
                    line
                );
            }
        }

        #[rstest]
        #[case::add_one_line("line1\n", "line1\nline2\n", vec![" line1", "+line2"])]
        #[case::add_multiple("a\n", "a\nb\nc\n", vec![" a", "+b", "+c"])]
        #[case::add_to_empty("", "new\n", vec!["+new"])]
        fn test_additions(#[case] old: &str, #[case] new: &str, #[case] expected: Vec<&str>) {
            let diff = format_diff(old, new);
            for line in expected {
                assert!(
                    diff.contains(line),
                    "Expected '{}' in diff:\n{}",
                    line,
                    diff
                );
            }
        }

        #[rstest]
        #[case::delete_one_line("line1\nline2\n", "line1\n", vec![" line1", "-line2"])]
        #[case::delete_multiple("a\nb\nc\n", "a\n", vec![" a", "-b", "-c"])]
        #[case::delete_all("old\n", "", vec!["-old"])]
        fn test_deletions(#[case] old: &str, #[case] new: &str, #[case] expected: Vec<&str>) {
            let diff = format_diff(old, new);
            for line in expected {
                assert!(
                    diff.contains(line),
                    "Expected '{}' in diff:\n{}",
                    line,
                    diff
                );
            }
        }

        #[rstest]
        #[case::simple_modification("old\n", "new\n", vec!["-old", "+new"])]
        #[case::modify_middle("a\nold\nc\n", "a\nnew\nc\n", vec![" a", "-old", "+new", " c"])]
        #[case::complex_change("foo\nbar\nbaz\n", "foo\nqux\nbaz\n", vec![" foo", "-bar", "+qux", " baz"])]
        fn test_modifications(#[case] old: &str, #[case] new: &str, #[case] expected: Vec<&str>) {
            let diff = format_diff(old, new);
            for line in expected {
                assert!(
                    diff.contains(line),
                    "Expected '{}' in diff:\n{}",
                    line,
                    diff
                );
            }
        }

        #[rstest]
        #[case::mixed_operations(
            "keep\ndelete\nmodify\n",
            "keep\nmodified\nnew\n",
            vec![" keep", "-delete", "-modify", "+modified", "+new"]
        )]
        fn test_mixed_changes(#[case] old: &str, #[case] new: &str, #[case] expected: Vec<&str>) {
            let diff = format_diff(old, new);
            for line in expected {
                assert!(
                    diff.contains(line),
                    "Expected '{}' in diff:\n{}",
                    line,
                    diff
                );
            }
        }
    }
}
