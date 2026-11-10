import type { Session } from '#types'

type ProjectStatus = 'Review' | 'Working' | 'Idle'

export type BoardProject = {
  name: string
  sessions: Session[]
  repos: string[]
  needsActionCount: number
  inProgressCount: number
  quiet: boolean
  status: ProjectStatus
}

export function buildBoardProjects(
  sessions: readonly Session[],
): BoardProject[] {
  const sessionsByProject = new Map<string, Session[]>()

  for (const session of sessions) {
    const project = session.project
    const name =
      project === undefined || project.trim().length === 0
        ? 'Unsorted'
        : project.trim()
    const projectSessions = sessionsByProject.get(name) ?? []
    projectSessions.push(session)
    sessionsByProject.set(name, projectSessions)
  }

  return Array.from(sessionsByProject, ([name, projectSessions]) => {
    const needsActionCount = projectSessions.filter(
      (session) => session.category === 'needs_action',
    ).length
    const inProgressCount = projectSessions.filter(
      (session) => session.category === 'in_progress',
    ).length
    const repos = Array.from(
      new Set(
        projectSessions
          .map((session) => session.repo)
          .filter((repo) => repo.length > 0),
      ),
    ).sort((left, right) => left.localeCompare(right, 'en'))
    const status: ProjectStatus =
      needsActionCount > 0 ? 'Review' : inProgressCount > 0 ? 'Working' : 'Idle'

    return {
      name,
      sessions: projectSessions,
      repos,
      needsActionCount,
      inProgressCount,
      quiet: needsActionCount === 0 && inProgressCount === 0,
      status,
    }
  }).sort(
    (left, right) =>
      right.needsActionCount - left.needsActionCount ||
      right.inProgressCount - left.inProgressCount ||
      left.name.localeCompare(right.name, 'en'),
  )
}

export function sortPmProjects(projects: readonly BoardProject[]) {
  const statusOrder: Record<ProjectStatus, number> = {
    Review: 0,
    Working: 1,
    Idle: 2,
  }

  return [...projects].sort(
    (left, right) =>
      statusOrder[left.status] - statusOrder[right.status] ||
      right.needsActionCount - left.needsActionCount ||
      right.inProgressCount - left.inProgressCount ||
      left.name.localeCompare(right.name, 'en'),
  )
}
