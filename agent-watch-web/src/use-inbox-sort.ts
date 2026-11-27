import type { Dispatch, SetStateAction } from 'react'
import { useCallback, useEffect, useMemo, useState } from 'react'

import {
  type InboxSort,
  isSelectedSessionBeyondVisibleInProgress,
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
}

export function useInboxSort({
  orderedSessions,
  navigableSessions,
  selectedId,
  setShowAllInProgress,
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
  }, [orderedSessions, selectedId, setShowAllInProgress, sortState.sort])

  return {
    sort: sortState.sort,
    storageAvailable: sortState.storageAvailable,
    navigableSessions: sortedNavigableSessions,
    setSort,
  }
}
