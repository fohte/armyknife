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

type WorkTypeNavProject = {
  name: string
  status: ProjectStatus
  needsActionCount: number
  sessionId: string
}

export type WorkTypeNavGroup = {
  label: string
  projects: WorkTypeNavProject[]
}

export function buildBoardProjects(
  sessions: readonly Session[],
): BoardProject[] {
  const sessionsByProject = new Map<string, Session[]>()

  for (const session of sessions) {
    const name = session.project.trim() || 'Unsorted'
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

export function buildWorkTypeNavGroups(
  sessions: readonly Session[],
  workTypeNavLabels: Readonly<Record<string, string>>,
): WorkTypeNavGroup[] {
  const workTypesByLabel = new Map<string, Set<string>>()
  for (const [workType, label] of Object.entries(workTypeNavLabels)) {
    if (label.trim().length === 0) {
      continue
    }

    const workTypes = workTypesByLabel.get(label) ?? new Set<string>()
    workTypes.add(workType)
    workTypesByLabel.set(label, workTypes)
  }

  const projects = buildBoardProjects(sessions)
  return Array.from(workTypesByLabel, ([label, workTypes]) => ({
    label,
    projects: projects.flatMap((project) => {
      const matchingSessions = project.sessions
        .filter(
          (session) =>
            session.workType !== null &&
            session.workType !== undefined &&
            workTypes.has(session.workType),
        )
        .sort(compareNavSessions)
      const session = matchingSessions[0]

      return session === undefined
        ? []
        : [
            {
              name: project.name,
              status: project.status,
              needsActionCount: project.needsActionCount,
              sessionId: session.id,
            },
          ]
    }),
  })).sort((left, right) => left.label.localeCompare(right.label, 'en'))
}

function compareNavSessions(left: Session, right: Session) {
  const categoryOrder: Record<string, number> = {
    needs_action: 0,
    in_progress: 1,
    later: 2,
    done: 3,
  }

  return (
    (categoryOrder[left.category] ?? 4) -
      (categoryOrder[right.category] ?? 4) ||
    left.elapsedSeconds - right.elapsedSeconds ||
    left.id.localeCompare(right.id, 'en')
  )
}
