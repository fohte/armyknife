import type { Session } from '#types'

type ProjectStatus = 'Review' | 'Working' | 'Waiting' | 'Idle'

export type BoardProject = {
  name: string
  sessions: Session[]
  repos: string[]
  needsActionCount: number
  waitingCount: number
  inProgressCount: number
  quiet: boolean
  status: ProjectStatus
}

export type WorkTypeNavGroup = {
  label: string
  sessions: Session[]
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
    const waitingCount = projectSessions.filter(
      (session) => session.category === 'waiting',
    ).length
    const repos = Array.from(
      new Set(
        projectSessions
          .map((session) => session.repo)
          .filter((repo) => repo.length > 0),
      ),
    ).sort((left, right) => left.localeCompare(right, 'en'))
    const status: ProjectStatus =
      needsActionCount > 0
        ? 'Review'
        : inProgressCount > 0
          ? 'Working'
          : waitingCount > 0
            ? 'Waiting'
            : 'Idle'

    return {
      name,
      sessions: projectSessions,
      repos,
      needsActionCount,
      waitingCount,
      inProgressCount,
      quiet:
        needsActionCount === 0 && waitingCount === 0 && inProgressCount === 0,
      status,
    }
  }).sort(
    (left, right) =>
      right.needsActionCount - left.needsActionCount ||
      right.inProgressCount - left.inProgressCount ||
      right.waitingCount - left.waitingCount ||
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

  return Array.from(workTypesByLabel, ([label, workTypes]) => ({
    label,
    sessions: sessions
      .filter(
        (session) =>
          session.workType !== null &&
          session.workType !== undefined &&
          workTypes.has(session.workType),
      )
      .toSorted(compareNavSessions),
  })).sort((left, right) => left.label.localeCompare(right.label, 'en'))
}

function compareNavSessions(left: Session, right: Session) {
  const categoryOrder: Record<string, number> = {
    needs_action: 0,
    later: 1,
    waiting: 2,
    in_progress: 3,
    done: 4,
  }

  return (
    (categoryOrder[left.category] ?? 4) -
      (categoryOrder[right.category] ?? 4) ||
    left.elapsedSeconds - right.elapsedSeconds ||
    left.id.localeCompare(right.id, 'en')
  )
}
