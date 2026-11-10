import type { Session } from '#types'

export type PaletteScope =
  'all' | 'sessions' | 'tasks' | 'projects' | 'commands'

export type PaletteSessionItem = {
  type: 'session'
  id: string
  session: Session
}

export type PaletteTaskItem = {
  type: 'task'
  id: string
  task: NonNullable<Session['task']>
  sessions: Session[]
}

export type PaletteProjectItem = {
  type: 'project'
  id: string
  name: string
  sessions: Session[]
}

export type PaletteCommandItem = {
  type: 'command'
  id: string
  label: string
  description: string
  taskNumber?: number
}

export type PaletteItem =
  PaletteSessionItem | PaletteTaskItem | PaletteProjectItem | PaletteCommandItem

export type PaletteGroup = {
  heading: string
  items: PaletteItem[]
}

const RECENT_SESSION_IDS_KEY = 'agentWatchRecentSessionIds'
const RECENT_SESSION_LIMIT = 8

export function readRecentSessionIds(storage: Storage): string[] {
  const value = storage.getItem(RECENT_SESSION_IDS_KEY)
  if (value === null) return []

  const parsed: unknown = JSON.parse(value)
  return Array.isArray(parsed)
    ? parsed.filter((entry): entry is string => typeof entry === 'string')
    : []
}

export function recordRecentSessionId(storage: Storage, id: string) {
  const recentIds = readRecentSessionIds(storage).filter(
    (recentId) => recentId !== id,
  )
  storage.setItem(
    RECENT_SESSION_IDS_KEY,
    JSON.stringify([id, ...recentIds].slice(0, RECENT_SESSION_LIMIT)),
  )
}

function resolveScope(query: string, selectedScope: PaletteScope) {
  const prefix = query.trimStart()[0]
  const scopeByPrefix: Record<string, PaletteScope> = {
    '#': 'tasks',
    '@': 'projects',
    '>': 'commands',
  }
  const prefixScope = prefix === undefined ? undefined : scopeByPrefix[prefix]
  return {
    scope: prefixScope ?? selectedScope,
    query:
      prefixScope === undefined ? query.trim() : query.trim().slice(1).trim(),
  }
}

function matches(query: string, ...values: Array<string | null | undefined>) {
  const needle = query.toLocaleLowerCase()
  return (
    needle.length === 0 ||
    values.some(
      (value) =>
        value !== null &&
        value !== undefined &&
        value.toLocaleLowerCase().includes(needle),
    )
  )
}

function taskItems(sessions: readonly Session[]): PaletteTaskItem[] {
  const byId = new Map<string, PaletteTaskItem>()
  for (const session of sessions) {
    if (session.task === null) continue
    const item = byId.get(session.task.id)
    if (item === undefined) {
      byId.set(session.task.id, {
        type: 'task',
        id: `task:${session.task.id}`,
        task: session.task,
        sessions: [session],
      })
    } else {
      item.sessions.push(session)
    }
  }
  return Array.from(byId.values()).sort(
    (left, right) => left.task.number - right.task.number,
  )
}

function projectItems(sessions: readonly Session[]): PaletteProjectItem[] {
  const byName = new Map<string, PaletteProjectItem>()
  for (const session of sessions) {
    const name = session.project.trim() || 'Unsorted'
    const item = byName.get(name)
    if (item === undefined) {
      byName.set(name, {
        type: 'project',
        id: `project:${name}`,
        name,
        sessions: [session],
      })
    } else {
      item.sessions.push(session)
    }
  }
  return Array.from(byName.values()).sort((left, right) =>
    left.name.localeCompare(right.name, 'en'),
  )
}

function baseCommands(hasInboxFilter: boolean): PaletteCommandItem[] {
  return [
    {
      type: 'command',
      id: 'switch-board',
      label: 'Switch to Board',
      description: 'Show projects and sessions',
    },
    {
      type: 'command',
      id: 'switch-inbox',
      label: 'Switch to Inbox',
      description: 'Show the session inbox',
    },
    {
      type: 'command',
      id: 'archive-done',
      label: 'Archive all done',
      description: 'Archive every done session',
    },
    ...(hasInboxFilter
      ? [
          {
            type: 'command' as const,
            id: 'clear-inbox-filter',
            label: 'Clear inbox filter',
            description: 'Show every session in the inbox',
          },
        ]
      : []),
  ]
}

function itemMatches(query: string, item: PaletteItem) {
  switch (item.type) {
    case 'session':
      return matches(
        query,
        item.session.title,
        item.session.kind,
        item.session.task?.title,
        item.session.task === null
          ? null
          : `#${String(item.session.task.number)}`,
        item.session.project,
        item.session.repo,
      )
    case 'task':
      return matches(
        query,
        item.task.title,
        `#${String(item.task.number)}`,
        ...item.sessions.map((session) => session.repo),
        ...item.sessions.map((session) => session.project),
      )
    case 'project':
      return matches(
        query,
        item.name,
        ...item.sessions.map((session) => session.repo),
      )
    case 'command':
      return matches(query, item.label, item.description)
  }
}

export function buildCommandPaletteGroups(
  sessions: readonly Session[],
  input: string,
  selectedScope: PaletteScope,
  recentSessionIds: readonly string[],
  hasInboxFilter: boolean,
): PaletteGroup[] {
  const { scope, query } = resolveScope(input, selectedScope)
  const needsAction = sessions.filter(
    (session) => session.category === 'needs_action',
  )
  const recentSessions = recentSessionIds.flatMap((id) => {
    const session = sessions.find((candidate) => candidate.id === id)
    return session === undefined
      ? []
      : [{ type: 'session' as const, id, session }]
  })
  const tasks = taskItems(sessions)
  const projects = projectItems(sessions)
  const commands = [
    ...baseCommands(hasInboxFilter),
    ...tasks.map((item) => ({
      type: 'command' as const,
      id: `filter-task:${String(item.task.number)}`,
      label: `Filter inbox by task #${String(item.task.number)}`,
      description: item.task.title,
      taskNumber: item.task.number,
    })),
  ]

  if (query.length === 0 && scope === 'all') {
    const groups = [
      {
        heading: 'NEEDS ACTION',
        items: needsAction.slice(0, 3).map((session) => ({
          type: 'session' as const,
          id: session.id,
          session,
        })),
      },
      { heading: 'RECENT', items: recentSessions },
      { heading: 'COMMANDS', items: commands },
    ]
    return groups.filter((group) => group.items.length > 0)
  }

  const sessionItems = sessions
    .filter((session) =>
      itemMatches(query, { type: 'session', id: session.id, session }),
    )
    .map((session) => ({ type: 'session' as const, id: session.id, session }))
  const candidates: Array<[PaletteScope, string, PaletteItem[]]> = [
    ['sessions', 'SESSIONS', sessionItems],
    ['tasks', 'TQ TASKS', tasks],
    ['projects', 'PROJECTS', projects],
    ['commands', 'COMMANDS', commands],
  ]
  return candidates.flatMap(([candidateScope, heading, items]) => {
    if (scope !== 'all' && scope !== candidateScope) return []
    const filtered = items.filter((item) => itemMatches(query, item))
    return filtered.length === 0 ? [] : [{ heading, items: filtered }]
  })
}
