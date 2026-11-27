import { Result } from 'neverthrow'

import type { Session } from '#types'

export type InboxSort = 'priority' | 'updated' | 'project'

export const DEFAULT_INBOX_SORT: InboxSort = 'priority'
export const INBOX_SORT_STORAGE_KEY = 'agent-watch-web:inbox-sort'

function isInboxSort(value: string | null): value is InboxSort {
  return value === 'priority' || value === 'updated' || value === 'project'
}

export function readInboxSort(storage?: Storage): InboxSort {
  return Result.fromThrowable(
    () => (storage ?? window.localStorage).getItem(INBOX_SORT_STORAGE_KEY),
    (error: unknown) => error,
  )().match(
    (value) => (isInboxSort(value) ? value : DEFAULT_INBOX_SORT),
    () => DEFAULT_INBOX_SORT,
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

function comparePriority(left: Session, right: Session) {
  const leftPriority = left.task?.priority ?? Number.POSITIVE_INFINITY
  const rightPriority = right.task?.priority ?? Number.POSITIVE_INFINITY
  if (leftPriority !== rightPriority) return leftPriority - rightPriority
  return updatedAt(right) - updatedAt(left)
}

function compareInboxSessions(sort: InboxSort, left: Session, right: Session) {
  switch (sort) {
    case 'priority':
      return comparePriority(left, right)
    case 'updated':
      return updatedAt(right) - updatedAt(left)
    case 'project':
      return (
        left.project.localeCompare(right.project) ||
        comparePriority(left, right)
      )
  }
}

export function sortInboxSessions(
  sessions: readonly Session[],
  sort: InboxSort,
): Session[] {
  return sessions
    .map((session, index) => ({ session, index }))
    .sort(
      (left, right) =>
        compareInboxSessions(sort, left.session, right.session) ||
        left.index - right.index,
    )
    .map(({ session }) => session)
}
