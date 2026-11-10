import type { Session } from '#types'

export const INBOX_FILTER_FIELDS = [
  { key: 'project', label: 'Project', prefix: 'project:' },
  { key: 'kind', label: 'Kind', prefix: 'kind:' },
  { key: 'task', label: 'tq task', prefix: 'task:' },
  { key: 'repo', label: 'Repository', prefix: 'repo:' },
  { key: 'engine', label: 'Engine', prefix: 'engine:' },
] as const

export type InboxFilterKey = (typeof INBOX_FILTER_FIELDS)[number]['key']
export type InboxFilter = { key: InboxFilterKey; values: string[] }

export function normalizeInboxFilterValue(key: InboxFilterKey, value: string) {
  const trimmed = value.trim()
  return key === 'task'
    ? trimmed.replace(/^#/, '').toLowerCase()
    : trimmed.toLowerCase()
}

function splitClauses(query: string) {
  const clauses: string[] = []
  let start = 0
  let quoted = false
  let escaped = false

  for (let index = 0; index < query.length; index += 1) {
    const character = query[index]
    if (escaped) {
      escaped = false
    } else if (character === '\\' && quoted) {
      escaped = true
    } else if (character === '"') {
      quoted = !quoted
    } else if (
      character !== undefined &&
      /\s/.test(character) &&
      !quoted &&
      /\s+(?:project|kind|task|repo|engine):/i.test(query.slice(index))
    ) {
      const whitespace = query.slice(index).match(/^\s+/)?.[0].length ?? 0
      clauses.push(query.slice(start, index).trim())
      start = index + whitespace
      index = start - 1
    }
  }

  clauses.push(query.slice(start).trim())
  return clauses.filter((clause) => clause.length > 0)
}

function splitValues(valueList: string) {
  const values: string[] = []
  let value = ''
  let quoted = false
  let escaped = false

  for (const character of valueList) {
    if (escaped) {
      value += character
      escaped = false
    } else if (character === '\\' && quoted) {
      escaped = true
    } else if (character === '"') {
      quoted = !quoted
    } else if (character === ',' && !quoted) {
      values.push(value.trim())
      value = ''
    } else {
      value += character
    }
  }
  values.push(value.trim())
  return values.filter((candidate) => candidate.length > 0)
}

function formatValue(value: string) {
  return /[\s,"]/.test(value)
    ? `"${value.replaceAll('\\', '\\\\').replaceAll('"', '\\"')}"`
    : value
}

function sessionValues(session: Session, key: InboxFilterKey): string[] {
  switch (key) {
    case 'project':
      return [session.project]
    case 'kind':
      return session.kind === null ? [] : [session.kind]
    case 'task': {
      const linkedTasks = session.tasks ?? []
      const tasks =
        linkedTasks.length > 0
          ? linkedTasks
          : session.task === null
            ? []
            : [session.task]
      return tasks.map((task) => String(task.number))
    }
    case 'repo':
      return [session.repo]
    case 'engine':
      return session.engine === undefined ? [] : [session.engine]
  }
}

export function parseInboxFilterQuery(query: string): InboxFilter[] {
  const conditions = new Map<InboxFilterKey, string[]>()
  const clauses = splitClauses(query.trim())

  for (const clause of clauses) {
    const colonIndex = clause.indexOf(':')
    if (colonIndex < 1) continue
    const prefix = clause.slice(0, colonIndex).toLocaleLowerCase()
    const field = INBOX_FILTER_FIELDS.find(
      (candidate) => candidate.key === prefix,
    )
    if (field === undefined) continue

    const values = splitValues(clause.slice(colonIndex + 1))
    if (values.length === 0) continue

    const currentValues = conditions.get(field.key) ?? []
    for (const value of values) {
      if (
        !currentValues.some(
          (current) =>
            normalizeInboxFilterValue(field.key, current) ===
            normalizeInboxFilterValue(field.key, value),
        )
      ) {
        currentValues.push(value)
      }
    }
    conditions.set(field.key, currentValues)
  }

  return INBOX_FILTER_FIELDS.flatMap(({ key }) => {
    const values = conditions.get(key)
    return values === undefined ? [] : [{ key, values }]
  })
}

export function formatInboxFilterQuery(filters: readonly InboxFilter[]) {
  return INBOX_FILTER_FIELDS.flatMap(({ key }) => {
    const values = filters.find((filter) => filter.key === key)?.values ?? []
    return values.length === 0
      ? []
      : [`${key}:${values.map(formatValue).join(',')}`]
  }).join(' ')
}

export function filterInboxSessions(
  sessions: readonly Session[],
  filters: readonly InboxFilter[],
) {
  if (filters.length === 0) return [...sessions]

  return sessions.filter((session) =>
    filters.every((filter) => {
      const values = sessionValues(session, filter.key)
      return (
        values.length > 0 &&
        filter.values.some((candidate) =>
          values.some(
            (value) =>
              normalizeInboxFilterValue(filter.key, candidate) ===
              normalizeInboxFilterValue(filter.key, value),
          ),
        )
      )
    }),
  )
}

export function getInboxFilterOptions(
  sessions: readonly Session[],
  filters: readonly InboxFilter[],
  key: InboxFilterKey,
) {
  const otherFilters = filters.filter((filter) => filter.key !== key)
  const matchingSessions = filterInboxSessions(sessions, otherFilters)
  const counts = new Map<string, { value: string; count: number }>()

  for (const session of matchingSessions) {
    for (const value of sessionValues(session, key)) {
      const normalized = normalizeInboxFilterValue(key, value)
      const existing = counts.get(normalized)
      counts.set(normalized, {
        value: existing?.value ?? value,
        count: (existing?.count ?? 0) + 1,
      })
    }
  }

  return [...counts.values()].sort((left, right) =>
    left.value.localeCompare(right.value),
  )
}
