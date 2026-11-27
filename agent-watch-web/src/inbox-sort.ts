import { Result } from 'neverthrow'

import type { Session } from '#types'

export type InboxSort = 'priority' | 'updated' | 'project'

const INBOX_CATEGORY_ORDER: readonly Session['category'][] = [
  'needs_action',
  'later',
  'waiting',
  'in_progress',
  'done',
]

const DEFAULT_INBOX_SORT: InboxSort = 'priority'
const INBOX_SORT_STORAGE_KEY = 'agent-watch-web:inbox-sort'

type InboxSortRead = {
  sort: InboxSort
  storageAvailable: boolean
}

function isInboxSort(value: string | null): value is InboxSort {
  return value === 'priority' || value === 'updated' || value === 'project'
}

export function readInboxSort(storage?: Storage): InboxSortRead {
  return Result.fromThrowable(
    () => (storage ?? window.localStorage).getItem(INBOX_SORT_STORAGE_KEY),
    (error: unknown) => error,
  )().match(
    (value) => ({
      sort: isInboxSort(value) ? value : DEFAULT_INBOX_SORT,
      storageAvailable: true,
    }),
    () => ({ sort: DEFAULT_INBOX_SORT, storageAvailable: false }),
  )
}

export function writeInboxSort(storage: Storage | undefined, sort: InboxSort) {
  return Result.fromThrowable(
    () => {
      const targetStorage = storage ?? window.localStorage
      targetStorage.setItem(INBOX_SORT_STORAGE_KEY, sort)
    },
    (error: unknown) => error,
  )().isOk()
}

function updatedAt(session: Session) {
  const timestamp = Date.parse(session.updatedAt ?? '')
  return Number.isNaN(timestamp) ? Number.NEGATIVE_INFINITY : timestamp
}

function compareInboxSessions(
  sort: Exclude<InboxSort, 'priority'>,
  left: Session,
  right: Session,
) {
  switch (sort) {
    case 'updated':
      return updatedAt(right) - updatedAt(left)
    case 'project':
      return left.project.localeCompare(right.project)
  }
}

export function sortInboxSessions(
  sessions: readonly Session[],
  sort: InboxSort,
): Session[] {
  if (sort === 'priority') return [...sessions]

  return sessions
    .map((session, index) => ({ session, index }))
    .sort(
      (left, right) =>
        compareInboxSessions(sort, left.session, right.session) ||
        left.index - right.index,
    )
    .map(({ session }) => session)
}

export function sortInboxSessionsByCategory(
  sessions: readonly Session[],
  sort: InboxSort,
): Session[] {
  return INBOX_CATEGORY_ORDER.flatMap((category) =>
    sortInboxSessions(
      sessions.filter((session) => session.category === category),
      sort,
    ),
  )
}

function isSelectedSessionBeyondVisibleCategory(
  sessions: readonly Session[],
  selectedId: string | null,
  category: Session['category'],
  sort: InboxSort,
): boolean {
  if (selectedId === null) return false

  return sortInboxSessions(
    sessions.filter((session) => session.category === category),
    sort,
  )
    .slice(2)
    .some((session) => session.id === selectedId)
}

export function isSelectedSessionBeyondVisibleInProgress(
  sessions: readonly Session[],
  selectedId: string | null,
  sort: InboxSort,
): boolean {
  return isSelectedSessionBeyondVisibleCategory(
    sessions,
    selectedId,
    'in_progress',
    sort,
  )
}

export function isSelectedSessionBeyondVisibleWaiting(
  sessions: readonly Session[],
  selectedId: string | null,
  sort: InboxSort,
): boolean {
  return isSelectedSessionBeyondVisibleCategory(
    sessions,
    selectedId,
    'waiting',
    sort,
  )
}
