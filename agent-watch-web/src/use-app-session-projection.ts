import type { Dispatch, SetStateAction } from 'react'
import { useMemo } from 'react'

import { buildBoardProjects, buildWorkTypeNavGroups } from '#board-model'
import {
  filterInboxSessions,
  formatInboxFilterQuery,
  parseInboxFilterQuery,
} from '#inbox-filter'
import type { SessionHistoryState } from '#session-history'
import type { Session } from '#types'
import { useSavedInboxViews } from '#use-saved-inbox-views'

type UseAppSessionProjectionOptions = {
  optimisticSessions: readonly Session[]
  nowMs: number
  inboxFilterQuery: string
  laterExpanded: boolean
  workTypeNavLabels: Readonly<Record<string, string>>
  view: SessionHistoryState['view']
  setError: Dispatch<SetStateAction<string | null>>
}

export function useAppSessionProjection({
  optimisticSessions,
  nowMs,
  inboxFilterQuery,
  laterExpanded,
  workTypeNavLabels,
  view,
  setError,
}: UseAppSessionProjectionOptions) {
  const sessionsWithElapsedTime = useMemo(
    () =>
      optimisticSessions.map((session) => {
        const deadlinePassed =
          session.category === 'later' &&
          session.laterUntil !== null &&
          session.laterUntil !== undefined &&
          Date.parse(session.laterUntil) <= nowMs
        return {
          ...session,
          category: deadlinePassed
            ? (session.categoryWhenNotLater ?? 'needs_action')
            : session.category,
          elapsedSeconds: session.elapsedSeconds,
        }
      }),
    [nowMs, optimisticSessions],
  )
  const todaySessions = useMemo(
    () => sessionsWithElapsedTime.filter((session) => session.isToday === true),
    [sessionsWithElapsedTime],
  )
  const orderedSessions = useMemo(
    () => [
      ...sessionsWithElapsedTime.filter(
        (session) => session.category === 'needs_action',
      ),
      ...sessionsWithElapsedTime.filter(
        (session) => session.category === 'later',
      ),
      ...sessionsWithElapsedTime.filter(
        (session) => session.category === 'in_progress',
      ),
      ...sessionsWithElapsedTime.filter(
        (session) => session.category === 'done',
      ),
    ],
    [sessionsWithElapsedTime],
  )
  const inboxFilters = useMemo(
    () => parseInboxFilterQuery(inboxFilterQuery),
    [inboxFilterQuery],
  )
  const filteredOrderedSessions = useMemo(
    () => filterInboxSessions(orderedSessions, inboxFilters),
    [inboxFilters, orderedSessions],
  )
  const normalizedInboxFilterQuery = formatInboxFilterQuery(inboxFilters)
  const savedViews = useSavedInboxViews(
    sessionsWithElapsedTime,
    normalizedInboxFilterQuery,
    view === 'inbox',
    setError,
  )
  const navigableSessions = useMemo(
    () =>
      filteredOrderedSessions.filter(
        (session) => laterExpanded || session.category !== 'later',
      ),
    [filteredOrderedSessions, laterExpanded],
  )
  const projects = useMemo(
    () => buildBoardProjects(sessionsWithElapsedTime),
    [sessionsWithElapsedTime],
  )
  const workTypeNavGroups = useMemo(
    () => buildWorkTypeNavGroups(sessionsWithElapsedTime, workTypeNavLabels),
    [sessionsWithElapsedTime, workTypeNavLabels],
  )
  const needsActionCount = filteredOrderedSessions.filter(
    (session) => session.category === 'needs_action',
  ).length

  return {
    sessionsWithElapsedTime,
    todaySessions,
    orderedSessions,
    navigableSessions,
    savedViews,
    projects,
    workTypeNavGroups,
    needsActionCount,
  }
}
