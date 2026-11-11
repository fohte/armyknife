use std::collections::{HashMap, HashSet};

use super::SessionTaskMetadata;
use crate::commands::agent::types::Session;
use crate::infra::git::{github_owner_and_repo, open_repo_at};

pub(super) fn resolve_repos_for_sessions(
    sessions: &[Session],
    all_sessions: &[Session],
    mut resolve_repo: impl FnMut(&std::path::Path) -> String,
) -> HashMap<String, String> {
    let relevant_ids = sessions
        .iter()
        .flat_map(|session| {
            std::iter::once(session.session_id.clone())
                .chain(session.ancestor_session_ids.iter().cloned())
        })
        .collect::<HashSet<_>>();
    let mut repo_by_cwd = HashMap::<std::path::PathBuf, String>::new();

    all_sessions
        .iter()
        .filter(|session| relevant_ids.contains(&session.session_id))
        .map(|session| {
            let repo = repo_by_cwd
                .entry(session.cwd.clone())
                .or_insert_with(|| resolve_repo(&session.cwd))
                .clone();
            (session.session_id.clone(), repo)
        })
        .collect()
}

pub(super) fn resolve_repo(cwd: &std::path::Path) -> String {
    if let Ok(repo) = open_repo_at(cwd)
        && let Ok((owner, repo)) = github_owner_and_repo(&repo)
    {
        return format!("{owner}/{repo}");
    }

    cwd.file_name()
        .and_then(|name| name.to_str())
        .map(String::from)
        .unwrap_or_else(|| cwd.display().to_string())
}

pub(super) fn resolve_project(
    parent_project: Option<&str>,
    task_project: Option<&str>,
    repo: &str,
) -> String {
    parent_project
        .filter(|project| !project.trim().is_empty())
        .or_else(|| task_project.filter(|project| !project.trim().is_empty()))
        .or_else(|| (!repo.trim().is_empty()).then_some(repo))
        .unwrap_or("Unsorted")
        .to_string()
}

pub(super) fn resolve_parent_project<'a>(
    ancestor_session_ids: &[String],
    task_metadata: &'a SessionTaskMetadata,
    repo_by_id: &'a HashMap<String, String>,
) -> Option<&'a str> {
    ancestor_session_ids.iter().find_map(|ancestor_id| {
        task_metadata
            .links
            .get(ancestor_id)
            .and_then(|tasks| tasks.first())
            .and_then(|task| task_metadata.task_projects.get(&task.task_id))
            .map(String::as_str)
            .filter(|project| !project.trim().is_empty())
            .or_else(|| {
                repo_by_id
                    .get(ancestor_id)
                    .map(String::as_str)
                    .filter(|repo| !repo.trim().is_empty())
            })
    })
}

#[cfg(test)]
mod tests {
    use super::{SessionTaskMetadata, resolve_parent_project, resolve_project};
    use crate::commands::agent::tq_session_tasks::SessionTask;
    use rstest::rstest;
    use std::collections::HashMap;

    fn task_metadata(links: &[(&str, &str)], projects: &[(&str, &str)]) -> SessionTaskMetadata {
        SessionTaskMetadata {
            links: links
                .iter()
                .map(|(session_id, task_id)| {
                    (
                        (*session_id).to_string(),
                        vec![SessionTask {
                            task_id: (*task_id).to_string(),
                            task_number: 1,
                            task_title: "Sample task".to_string(),
                            parent_task_id: None,
                            is_closed: false,
                        }],
                    )
                })
                .collect(),
            task_projects: projects
                .iter()
                .map(|(task_id, project)| (task_id.to_string(), project.to_string()))
                .collect(),
            ..SessionTaskMetadata::default()
        }
    }

    #[rstest]
    #[case::parent_project_wins(Some("Parent"), Some("Task"), "repo", "Parent")]
    #[case::task_project_precedes_repo(None, Some("Task"), "repo", "Task")]
    #[case::repo_is_the_fallback(None, None, "repo", "repo")]
    #[case::unsorted_is_the_final_fallback(None, None, "", "Unsorted")]
    #[case::empty_candidates_are_ignored(Some(" "), Some(""), " ", "Unsorted")]
    fn resolves_projects_by_source_precedence(
        #[case] parent: Option<&str>,
        #[case] task: Option<&str>,
        #[case] repo: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(resolve_project(parent, task, repo), expected);
    }

    #[rstest]
    #[case::single_parent_uses_its_task_project(vec!["parent"], vec![("parent", "task")], vec![("task", "Project")], vec![], Some("Project"))]
    #[case::single_parent_falls_back_to_its_repo(vec!["parent"], vec![], vec![], vec![("parent", "repo")], Some("repo"))]
    #[case::three_levels_use_the_root_project_first(vec!["root", "parent"], vec![("root", "task-root"), ("parent", "task-parent")], vec![("task-root", "Root"), ("task-parent", "Parent")], vec![], Some("Root"))]
    #[case::unknown_root_uses_the_next_available_ancestor(vec!["unknown", "parent"], vec![("parent", "task")], vec![("task", "Project")], vec![], Some("Project"))]
    #[case::unknown_ancestors_fall_back_to_the_current_session(vec!["unknown"], vec![], vec![], vec![], None)]
    fn resolves_parent_project_from_root_to_parent(
        #[case] ancestor_ids: Vec<&str>,
        #[case] links: Vec<(&str, &str)>,
        #[case] projects: Vec<(&str, &str)>,
        #[case] repos: Vec<(&str, &str)>,
        #[case] expected: Option<&str>,
    ) {
        let ancestor_ids = ancestor_ids
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let metadata = task_metadata(&links, &projects);
        let repo_by_id = repos
            .into_iter()
            .map(|(session_id, repo)| (session_id.to_string(), repo.to_string()))
            .collect::<HashMap<_, _>>();

        assert_eq!(
            resolve_parent_project(&ancestor_ids, &metadata, &repo_by_id),
            expected,
        );
    }
}
