import type { Dispatch, SetStateAction } from 'react'
import { useCallback, useEffect, useMemo, useState } from 'react'

import {
  type InboxSort,
  isSelectedSessionBeyondVisibleInProgress,
  isSelectedSessionBeyondVisibleWaiting,
  readInboxSort,
  sortInboxSessionsByCategory,
  writeInboxSort,
} from '#inbox-sort'
import type { Session } from '#types'

type UseInboxSortOptions = {
  orderedSessions: readonly Session[]
  navigableSessions: readonly Session[]
  selectedId: string | null
  setShowAllInProgress: Dispatch<SetStateAction<boolean>>
  setShowAllWaiting: Dispatch<SetStateAction<boolean>>
}

export function useInboxSort({
  orderedSessions,
  navigableSessions,
  selectedId,
  setShowAllInProgress,
  setShowAllWaiting,
}: UseInboxSortOptions) {
  const [sortState, setSortState] = useState(readInboxSort)
  const sortedNavigableSessions = useMemo(
    () => sortInboxSessionsByCategory(navigableSessions, sortState.sort),
    [navigableSessions, sortState.sort],
  )
  const setSort = useCallback((sort: InboxSort) => {
    const storageAvailable = writeInboxSort(undefined, sort)
    setSortState({ sort, storageAvailable })
  }, [])

  useEffect(() => {
    if (
      isSelectedSessionBeyondVisibleInProgress(
        orderedSessions,
        selectedId,
        sortState.sort,
      )
    ) {
      setShowAllInProgress(true)
    }
    if (
      isSelectedSessionBeyondVisibleWaiting(
        orderedSessions,
        selectedId,
        sortState.sort,
      )
    ) {
      setShowAllWaiting(true)
    }
  }, [
    orderedSessions,
    selectedId,
    setShowAllInProgress,
    setShowAllWaiting,
    sortState.sort,
  ])

  return {
    sort: sortState.sort,
    storageAvailable: sortState.storageAvailable,
    navigableSessions: sortedNavigableSessions,
    setSort,
  }
}
