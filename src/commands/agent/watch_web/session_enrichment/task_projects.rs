use std::collections::{HashMap, HashSet};
use std::future::Future;

use crate::infra::tq::{TqProjectSummary, TqTaskSummary};

pub(super) async fn fetch_task_projects<F, Fut>(
    tasks: Vec<TqTaskSummary>,
    previous_projects: HashMap<String, String>,
    previous_task_projects: HashMap<String, String>,
    fetch_projects: F,
) -> (HashMap<String, String>, HashMap<String, String>)
where
    F: FnOnce() -> Fut + Send,
    Fut: Future<Output = Result<Vec<TqProjectSummary>, String>> + Send,
{
    let project_ids = tasks
        .iter()
        .filter_map(|task| task.project_id.as_ref())
        .collect::<HashSet<_>>();
    if project_ids.is_empty() {
        return (HashMap::new(), previous_projects);
    }

    if project_ids
        .iter()
        .all(|project_id| previous_projects.contains_key(*project_id))
    {
        return (
            task_project_names(tasks, previous_projects.clone()),
            previous_projects,
        );
    }

    match fetch_projects().await {
        Ok(projects) => {
            let project_names = projects
                .into_iter()
                .map(|project| (project.id, project.title))
                .collect::<HashMap<_, _>>();
            let task_projects = task_project_names(tasks, project_names.clone());
            (task_projects, project_names)
        }
        Err(error) => {
            tracing::warn!(%error, "Failed to fetch agent watch tq projects; keeping cached values");
            (previous_task_projects, previous_projects)
        }
    }
}

fn task_project_names(
    tasks: Vec<TqTaskSummary>,
    project_names: HashMap<String, String>,
) -> HashMap<String, String> {
    tasks
        .into_iter()
        .filter_map(|task| {
            let project_name = project_names.get(task.project_id.as_deref()?)?;
            Some((task.id, project_name.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use rstest::rstest;

    fn task(id: &str, project_id: Option<&str>) -> TqTaskSummary {
        TqTaskSummary {
            id: id.to_string(),
            number: 1,
            title: "example task".to_string(),
            updated_at: None,
            priority: None,
            project_id: project_id.map(str::to_string),
            github_links: Vec::new(),
        }
    }

    #[rstest]
    #[case::project_list_succeeds(
        Ok(vec![TqProjectSummary {
            id: "project-a".to_string(),
            title: "Sample project".to_string(),
        }]),
        HashMap::new(),
        HashMap::new(),
        (
            HashMap::from([("task-a".to_string(), "Sample project".to_string())]),
            HashMap::from([("project-a".to_string(), "Sample project".to_string())]),
        ),
    )]
    #[case::project_list_failure_keeps_cached_values(
        Err("temporary failure".to_string()),
        HashMap::from([("project-a".to_string(), "Cached project".to_string())]),
        HashMap::from([("task-a".to_string(), "Cached project".to_string())]),
        (
            HashMap::from([("task-a".to_string(), "Cached project".to_string())]),
            HashMap::from([("project-a".to_string(), "Cached project".to_string())]),
        ),
    )]
    #[tokio::test]
    async fn resolves_task_projects_from_the_cached_project_list(
        #[case] project_result: std::result::Result<Vec<TqProjectSummary>, String>,
        #[case] previous_projects: HashMap<String, String>,
        #[case] previous_task_projects: HashMap<String, String>,
        #[case] expected: (HashMap<String, String>, HashMap<String, String>),
    ) {
        assert_eq!(
            fetch_task_projects(
                vec![task("task-a", Some("project-a"))],
                previous_projects,
                previous_task_projects,
                move || async move { project_result },
            )
            .await,
            expected,
        );
    }

    #[tokio::test]
    async fn skips_project_list_when_cached_projects_cover_task_project_ids() {
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch_calls = Arc::clone(&calls);

        let actual = fetch_task_projects(
            vec![task("task-a", Some("project-a"))],
            HashMap::from([("project-a".to_string(), "Sample project".to_string())]),
            HashMap::new(),
            move || async move {
                fetch_calls.fetch_add(1, Ordering::SeqCst);
                Ok(Vec::new())
            },
        )
        .await;

        assert_eq!(
            (actual, calls.load(Ordering::SeqCst)),
            (
                (
                    HashMap::from([("task-a".to_string(), "Sample project".to_string())]),
                    HashMap::from([("project-a".to_string(), "Sample project".to_string())]),
                ),
                0,
            )
        );
    }
}
