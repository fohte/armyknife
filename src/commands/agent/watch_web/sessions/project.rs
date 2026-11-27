use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use super::SessionTaskMetadata;
use crate::commands::agent::types::Session;
use crate::infra::git::{github_owner_and_repo, open_repo_at};

const REPO_NAME_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

static REPO_NAME_CACHE: OnceLock<RepoNameCache> = OnceLock::new();

#[derive(Default)]
pub(super) struct RepoNameCache {
    entries: Mutex<HashMap<PathBuf, CachedRepoName>>,
}

struct CachedRepoName {
    name: String,
    expires_at: Instant,
}

impl RepoNameCache {
    fn resolve(&self, cwd: &Path, resolve_repo: impl FnOnce() -> String) -> String {
        self.resolve_at(cwd, Instant::now(), resolve_repo)
    }

    fn resolve_at(
        &self,
        cwd: &Path,
        now: Instant,
        resolve_repo: impl FnOnce() -> String,
    ) -> String {
        {
            let mut entries = self.lock_entries();
            entries.retain(|_, entry| entry.expires_at > now);
            if let Some(entry) = entries.get(cwd) {
                return entry.name.clone();
            }
        }

        let name = resolve_repo();
        let mut entries = self.lock_entries();
        if let Some(entry) = entries.get(cwd).filter(|entry| entry.expires_at > now) {
            return entry.name.clone();
        }
        entries.insert(
            cwd.to_path_buf(),
            CachedRepoName {
                name: name.clone(),
                expires_at: now + REPO_NAME_CACHE_TTL,
            },
        );
        name
    }

    fn lock_entries(&self) -> MutexGuard<'_, HashMap<PathBuf, CachedRepoName>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub(super) fn repo_name_cache() -> &'static RepoNameCache {
    REPO_NAME_CACHE.get_or_init(RepoNameCache::default)
}

pub(super) fn resolve_repos_for_sessions(
    sessions: &[Session],
    all_sessions: &[Session],
    repo_cache: &RepoNameCache,
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
                .or_insert_with(|| repo_cache.resolve(&session.cwd, || resolve_repo(&session.cwd)))
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
    use super::{
        REPO_NAME_CACHE_TTL, RepoNameCache, SessionTaskMetadata, resolve_parent_project,
        resolve_project,
    };
    use crate::commands::agent::tq_session_tasks::SessionTask;
    use rstest::rstest;
    use std::collections::HashMap;
    use std::path::Path;
    use std::time::{Duration, Instant};

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

    #[test]
    fn cached_repo_names_are_reused_until_the_ttl_expires() {
        let cache = RepoNameCache::default();
        let cwd = Path::new("/example/repository");
        let resolved_at = Instant::now();
        let mut resolve_count = 0;

        let first = cache.resolve_at(cwd, resolved_at, || {
            resolve_count += 1;
            "owner/first-repository".to_string()
        });
        let before_expiration = cache.resolve_at(
            cwd,
            resolved_at + REPO_NAME_CACHE_TTL - Duration::from_nanos(1),
            || {
                resolve_count += 1;
                "owner/second-repository".to_string()
            },
        );
        let after_expiration = cache.resolve_at(cwd, resolved_at + REPO_NAME_CACHE_TTL, || {
            resolve_count += 1;
            "owner/second-repository".to_string()
        });

        assert_eq!(
            (first, before_expiration, after_expiration, resolve_count),
            (
                "owner/first-repository".to_string(),
                "owner/first-repository".to_string(),
                "owner/second-repository".to_string(),
                2,
            ),
        );
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
