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

function normalizeValue(key: InboxFilterKey, value: string) {
  const trimmed = value.trim()
  return key === 'task'
    ? trimmed.replace(/^#/, '').toLocaleLowerCase()
    : trimmed.toLocaleLowerCase()
}

function sessionValue(session: Session, key: InboxFilterKey): string | null {
  switch (key) {
    case 'project':
      return session.project
    case 'kind':
      return session.kind
    case 'task':
      return session.task === null ? null : String(session.task.number)
    case 'repo':
      return session.repo
    case 'engine':
      return session.engine ?? null
  }
}

export function parseInboxFilterQuery(query: string): InboxFilter[] {
  const conditions = new Map<InboxFilterKey, string[]>()
  const clauses = query
    .trim()
    .split(/\s+(?=(?:project|kind|task|repo|engine):)/i)

  for (const clause of clauses) {
    const colonIndex = clause.indexOf(':')
    if (colonIndex < 1) continue
    const prefix = clause.slice(0, colonIndex).toLocaleLowerCase()
    const field = INBOX_FILTER_FIELDS.find(
      (candidate) => candidate.key === prefix,
    )
    if (field === undefined) continue

    const values = clause
      .slice(colonIndex + 1)
      .split(',')
      .map((value) => value.trim())
      .filter((value) => value.length > 0)
    if (values.length === 0) continue

    const currentValues = conditions.get(field.key) ?? []
    for (const value of values) {
      if (
        !currentValues.some(
          (current) =>
            normalizeValue(field.key, current) ===
            normalizeValue(field.key, value),
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
    return values.length === 0 ? [] : [`${key}:${values.join(',')}`]
  }).join(' ')
}

export function filterInboxSessions(
  sessions: readonly Session[],
  filters: readonly InboxFilter[],
) {
  if (filters.length === 0) return [...sessions]

  return sessions.filter((session) =>
    filters.every((filter) => {
      const value = sessionValue(session, filter.key)
      return (
        value !== null &&
        filter.values.some(
          (candidate) =>
            normalizeValue(filter.key, candidate) ===
            normalizeValue(filter.key, value),
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
    const value = sessionValue(session, key)
    if (value === null) continue
    const normalized = normalizeValue(key, value)
    const existing = counts.get(normalized)
    counts.set(normalized, {
      value: existing?.value ?? value,
      count: (existing?.count ?? 0) + 1,
    })
  }

  return [...counts.values()].sort((left, right) =>
    left.value.localeCompare(right.value),
  )
}
